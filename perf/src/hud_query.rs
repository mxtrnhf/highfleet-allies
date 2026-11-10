//! Callback-scoped native fixture view for the two incoming-warning ray fans.
//! Evidence, ABI, limits and equivalence boundary: docs/hud-spatial-queries.md.

use std::{
    cell::{Cell, RefCell},
    mem::offset_of,
    sync::atomic::{AtomicBool, Ordering},
};

use highfleet::{general::Node, v1_163::Body};

use crate::spatial::{Bounds, Ray, SpatialIndex, MAX_FIXTURES};

pub const WARNING: usize = 0x1_4024_5180;
const BODY_ARRAY: usize = 0x1_43a3_3c90;
const BODY_COUNT: usize = 0x1_43a5_1150;
const BODY_LIMIT: usize = 15_000;
const ACTIVE_METHOD: usize = 0x1_4000_5e60;
const POLYGON_RAY: usize = 0x1_4031_98d0;
const POLYGON_BOUNDS: usize = 0x1_4031_8b20;
const NODE_STATE: usize = offset_of!(Node, state);
const BODY_LAYER: usize = offset_of!(Body, layer);
// Uncovered upstream fields, verified in the native scanner and fixture map.
const BODY_CLASS: usize = 0x28;
const BODY_B2: usize = 0x178;
const BODY_EXCLUDED: usize = 0x224;
const BOUNDS_PADDING: f32 = 1.0; // 20 world units; conservative f32 clipping guard band.
const MAX_BOUNDED_COORDINATE: f32 = 32768.0;
// Conservative admission tolerances, not a general floating-point error proof.
// Geometry outside them stays in every query instead of being spatially culled.
const ROTATION_NORM_TOLERANCE: f64 = 0.00001;
const NORMAL_LENGTH_SQUARED_TOLERANCE: f64 = 0.01;
const NORMAL_COMPONENT_TOLERANCE: f64 = 0.000001;
const MIN_CONVEX_TURN: f64 = 0.0001;
const CONVEX_PLANE_TOLERANCE: f64 = 0.0001;

const _: () = assert!(NODE_STATE == 0x48 && BODY_LAYER == 0x1f0);
const _: () = assert!(size_of::<Ray>() == 20 && size_of::<Bounds>() == 16);

static VERIFY: AtomicBool = AtomicBool::new(true);
static DISABLED: AtomicBool = AtomicBool::new(false);
static REPORTED: AtomicBool = AtomicBool::new(false);
static FALLBACK_REPORTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static SCOPE: RefCell<QueryScope> = RefCell::new(QueryScope::default());
}

#[derive(Clone, Copy)]
struct FixtureView {
    body: usize,
    shape: usize,
    transform: usize,
}

#[derive(Clone, Copy)]
struct Hit {
    body: usize,
    fraction: f32,
}

impl Default for Hit {
    fn default() -> Self {
        Self {
            body: 0,
            fraction: 1.0,
        }
    }
}

#[derive(Default)]
struct QueryScope {
    records: Vec<FixtureView>,
    bounds: Vec<Bounds>,
    index: SpatialIndex,
    candidates: Vec<usize>,
    built: bool,
    failed: bool,
    body_count: usize,
    verified_fans: u8,
    rays: u64,
    tested: u64,
    error: Option<&'static str>,
}

impl QueryScope {
    fn clear(&mut self) {
        // Retain Rust allocation capacity, never the logical native view.
        self.records.clear();
        self.bounds.clear();
        self.index.clear();
        self.candidates.clear();
        self.built = false;
        self.failed = false;
        self.verified_fans = 0;
        self.rays = 0;
        self.tested = 0;
        self.error = None;
    }

