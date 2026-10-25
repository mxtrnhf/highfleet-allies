//! Per-tick log of the AI obstacle scan and the steering numbers the game actually computed (`log_scan`).
//!
//! Hook sites and register maps: `hf_notes/docs/findings/ai/ai-obstacle-hook-sites.md`. Ten inline hooks (no game
//! behaviour changes): inside `AiShip_ScanObstacles` (entry, body considered, mass test, body kept, AddObstacleNode
//! entry) and inside `AiShipTick` (A final, F before the attitude term, bank vectors after the clamp, after the
//! 5-slot filter). Every hook copies a fixed set of registers to [`SNAP`] and calls [`on_site`] with the site id.
//!
//! Files (one pair per battle, named with hf_ally's session id so the `.hftm` packer finds them):
//! * `hf_ai_scan_<id>.csv`: one row per AI ship per scan (50 Hz), the tick summary;
//! * `hf_ai_obst_<id>.csv`: `N` rows (kept nodes), `D` rows (bodies below the mass filter, only when the ship's target
//!   is within 1500), `C` rows (bodies considered, only with `log_scan_bodies`, only on ticks with a kept node).
//!
//! Raw columns are what the game held in a register or field; columns ending `_d` are derived by this mod.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};

use crate::{plausible, rd, CLOCK, OFF_CODE, OFF_OWNER, OFF_TARGET, STEER_BUFFER};

pub static LOG_BODIES: AtomicBool = AtomicBool::new(false);

const OFF_MASS: usize = 0x15b8;

/// A hook site: address, bytes there (= the bytes stolen, replayed after the hook), what the stack looks like.
struct Site {
    id: u32,
    rva: usize,
    bytes: &'static [u8],
    /// RSP is 8 mod 16 here (function entry, before the prologue): the stub realigns.
    entry: bool,
    /// The flags are live across the site: the stub saves them.
    flags: bool,
}

const S_SCAN_ENTRY: u32 = 1;
const S_CONSIDERED: u32 = 2;
const S_MASS: u32 = 3;
const S_KEPT: u32 = 4;
const S_ADD_NODE: u32 = 5;
const S_A_FINAL: u32 = 6;
const S_FORCE: u32 = 7;
const S_BANK1: u32 = 8;
const S_CLAMPED: u32 = 9;
const S_FILTERED: u32 = 10;

const SITES: [Site; 10] = [
    Site { id: S_SCAN_ENTRY, rva: 0x24cc29, bytes: &[0x4c, 0x8b, 0xdc, 0x41, 0x54], entry: true, flags: false },
    Site { id: S_CONSIDERED, rva: 0x24cd46, bytes: &[0x40, 0x32, 0xff, 0x44, 0x0f, 0x2e, 0xc6], entry: false, flags: false },
    Site { id: S_MASS, rva: 0x24cd69, bytes: &[0x44, 0x0f, 0x2f, 0xc0, 0x40, 0x0f, 0xb6, 0xff], entry: false, flags: false },
    Site { id: S_KEPT, rva: 0x24cdb5, bytes: &[0x4d, 0x8b, 0xcc, 0xf3, 0x44, 0x0f, 0x11, 0x4c, 0x24, 0x20], entry: false, flags: false },
    Site { id: S_ADD_NODE, rva: 0x24ce59, bytes: &[0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x10], entry: true, flags: false },
    Site { id: S_A_FINAL, rva: 0x64f58, bytes: &[0xf3, 0x0f, 0x10, 0x55, 0xa0, 0xf3, 0x45, 0x0f, 0x58, 0xc8], entry: false, flags: false },
    Site { id: S_FORCE, rva: 0x65002, bytes: &[0xf3, 0x44, 0x0f, 0x11, 0x54, 0x24, 0x3c, 0x45, 0x0f, 0x28, 0xd0], entry: false, flags: true },
    Site { id: S_BANK1, rva: 0x65233, bytes: &[0x41, 0x0f, 0x28, 0xc3, 0x41, 0x0f, 0x28, 0xc9], entry: false, flags: false },
    Site { id: S_CLAMPED, rva: 0x652d7, bytes: &[0xf3, 0x45, 0x0f, 0x58, 0xa6, 0x1c, 0x0c, 0x00, 0x00], entry: false, flags: false },
    Site { id: S_FILTERED, rva: 0x653ea, bytes: &[0x41, 0xff, 0x86, 0x6c, 0x0c, 0x00, 0x00], entry: false, flags: false },
];

