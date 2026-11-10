//! Rust-owned, bounded BVH. Contains geometry/ordinals only, never game pointers.

const LEAF_SIZE: usize = 8;
pub const MAX_FIXTURES: usize = 65_536;

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Bounds {
    pub lower: [f32; 2],
    pub upper: [f32; 2],
}

impl Bounds {
    pub fn valid(self) -> bool {
        (0..2).all(|axis| {
            self.lower[axis].is_finite()
                && self.upper[axis].is_finite()
                && self.lower[axis] <= self.upper[axis]
        })
    }

    fn union(self, other: Self) -> Self {
        Self {
            lower: [
                self.lower[0].min(other.lower[0]),
                self.lower[1].min(other.lower[1]),
            ],
            upper: [
                self.upper[0].max(other.upper[0]),
                self.upper[1].max(other.upper[1]),
            ],
        }
    }

    pub fn intersects(self, ray: Ray) -> bool {
        // f64 slabs avoid f32 reciprocal overflow and preserve axis-aligned and
        // boundary-touching rays. Never shorten maxFraction using the best hit:
        // all candidates must be processed in native order for exact ties.
        let mut near = 0.0_f64;
        let mut far = f64::from(ray.max_fraction);
        for axis in 0..2 {
            let start = f64::from(ray.start[axis]);
            let direction = f64::from(ray.end[axis]) - start;
            let lower = f64::from(self.lower[axis]);
            let upper = f64::from(self.upper[axis]);
            if direction == 0.0 {
                if start < lower || start > upper {
                    return false;
                }
                continue;
            }
            let first = (lower - start) / direction;
            let second = (upper - start) / direction;
            near = near.max(first.min(second));
            far = far.min(first.max(second));
            if near > far {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Ray {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub max_fraction: f32,
}

impl Ray {
    pub fn valid(self) -> bool {
        self.start.into_iter().chain(self.end).all(f32::is_finite)
            && self.max_fraction.is_finite()
            && self.max_fraction >= 0.0
            && self.max_fraction <= 1.0
    }
}

struct Branch {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

#[derive(Default)]
pub struct SpatialIndex {
    bounds: Vec<Bounds>,
    order: Vec<usize>,
    branches: Vec<Branch>,
}

impl SpatialIndex {
    pub fn clear(&mut self) {
        self.bounds.clear();
        self.order.clear();
        self.branches.clear();
    }

    pub fn build(&mut self, bounds: &[Bounds]) -> Result<(), &'static str> {
        self.clear();
        if bounds.len() > MAX_FIXTURES || !bounds.iter().all(|bounds| bounds.valid()) {
            return Err("invalid fixture bounds/count");
        }
        self.bounds
            .try_reserve(bounds.len())
            .map_err(|_| "bounds allocation")?;
        self.order
            .try_reserve(bounds.len())
            .map_err(|_| "order allocation")?;
        self.branches
            .try_reserve(bounds.len() * 2)
            .map_err(|_| "BVH allocation")?;
        self.bounds.extend_from_slice(bounds);
        self.order.extend(0..bounds.len());
        if !bounds.is_empty() {
            self.partition(0, bounds.len());
        }
        Ok(())
    }

    fn partition(&mut self, start: usize, end: usize) -> usize {
        // Private ranges originate in build() and are split exactly in half.
        // At most 65536 leaves => <=16 subdivision levels; never native counts.
        let mut bounds = self.bounds[self.order[start]];
        for &index in &self.order[start + 1..end] {
            bounds = bounds.union(self.bounds[index]);
        }
        let branch = self.branches.len();
        self.branches.push(Branch {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start <= LEAF_SIZE {
            return branch;
        }
        let axis =
            usize::from(bounds.upper[1] - bounds.lower[1] > bounds.upper[0] - bounds.lower[0]);
        let middle = (start + end) / 2;
        let all_bounds = &self.bounds;
        self.order[start..end].select_nth_unstable_by(middle - start, |&left, &right| {
            let center = |index: usize| {
                f64::from(all_bounds[index].lower[axis]) + f64::from(all_bounds[index].upper[axis])
            };
            center(left)
                .total_cmp(&center(right))
                .then_with(|| left.cmp(&right))
        });
        let left = self.partition(start, middle);
        let right = self.partition(middle, end);
        self.branches[branch].children = Some((left, right));
        branch
    }

    pub fn candidates(&self, ray: Ray, output: &mut Vec<usize>) -> bool {
        output.clear();
        if !ray.valid() || output.capacity() < self.bounds.len() {
            return false;
        }
        if self.branches.is_empty() {
            return true;
        }
        let mut stack = [0_usize; 32];
        let mut pending = 1;
        while pending != 0 {
            pending -= 1;
            let Some(branch) = self.branches.get(stack[pending]) else {
                return false;
            };
            if !branch.bounds.intersects(ray) {
                continue;
            }
            if let Some((left, right)) = branch.children {
                if pending + 2 > stack.len() {
                    return false;
                }
                stack[pending] = right;
                stack[pending + 1] = left;
                pending += 2;
                continue;
            }
            for &index in &self.order[branch.start..branch.end] {
                if self.bounds[index].intersects(ray) {
                    output.push(index);
                }
            }
        }
        // BVH traversal order is spatial, while native tie-breaking is by the
        // original body-array order then fixture linked-list order.
        output.sort_unstable();
        true
    }
}