    unsafe fn build(&mut self) -> Result<(), &'static str> {
        // SAFETY: Called only inside the synchronous native warning callback;
        // objects/fixtures remain borrowed on its rendering thread. Every walk
        // has a hard cap, and only reviewed pure virtual methods are accepted.
        unsafe {
            self.body_count = read::<u32>(BODY_COUNT) as usize;
            if self.body_count > BODY_LIMIT {
                return Err("native body count exceeds capacity");
            }
            self.records
                .try_reserve(MAX_FIXTURES)
                .map_err(|_| "fixture view allocation")?;
            self.bounds
                .try_reserve(MAX_FIXTURES)
                .map_err(|_| "fixture bounds allocation")?;
            self.candidates
                .try_reserve(MAX_FIXTURES)
                .map_err(|_| "query output allocation")?;
            for ordinal in 0..self.body_count {
                let body = read::<usize>(BODY_ARRAY + ordinal * 8);
                if body == 0 {
                    return Err("null native body");
                }
                let vtable = read::<usize>(body);
                if vtable == 0 || read::<usize>(vtable + 0x78) != ACTIVE_METHOD {
                    return Err("unreviewed body activity virtual");
                }
                if read::<i32>(body + NODE_STATE) < 2 {
                    continue;
                }
                if read::<u8>(body + BODY_CLASS) & 0xf != 0xf {
                    continue;
                }
                let physics = read::<usize>(body + BODY_B2);
                if physics == 0 || read::<i32>(body + BODY_LAYER) == 0 {
                    continue;
                }
                let mut fixture = read::<usize>(physics + 0x70);
                let mut walked = 0;
                while fixture != 0 {
                    walked += 1;
                    if walked > MAX_FIXTURES || self.records.len() == MAX_FIXTURES {
                        return Err("fixture traversal/capacity limit");
                    }
                    let shape = read::<usize>(fixture + 0x18);
                    let owner = read::<usize>(fixture + 0x10);
                    if shape == 0 || owner == 0 {
                        return Err("null fixture shape/body");
                    }
                    let methods = read::<usize>(shape);
                    if methods == 0
                        || read::<usize>(methods + 0x20) != POLYGON_RAY
                        || read::<usize>(methods + 0x28) != POLYGON_BOUNDS
                    {
                        return Err("unreviewed fixture shape virtual");
                    }
                    let transform = owner + 0xc;
                    self.bounds.push(shape_bounds(shape, transform)?);
                    self.records.push(FixtureView {
                        body,
                        shape,
                        transform,
                    });
                    fixture = read::<usize>(fixture + 8);
                }
            }
            self.index.build(&self.bounds)?;
            self.built = true;
            Ok(())
        }
    }

    unsafe fn query(&mut self, ray: Ray, fan: u8) -> Option<Hit> {
        // SAFETY: Same callback-local native contract as build; reviewed native
        // raycasts only read shape/transform and write our plain scalar output.
        unsafe {
            if self.failed
                || fan > 1
                || !ray.valid()
                || !ray
                    .start
                    .into_iter()
                    .chain(ray.end)
                    .all(|value| value.abs() <= MAX_BOUNDED_COORDINATE)
            {
                return None;
            }
            if !self.built {
                if let Err(error) = self.build() {
                    self.failed = true;
                    self.error = Some(error);
                    return None;
                }
            }
            if read::<u32>(BODY_COUNT) as usize != self.body_count {
                self.failed = true;
                self.error = Some("native body array changed during callback");
                return None;
            }
            if !self.index.candidates(ray, &mut self.candidates) {
                return None;
            }
            let mut nearest = Hit::default();
            for &ordinal in &self.candidates {
                let record = self.records.get(ordinal)?;
                if read::<i32>(record.body + NODE_STATE) < 2
                    || read::<i32>(record.body + BODY_EXCLUDED) == 1
                {
                    continue;
                }
                self.tested += 1;
                intersect(*record, &ray, &mut nearest);
            }
            let bit = 1 << fan;
            if VERIFY.load(Ordering::Relaxed) && self.verified_fans & bit == 0 {
                // Real native ray predicates over the full live array, not a
                // comparison against the same spatial candidate list.
                let original = native_scan(&ray)?;
                self.verified_fans |= bit;
                if original.body != nearest.body
                    || original.fraction.to_bits() != nearest.fraction.to_bits()
                {
                    DISABLED.store(true, Ordering::Relaxed);
                    self.error = Some(
                        "native/indexed result mismatch; spatial queries disabled for session",
                    );
                    self.failed = true;
                    return None;
                }
            }
            self.rays += 1;
            Some(nearest)
        }
    }
}

/// Scalar/raw-pointer field reads only, never an owning native object copy.
unsafe fn read<T: Copy>(address: usize) -> T {
    // SAFETY: Each caller establishes the containing live native allocation and
    // readable field width. Unaligned scalar reads introduce no Rust references.
    unsafe { (address as *const T).read_unaligned() }
}