// ---- register snapshot -------------------------------------------------------------------------------------------

/// Slots of 8 bytes: general registers, the original RSP, then the XMM registers in [`XMM`] order (low float).
const GP: [u8; 7] = [0, 1, 2, 3, 9, 5, 14]; // rax rcx rdx rbx r9 rbp r14
const XMM: [u8; 12] = [0, 2, 3, 5, 6, 8, 9, 10, 11, 12, 14, 15];
const SLOT_RSP: usize = 7;
static SNAP: [AtomicU64; 8 + 12] = [const { AtomicU64::new(0) }; 20];

fn gp(slot: usize) -> u64 {
    SNAP[slot].load(Ordering::Relaxed)
}
/// Low float of XMM`n`.
fn xmm(n: u8) -> f32 {
    let index = XMM.iter().position(|&x| x == n).expect("xmm register not in the snapshot");
    f32::from_bits(SNAP[8 + index].load(Ordering::Relaxed) as u32)
}

/// Code that copies the registers into [`SNAP`]; `rsp_extra` = bytes the stub pushed before `preserved_call`.
fn snapshot_setup(site: u32, rsp_extra: u32) -> Vec<u8> {
    let mut code = vec![0x49, 0xba]; // mov r10, imm64
    code.extend_from_slice(&(SNAP.as_ptr() as u64).to_le_bytes());
    let slot = |i: usize| ((i * 8) as u32).to_le_bytes();
    for (i, &reg) in GP.iter().enumerate() {
        // mov [r10+disp32], reg
        code.extend_from_slice(&[0x49 | ((reg >> 3) << 2) | 0x08, 0x89, 0x82 | ((reg & 7) << 3)]);
        code.extend_from_slice(&slot(i));
    }
    // lea r11, [rsp + 0xC0 + rsp_extra]: the RSP the game had at the site; mov [r10+disp32], r11
    code.extend_from_slice(&[0x4c, 0x8d, 0x9c, 0x24]);
    code.extend_from_slice(&(0xc0 + rsp_extra).to_le_bytes());
    code.extend_from_slice(&[0x4d, 0x89, 0x9a]);
    code.extend_from_slice(&slot(SLOT_RSP));
    for (i, &reg) in XMM.iter().enumerate() {
        // movss [r10+disp32], xmmN
        code.extend_from_slice(&[0xf3, 0x41 | ((reg >> 3) << 2), 0x0f, 0x11, 0x82 | ((reg & 7) << 3)]);
        code.extend_from_slice(&slot(8 + i));
    }
    code.push(0xb9); // mov ecx, site
    code.extend_from_slice(&site.to_le_bytes());
    code
}

fn build_trampoline(site: &Site, module: ProcessModule) -> Result<Trampoline, String> {
    let (before, after, extra): (&[u8], &[u8], u32) = match (site.flags, site.entry) {
        (true, _) => (&[0x9c, 0x48, 0x83, 0xec, 0x08], &[0x48, 0x83, 0xc4, 0x08, 0x9d], 16), // pushfq; sub rsp,8
        (false, true) => (&[0x48, 0x83, 0xec, 0x08], &[0x48, 0x83, 0xc4, 0x08], 8),
        (false, false) => (&[], &[], 0),
    };
    let mut trampoline = Trampoline::new();
    trampoline.bytes(before);
    trampoline.preserved_call(on_site as *const (), &snapshot_setup(site.id, extra), &[], ReturnType::None);
    trampoline.bytes(after);
    trampoline.bytes(site.bytes);
    trampoline.absolute_jump(module.resolve_rva(site.rva + site.bytes.len()).map_err(|e| e.to_string())?);
    Ok(trampoline)
}