unsafe fn shape_bounds(shape: usize, transform: usize) -> Result<Bounds, &'static str> {
    // SAFETY: Reviewed polygon and live b2Transform, validated by build().
    unsafe {
        let count = read::<i32>(shape + 0x98);
        if !(3..=8).contains(&count) {
            return Err("invalid polygon vertex count");
        }
        let radius = read::<f32>(shape + 0xc);
        if !radius.is_finite() {
            return Err("nonfinite polygon radius");
        }
        let mut bounded = radius >= 0.0;
        for index in 0..4 {
            let value = read::<f32>(transform + index * 4);
            if !value.is_finite() {
                return Err("nonfinite transform");
            }
            bounded &= value.abs() <= MAX_BOUNDED_COORDINATE;
        }
        let sine = read::<f32>(transform + 8);
        let cosine = read::<f32>(transform + 12);
        bounded &= (f64::from(sine).powi(2) + f64::from(cosine).powi(2) - 1.0).abs()
            < ROTATION_NORM_TOLERANCE;
        for edge in 0..count as usize {
            let vertex = read::<[f32; 2]>(shape + 0x18 + edge * 8);
            let normal = read::<[f32; 2]>(shape + 0x58 + edge * 8);
            if !vertex.into_iter().chain(normal).all(f32::is_finite) {
                return Err("nonfinite polygon");
            }
            bounded &= vertex
                .into_iter()
                .all(|value| value.abs() <= MAX_BOUNDED_COORDINATE);
            let length_squared = f64::from(normal[0]).powi(2) + f64::from(normal[1]).powi(2);
            bounded &= (length_squared - 1.0).abs() <= NORMAL_LENGTH_SQUARED_TOLERANCE;
            let next = (edge + 1) % count as usize;
            let next_vertex = read::<[f32; 2]>(shape + 0x18 + next * 8);
            let edge_x = f64::from(next_vertex[0]) - f64::from(vertex[0]);
            let edge_y = f64::from(next_vertex[1]) - f64::from(vertex[1]);
            let edge_length = edge_x.hypot(edge_y);
            // A normal can support every vertex yet describe a much larger
            // half-plane intersection if it no longer matches its native edge.
            // Such geometry must be kept unbounded, never culled by vertex AABB.
            bounded &= edge_length > f64::from(f32::EPSILON);
            if edge_length > 0.0 {
                bounded &= (f64::from(normal[0]) - edge_y / edge_length).abs()
                    <= NORMAL_COMPONENT_TOLERANCE
                    && (f64::from(normal[1]) + edge_x / edge_length).abs()
                        <= NORMAL_COMPONENT_TOLERANCE;
            }
            let next_normal = read::<[f32; 2]>(shape + 0x58 + next * 8);
            let turn = f64::from(normal[0]) * f64::from(next_normal[1])
                - f64::from(normal[1]) * f64::from(next_normal[0]);
            bounded &= turn > MIN_CONVEX_TURN;
            for other in 0..count as usize {
                let point = read::<[f32; 2]>(shape + 0x18 + other * 8);
                let distance = (f64::from(point[0]) - f64::from(vertex[0])) * f64::from(normal[0])
                    + (f64::from(point[1]) - f64::from(vertex[1])) * f64::from(normal[1]);
                bounded &= distance <= CONVEX_PLANE_TOLERANCE;
            }
        }
        if !bounded {
            // Degenerate/non-convex/large geometry is never culled. Keep the
            // exact native predicate in original order rather than guessing.
            return Ok(Bounds {
                lower: [-f32::MAX; 2],
                upper: [f32::MAX; 2],
            });
        }
        let compute: unsafe extern "win64" fn(usize, *mut Bounds, usize, i32) =
            std::mem::transmute(POLYGON_BOUNDS);
        let mut bounds = Bounds::default();
        compute(shape, &mut bounds, transform, 0);
        if !bounds.valid() {
            return Err("invalid native polygon bounds");
        }
        for axis in 0..2 {
            bounds.lower[axis] -= BOUNDS_PADDING;
            bounds.upper[axis] += BOUNDS_PADDING;
        }
        Ok(bounds)
    }
}

unsafe fn intersect(record: FixtureView, ray: &Ray, nearest: &mut Hit) {
    // SAFETY: Whitelisted native pure polygon method, exact five-argument Win64
    // ABI; output is three f32 values normal.x/normal.y/fraction. No native owner
    // allocation or lifetime is transferred into Rust.
    unsafe {
        let cast: unsafe extern "win64" fn(usize, *mut [f32; 3], *const Ray, usize, i32) -> u8 =
            std::mem::transmute(POLYGON_RAY);
        let mut output = [0.0; 3];
        if cast(record.shape, &mut output, ray, record.transform, 0) != 0
            && output[2] < nearest.fraction
        {
            *nearest = Hit {
                body: record.body,
                fraction: output[2],
            };
        }
    }
}

unsafe fn native_scan(ray: &Ray) -> Option<Hit> {
    // SAFETY: Same borrowed state and exact native predicate/iteration order;
    // used only after build verifies all activity and shape virtuals are pure.
    unsafe {
        let count = read::<u32>(BODY_COUNT) as usize;
        if count > BODY_LIMIT {
            return None;
        }
        let mut hit = Hit::default();
        let mut fixtures = 0;
        for ordinal in 0..count {
            let body = read::<usize>(BODY_ARRAY + ordinal * 8);
            if body == 0 {
                return None;
            }
            if read::<i32>(body + NODE_STATE) < 2 || read::<i32>(body + BODY_EXCLUDED) == 1 {
                continue;
            }
            if read::<u8>(body + BODY_CLASS) & 0xf != 0xf {
                continue;
            }
            let physics = read::<usize>(body + BODY_B2);
            if physics == 0 || read::<i32>(body + BODY_LAYER) == 0 {
                continue;
            }
            let mut fixture = read::<usize>(physics + 0x70);
            while fixture != 0 {
                fixtures += 1;
                if fixtures > MAX_FIXTURES {
                    return None;
                }
                let record = FixtureView {
                    body,
                    shape: read::<usize>(fixture + 0x18),
                    transform: read::<usize>(fixture + 0x10) + 0xc,
                };
                intersect(record, ray, &mut hit);
                fixture = read::<usize>(fixture + 8);
            }
        }
        Some(hit)
    }
}

pub fn configure(verify: bool) {
    VERIFY.store(verify, Ordering::Relaxed);
}

pub unsafe extern "win64" fn draw_warning_scope() {
    crate::profile::count(0);
    let previous = DEPTH.with(|depth| depth.replace(depth.get().saturating_add(1)));
    if previous == 0 {
        SCOPE.with(|scope| {
            if let Ok(mut scope) = scope.try_borrow_mut() {
                scope.clear();
            }
        });
    }
    // SAFETY: Ordinary synchronous call to the unmodified no-argument warning
    // entry, on the native caller's thread/stack. Only its two loop heads detour.
    unsafe { std::mem::transmute::<usize, unsafe extern "win64" fn()>(WARNING)() };
    if previous == 0 {
        SCOPE.with(|scope| {
            if let Ok(mut scope) = scope.try_borrow_mut() {
                if scope.built || scope.failed {
                    let first_scope = !REPORTED.swap(true, Ordering::Relaxed);
                    let first_error = scope.error.is_some() && !FALLBACK_REPORTED.swap(true, Ordering::Relaxed);
                    if first_scope || first_error {
                        log::info!("hf_perf HUD query: fixtures={}, indexed_rays={}, exact_fixture_tests={}, status={:?}",
                            scope.records.len(), scope.rays, scope.tested, scope.error);
                    }
                }
                scope.clear();
            }
        });
    }
    DEPTH.with(|depth| depth.set(previous));
}

unsafe fn try_query_warning(ray: *const Ray, output: *mut u8, fan: u32) -> u8 {
    if ray.is_null()
        || output.is_null()
        || DISABLED.load(Ordering::Relaxed)
        || DEPTH.with(Cell::get) != 1
        || fan > 1
    {
        return 0;
    }
    // SAFETY: Assembly passes the native 20-byte input and 12-byte output
    // scratch in its current frame. Never write repr(C) tail padding over it.
    unsafe {
        let ray = ray.read_unaligned();
        SCOPE.with(|scope| {
            let Ok(mut scope) = scope.try_borrow_mut() else {
                return 0;
            };
            let Some(hit) = scope.query(ray, fan as u8) else {
                return 0;
            };
            output.cast::<usize>().write_unaligned(hit.body);
            output.add(8).cast::<f32>().write_unaligned(hit.fraction);
            1
        })
    }
}

pub unsafe extern "win64" fn query_warning(ray: *const Ray, output: *mut u8, fan: u32) -> u8 {
    crate::profile::count(1);
    // SAFETY: forwards the native loop's existing scratch pointers and fan ID.
    let handled = unsafe { try_query_warning(ray, output, fan) };
    crate::profile::count(if handled != 0 { 2 } else { 3 });
    handled
}