/// Adds the hooks to `session`; `Ok(false)` when some site has other bytes (nothing is added then).
pub unsafe fn prepare(session: &mut PatchSession, module: ProcessModule) -> Result<bool, String> {
    for site in &SITES {
        let address = module.resolve_rva(site.rva).map_err(|e| e.to_string())?;
        if std::slice::from_raw_parts(address as *const u8, site.bytes.len()) != site.bytes {
            log::error!("hf_ai scan log: site {:#x} has other bytes, no scan log", 0x1_4000_0000 + site.rva);
            return Ok(false);
        }
    }
    for site in &SITES {
        let trampoline = build_trampoline(site, module)?;
        let address = module.resolve_rva(site.rva).map_err(|e| e.to_string())?;
        session.detour_trampoline(address, site.bytes.len(), trampoline).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

// ---- tick state --------------------------------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct V2(Option<(f32, f32)>);

#[derive(Default)]
struct Tick {
    open: bool,
    creature: usize,
    ret: usize,
    t: f64,
    range: f32,
    min_mass: f32,
    max_expl: f32,
    own_pos: V2,
    own_radius: Option<f32>,
    considered: u32,
    mass_below: u32,
    add_calls: u32,
    kept_calls: u32,
    /// (body, mass) of bodies under minMass; a body that is kept anyway is removed again
    below: Vec<(usize, f32)>,
    /// (body, class byte, x, y) of every considered body (only with `log_scan_bodies`)
    bodies: Vec<(usize, u8, f32, f32)>,
    nodes: Option<u32>,
    a: V2,
    dir_t: V2,
    scmax: Option<f32>,
    mass_stack: Option<f32>,
    ts: Option<f32>,
    vel: V2,
    force: V2,
    b2_att: V2,
    b1_cl1: V2,
    b1: V2,
    b2: V2,
    f1: V2,
    f2: V2,
    ring: Option<u64>,
}

struct Out {
    battle: u64,
    active: bool,
    next_check: f64,
    scan: Option<BufWriter<File>>,
    obst: Option<BufWriter<File>>,
    last_flush: Instant,
}

struct State {
    tick: Tick,
    out: Out,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

unsafe extern "C" fn on_site(id: u32) {
    // A Rust panic must never unwind into the game.
    let _ = catch_unwind(AssertUnwindSafe(|| site(id)));
}

fn is_tick_path(ret: usize) -> bool {
    (0x1_4006_2f90..0x1_4006_5f9a).contains(&ret)
}

fn v2(x: f32, y: f32) -> V2 {
    V2(Some((x, y)))
}

unsafe fn site(id: u32) {
    let Ok(mut guard) = STATE.try_lock() else { return };
    let state = guard.get_or_insert_with(|| State {
        tick: Tick::default(),
        out: Out { battle: 0, active: true, next_check: 0.0, scan: None, obst: None, last_flush: Instant::now() },
    });
    let State { tick, out } = state;
    let (rcx, rdx, rbx, rbp, r14, rsp) = (gp(1) as usize, gp(2) as usize, gp(3) as usize, gp(5) as usize, gp(6) as usize, gp(SLOT_RSP) as usize);
    match id {
        S_SCAN_ENTRY => {
            if tick.open {
                finish_row(tick, out, true);
            }
            *tick = Tick::default();
            tick.open = true;
            tick.creature = rcx;
            tick.t = rd::<f64>(CLOCK).unwrap_or(0.0);
            tick.ret = rd::<usize>(rsp).unwrap_or(0);
            tick.range = xmm(2);
            tick.min_mass = xmm(3);
            tick.max_expl = rd::<f32>(rsp + 0x28).unwrap_or(f32::NAN);
        }
        _ if !tick.open => {}
        S_CONSIDERED => {
            tick.considered += 1;
            if LOG_BODIES.load(Ordering::Relaxed) && tick.bodies.len() < 400 {
                tick.bodies.push((rbx, rd(rbx + OFF_CODE).unwrap_or(0), rd(rbx + 0x128).unwrap_or(f32::NAN), rd(rbx + 0x12c).unwrap_or(f32::NAN)));
            }
        }
        S_MASS => {
            if xmm(8) > xmm(0) {
                tick.mass_below += 1;
                if tick.below.len() < 256 {
                    tick.below.push((rbx, xmm(0)));
                }
            }
        }
        S_KEPT => {
            tick.kept_calls += 1;
            if let Some(i) = tick.below.iter().position(|&(body, _)| body == rbx) {
                tick.below.swap_remove(i);
            }
        }
        S_ADD_NODE => {
            tick.add_calls += 1;
            if let (Some(x), Some(y)) = (rd::<f32>(rdx), rd::<f32>(rdx + 4)) {
                tick.own_pos = v2(x, y);
            }
            tick.own_radius = Some(xmm(2));
        }
        S_A_FINAL if r14 == tick.creature && is_tick_path(tick.ret) => {
            tick.a = v2(xmm(5), xmm(6));
            tick.dir_t = v2(xmm(9), xmm(10));
            tick.scmax = Some(xmm(12));
            tick.mass_stack = rd(rbp.wrapping_sub(0x60));
            tick.ts = rd(rbp.wrapping_sub(0x64));
            if let (Some(x), Some(y)) = (rd::<f32>(rsp + 0x74), rd::<f32>(rsp + 0x78)) {
                tick.vel = v2(x, y);
            }
            tick.nodes = rd::<u32>(r14 + 0xd80);
            write_nodes(tick, out);
        }
        S_FORCE if r14 == tick.creature => {
            if let (Some(x), Some(y)) = (rd::<f32>(rsp + 0x38), rd::<f32>(rsp + 0x3c)) {
                tick.force = v2(x, y);
            }
        }
        S_BANK1 if r14 == tick.creature => {
            tick.b1_cl1 = v2(xmm(12), xmm(10));
            tick.b2_att = v2(xmm(11), xmm(9));
        }
        S_CLAMPED if r14 == tick.creature => {
            tick.b1 = v2(xmm(12), xmm(10));
            tick.b2 = v2(xmm(11), xmm(9));
        }
        S_FILTERED if r14 == tick.creature => {
            tick.f1 = v2(xmm(12), xmm(10));
            tick.f2 = v2(xmm(11), xmm(9));
            tick.ring = Some(gp(0));
            finish_row(tick, out, false);
        }
        _ => {}
    }
}

// ---- output ------------------------------------------------------------------------------------------------------

fn step_of(t: f64) -> i64 {
    ((t - 0.04) / 0.02).round() as i64
}

/// Opens (or rotates to) the files of the current battle, `None` when logging is not possible.
fn open_files(out: &mut Out, now: f64) -> bool {
    if now >= out.next_check || now < out.next_check - 5.0 {
        out.next_check = now + 0.5;
        let (battle, active) = crate::battle_state();
        if battle != out.battle || active != out.active {
            out.battle = battle;
            out.active = active;
            for file in [&mut out.scan, &mut out.obst] {
                if let Some(mut writer) = file.take() {
                    let _ = writer.flush();
                }
            }
        }
    }
    if !out.active {
        return false;
    }
    if out.scan.is_some() {
        return true;
    }
    let stamp = if out.battle != 0 {
        out.battle
    } else {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    };
    let open = |kind: &str| std::fs::OpenOptions::new().create(true).append(true).open(format!("Modloader/logs/hf_ai_{kind}_{stamp}.csv")).map(BufWriter::new);
    let (Ok(mut scan), Ok(mut obst)) = (open("scan"), open("obst")) else { return false };
    let version = env!("CARGO_PKG_VERSION");
    let _ = writeln!(
        scan,
        "# hf_ai v{version} AI obstacle scan and steering, one row per AI ship per scan call. t = game clock, step = round((t-0.04)/0.02) (joins with the hook EA rows), cptr = creature pointer. \
         path = tick (AiShipTick inlined scan, return address in 0x140062f90..0x140065f99) / other (any other caller, e.g. AiShip_SteerController; ret = its return address; only the scan counters are filled then). \
         Raw columns come straight from registers/fields at the hook sites (hf_notes docs/findings/ai/ai-obstacle-hook-sites.md); columns ending _d are derived here. partial = 1: the tick ended before the filter site (T1 or T2 = 0, or the next scan began)."
    );
    let _ = writeln!(
        scan,
        "# own_x,own_y,own_r = ownPos/ownRadius as AddObstacleNode got them (bbox centre of the root); vx,vy = own velocity read by the force law; mass = [rbp-0x60]; range,min_mass,max_expl = scan arguments; \
         dirt = unit direction to the waypoint (xmm9/10); wp = creature+0xbf0/0xbf4; wp_dist_d = |wp - ship position of the steering hook|; s_d = clamp(sqrt(dist+0.1)/20,0,1); scmax = s*cmd_max as held (xmm12); ts = [rbp-0x64]; \
         A = avoidance sum(-w*dir) final (xmm5/6, includes the c04 weight if the code applies it); F = force before the attitude term; b2att = bank 2 after the attitude pair, before its clamp (bank 1 is only visible after its clamp: b1c1); \
         b1/b2 = both banks after the clamp; f1/f2 = after the 5-slot filter (these become the engine commands); ring = ring index used. t1,t2 = creature+0x1664/+0x1668, c74 = byte +0xc74, accel_d/cmd_max_d from the spec formulas. \
         nodes = list size at the end of the weight loop, considered = bodies that passed the scan's active/own/class filters, mass_below = of those under minMass at the mass test, kept_calls = bodies sent to AddObstacleNode, \
         add_calls = AddObstacleNode entries (compound children included), drop_mass_d = considered - kept_calls, drop_range_shadow_d = add_calls - nodes."
    );
    let _ = writeln!(
        scan,
        "t,step,cptr,path,ret,own_x,own_y,own_r,vx,vy,mass,range,min_mass,max_expl,dirt_x,dirt_y,wp_x,wp_y,wp_dist_d,s_d,scmax,ts,Ax,Ay,Fx,Fy,b2att_x,b2att_y,b1c1_x,b1c1_y,b1_x,b1_y,b2_x,b2_y,f1_x,f1_y,f2_x,f2_y,ring,t1,t2,accel_d,cmd_max_d,c74,nodes,considered,mass_below,kept_calls,add_calls,drop_mass_d,drop_range_shadow_d,partial"
    );
    let _ = writeln!(obst, "# hf_ai v{version} obstacle rows. kind N = kept node (after shadow culling, weights filled): t,step,cptr,idx,body,class(body+0x28),owner(+0x88),compound(owner if its class&0x1f==0x1f else 0),mass (returnMass),expl (+0x2dc),x,y,vx,vy,r,ux,uy (unit vector ship to obstacle),gap,bearing,sh_lo,sh_hi,w. \
        kind D = body under the mass filter and not kept (only when the ship's target is within 1500): body,class,x,y (origin +0x128/+0x12c),mass. kind C = body considered by the scan (log_scan_bodies; only ticks with a kept node): body,class,x,y.");
    let _ = writeln!(obst, "kind,t,step,cptr,idx,body,class,owner,compound,mass,expl,x,y,vx,vy,r,ux,uy,gap,bearing,sh_lo,sh_hi,w");
    out.scan = Some(scan);
    out.obst = Some(obst);
    true
}

fn maybe_flush(out: &mut Out) {
    if out.last_flush.elapsed().as_millis() >= 1000 {
        out.last_flush = Instant::now();
        for file in [&mut out.scan, &mut out.obst] {
            if let Some(writer) = file.as_mut() {
                let _ = writer.flush();
            }
        }
    }
}

fn opt(buf: &mut String, v: Option<f32>) {
    if let Some(v) = v {
        let _ = write!(buf, "{v}");
    }
    buf.push(',');
}
fn pair(buf: &mut String, v: V2) {
    match v.0 {
        Some((x, y)) => {
            let _ = write!(buf, "{x},{y},");
        }
        None => buf.push_str(",,"),
    }
}

/// The game's `returnMass(body)` (0x1400486e0).
unsafe fn return_mass(body: usize) -> f32 {
    let f: unsafe extern "C" fn(usize) -> f32 = std::mem::transmute(0x1_4004_86e0usize);
    f(body)
}

unsafe fn class_of(body: usize) -> u8 {
    rd(body + OFF_CODE).unwrap_or(0)
}

/// `N` rows for the final node list, then the `D` / `C` rows when they apply.
unsafe fn write_nodes(tick: &Tick, out: &mut Out) {
    if !open_files(out, tick.t) {
        return;
    }
    let step = step_of(tick.t);
    let creature = tick.creature;
    let mut text = String::new();
    let mut count = 0usize;
    if let Some(sentinel) = rd::<usize>(creature + 0xd78).filter(|&p| plausible(p)) {
        let mut node = rd::<usize>(sentinel).unwrap_or(sentinel);
        while node != sentinel && plausible(node) && count < 64 {
            let Some(e) = rd::<usize>(node + 0x10).filter(|&p| plausible(p)) else { break };
            let f = |off: usize| rd::<f32>(e + off).unwrap_or(f32::NAN);
            let body: usize = rd(e).unwrap_or(0);
            let owner: usize = rd(body + OFF_OWNER).unwrap_or(0);
            let compound = if plausible(owner) && class_of(owner) & 0x1f == 0x1f { owner } else { 0 };
            let mass = if plausible(body) { return_mass(body) } else { f32::NAN };
            let expl = rd::<f32>(body + 0x2dc).unwrap_or(f32::NAN);
            let _ = writeln!(
                text,
                "N,{:.3},{step},0x{creature:x},{count},0x{body:x},{},0x{owner:x},0x{compound:x},{mass},{expl},{},{},{},{},{},{},{},{},{},{},{},{}",
                tick.t, class_of(body), f(8), f(0xc), f(0x10), f(0x14), f(0x18), f(0x1c), f(0x20), f(0x24), f(0x28), f(0x2c), f(0x30), f(0x34)
            );
            count += 1;
            node = rd::<usize>(node).unwrap_or(sentinel);
        }
    }
    let near_target = rd::<usize>(creature + OFF_TARGET).and_then(|n| crate::target_info(n)).is_some_and(|(_, (tx, ty))| {
        let (sx, sy) = (f32::from_bits(STEER_BUFFER[2].load(Ordering::Relaxed)), f32::from_bits(STEER_BUFFER[3].load(Ordering::Relaxed)));
        ((tx - sx).powi(2) + (ty - sy).powi(2)).sqrt() < 1500.0
    });
    if near_target {
        for &(body, mass) in tick.below.iter().take(64) {
            let _ = writeln!(
                text,
                "D,{:.3},{step},0x{creature:x},,0x{body:x},{},,,{mass},,{},{}",
                tick.t, class_of(body), rd::<f32>(body + 0x128).unwrap_or(f32::NAN), rd::<f32>(body + 0x12c).unwrap_or(f32::NAN)
            );
        }
    }
    if count > 0 {
        for &(body, class, x, y) in &tick.bodies {
            let _ = writeln!(text, "C,{:.3},{step},0x{creature:x},,0x{body:x},{class},,,,,{x},{y}", tick.t);
        }
    }
    if let Some(file) = out.obst.as_mut() {
        let _ = file.write_all(text.as_bytes());
    }
}

unsafe fn finish_row(tick: &mut Tick, out: &mut Out, partial: bool) {
    tick.open = false;
    if !open_files(out, tick.t) {
        return;
    }
    let c = tick.creature;
    let path = if is_tick_path(tick.ret) { "tick" } else { "other" };
    let mut row = String::new();
    let _ = write!(row, "{:.3},{},0x{c:x},{path},0x{:x},", tick.t, step_of(tick.t), tick.ret);
    pair(&mut row, tick.own_pos);
    opt(&mut row, tick.own_radius);
    pair(&mut row, tick.vel);
    opt(&mut row, tick.mass_stack);
    let _ = write!(row, "{},{},{},", tick.range, tick.min_mass, tick.max_expl);
    pair(&mut row, tick.dir_t);
    let wp = (rd::<f32>(c + 0xbf0), rd::<f32>(c + 0xbf4));
    let ship = (f32::from_bits(STEER_BUFFER[2].load(Ordering::Relaxed)), f32::from_bits(STEER_BUFFER[3].load(Ordering::Relaxed)));
    match wp {
        (Some(x), Some(y)) => {
            let dist = ((x - ship.0).powi(2) + (y - ship.1).powi(2)).sqrt();
            let s = ((dist + 0.1).sqrt() / 20.0).clamp(0.0, 1.0);
            let _ = write!(row, "{x},{y},{dist},{s},");
        }
        _ => row.push_str(",,,,"),
    }
    opt(&mut row, tick.scmax);
    opt(&mut row, tick.ts);
    for v in [tick.a, tick.force, tick.b2_att, tick.b1_cl1, tick.b1, tick.b2, tick.f1, tick.f2] {
        pair(&mut row, v);
    }
    match tick.ring {
        Some(r) => {
            let _ = write!(row, "{},", r & 0xff);
        }
        None => row.push(','),
    }
    let (t1, t2, mass) = (rd::<f32>(c + 0x1664), rd::<f32>(c + 0x1668), rd::<f32>(c + OFF_MASS));
    let c74 = rd::<u8>(c + 0xc74);
    opt(&mut row, t1);
    opt(&mut row, t2);
    match (t1, t2, mass) {
        (Some(t1), Some(t2), Some(m)) if m > 0.0 => {
            let accel = (t1 + t2) / (m * 9.82);
            let a4 = accel.min(4.0);
            let cmd_max = if c74.unwrap_or(0) != 0 && accel < 4.0 { a4 * 40.0 } else { accel * 25.0 };
            let _ = write!(row, "{accel},{cmd_max},");
        }
        _ => row.push_str(",,"),
    }
    match c74 {
        Some(v) => {
            let _ = write!(row, "{v},");
        }
        None => row.push(','),
    }
    match tick.nodes {
        Some(n) => {
            let _ = write!(row, "{n},");
        }
        None => row.push(','),
    }
    let _ = write!(row, "{},{},{},{},", tick.considered, tick.mass_below, tick.kept_calls, tick.add_calls);
    let _ = write!(row, "{},", tick.considered.saturating_sub(tick.kept_calls));
    match tick.nodes {
        Some(n) => {
            let _ = write!(row, "{},", tick.add_calls.saturating_sub(n));
        }
        None => row.push(','),
    }
    let _ = writeln!(row, "{}", u8::from(partial));
    if let Some(file) = out.scan.as_mut() {
        let _ = file.write_all(row.as_bytes());
    }
    maybe_flush(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_site_builds_and_replays_its_bytes_then_jumps_back() {
        for site in &SITES {
            let module = ProcessModule::from_base(0x1_4000_0000);
            let built = build_trampoline(site, module).unwrap().build(0x1_8000_0000).unwrap();
            assert!(built.windows(site.bytes.len()).any(|w| w == site.bytes), "site {}", site.id);
            let tail = usize::from_le_bytes(built[built.len() - 8..].try_into().unwrap());
            assert_eq!(tail, 0x1_4000_0000 + site.rva + site.bytes.len());
            assert!(site.bytes.len() >= 5);
        }
    }

    #[test]
    fn snapshot_setup_dump() {
        std::fs::write(std::env::var("SNAP_OUT").unwrap_or_else(|_| "snap.bin".into()), snapshot_setup(3, 8)).unwrap();
    }
}
