//! Battle telemetry (config `telemetry`, default off). READ-ONLY: it only reads game memory, writes nothing to it,
//! and calls no game function. Every pointer is read through `diag::rd` (page checked) except the plain ship fields.
//!
//! Three CSV files per battle session, all in `Modloader/logs/`, written unbuffered (a crash loses nothing):
//! * `hf_ally_telemetry_<unix>.csv`: ~10 rows/s of GAME time per live ship (`P` = the ship the player flies, `E` = every
//!   other live ship, friend or foe: `side` says which) with position, state, target, weapon / Zenith / flare rounds,
//!   part counts and summed part health, fuel, crew, the AI's steering waypoint, control bytes, and for the player
//!   boost, buttons, the game's raw mouse (and its per-frame sum), zoom, candidate virtual-cursor floats, wave timer.
//!   `M` rows are F9 markers (optional).
//! * `hf_ally_parts_<unix>.csv`: one `S` row per part when a ship is first seen (what it is: `oid` like MDL_CANNON_30_6),
//!   an `H` row whenever a part's health or state changes (this is the DAMAGE log), an `L` row when a part leaves the
//!   ship's part list, and one `N` row per ship listing every text string found inside the ship object
//!   (`offset:text`), because the ship's design name is in there somewhere and we do not know the offset yet.
//! * `hf_ally_bodies_<unix>.csv`: every sample a `C` row with the count of flying bodies by logic id (shells, rockets,
//!   missiles, flares), and a `B` row for each body that has a target (missiles, torpedoes): where it is, how fast, who
//!   it is aimed at.
//!
//! Not logged, on purpose: velocity/acceleration of ships (derive from x,y,t), and anything that is a made-up score.
//! `hp_sum` / `hp_max0` are sums of the game's own per-part health numbers, not a game "ship health" value.

use patchy::{PatchSession, ReturnType, Trampoline};
use std::{
    collections::HashMap,
    fmt::Write as _,
    fs::File,
    io::Write as _,
    sync::atomic::{AtomicBool, Ordering},
    sync::Mutex,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::{
    creature_of, diag::rd, live_ship, part_list, plausible, read, roster, CLOCK, CREATURES_BEGIN, CREATURES_END,
    OFF_AGGRESSION, OFF_AI_FLAG, OFF_FLAGS, OFF_GUNS, OFF_LAUNCH_CD, OFF_MODE, OFF_MODE_D70, OFF_POS_X, OFF_POS_Y,
    OFF_RETARGET, OFF_SIDE, OFF_STATE, OFF_TARGET, OFF_WAYPOINT_X, OFF_WAYPOINT_Y, OFF_ZENITH,
};

const HGE_PTR: usize = 0x1_43a1_19e0;
const HGE_MOUSE_X: usize = 0x9fc;
const HGE_MOUSE_Y: usize = 0xa00;
const ZOOM_LEVEL: usize = 0x1_403c_11cc; // u32
/// Floats the probe saw change continuously with the mouse (0x1403c0dc8/dcc): a candidate virtual cursor. 8 floats from here.
const VC_BASE: usize = 0x1_403c_0dc0;
const WAVE_TIMER: usize = 0x1_47fc_7020; // f32, the battle manager's wave countdown
const BODIES: usize = 0x1_43a3_3c90; // array of Body*
const BODY_COUNT: usize = 0x1_43a5_1150; // u32
const OFF_BOOST: usize = 0xba8; // f32: 2.0 while Shift is held
const OFF_FLARES: (usize, usize) = (0x1408, 0x1410); // flare launcher list, 0x18-byte entries
const OFF_PARTS: (usize, usize) = (0x1330, 0x1338); // all parts of the ship, 0x18-byte entries, Node* at +8
const OFF_FUEL: usize = 0x1638; // f64 current, +8 capacity
const OFF_CREW: usize = 0x17ec; // i32 present, +4 capacity
const OFF_REACH: usize = 0x1838; // f32, +4 second
const OFF_CTL: usize = 0xba0; // 64 raw bytes dumped for the player
const OFF_NAME: usize = 0xa90; // std::string: the ship's design name ("Gladiator", "Kormoran"; found from the N rows 2026-10-18)

const PART_CODE: usize = 0x28;
const PART_STATE: usize = 0x48;
const PART_STATS: usize = 0x188;
const PART_OID: usize = 0x190;
const PART_HP: usize = 0x2c8; // f32, +4 max
const PART_LX: usize = 0x128;
const PART_LY: usize = 0x12c;
const BODY_LOGIC: usize = 0x3c4; // i32
const BODY_TARGET: usize = 0x498; // pointer
const BODY_TTL: usize = 0x2c0; // f32

const VK_F9: i32 = 0x78;
const VK_LBUTTON: i32 = 0x01;
const VK_RBUTTON: i32 = 0x02;
/// Seconds of game time between samples of the other ships and the bodies.
const PERIOD: f64 = 0.1;
/// Seconds of game time between samples of the player's ship (30 Hz: aim and dodge timing).
const PERIOD_PLAYER: f64 = 1.0 / 30.0;
const MAX_SHIPS: usize = 40;
const MAX_PARTS: usize = 600;
const MAX_BODY_ROWS: usize = 80;

pub static ENABLED: AtomicBool = AtomicBool::new(false);

const COLUMNS: &[&str] = &[
    "t", "kind", "id", "side", "state", "x", "y", "target_id", "gun_n", "gun_loaded", "gun_reserve", "zen_n", "zen_loaded",
    "zen_reserve", "fl_n", "fl_loaded", "fl_reserve", "parts_n", "parts_alive", "hp_sum", "hp_max0", "fuel", "fuel_cap",
    "crew", "crew_cap", "aggr", "retarget", "wp_x", "wp_y", "launch_cd", "mode", "role", "ai", "reach1", "reach2",
    "rp_x", "rp_y", "rp_vx", "rp_vy", "rp_ang", "cr_vx", "cr_vy", "cr_ang", "f0", "f1", "f2", "f3", "f4", "name", "gun_ammo",
    // player only from here
    "boost", "lmb", "rmb", "mx", "my", "amx", "amy", "zoom", "vc0", "vc1", "vc2", "vc3", "vc4", "vc5", "vc6", "vc7",
    "wave_t", "ctl",
];

struct ShipTrack {
    /// part pointer -> (part number within the ship, last hp, last state)
    parts: HashMap<usize, (u32, f32, i32)>,
    next_part: u32,
    hp_max0: f32,
}

/// What the per-frame scan last saw of one flying body.
struct Live {
    logic: i32,
    stats: i32,
    last: (f32, f32, f32, f32, f32),
}

struct Session {
    main: File,
    parts: File,
    bodies: File,
    /// Pointer -> small id, so a reused address in a later battle is not mistaken for the same ship.
    ids: Vec<usize>,
    tracks: HashMap<usize, ShipTrack>,
    next: f64,
    /// Next time the other ships and the bodies are sampled (`next` paces the player row).
    next_others: f64,
    marks: u32,
    mark_down: bool,
    /// Sum of the game mouse values read on every frame since the last sample (if they are deltas, this is the movement).
    acc: (f32, f32),
    rows: u64,
    reported_bad: bool,
    /// The ship the player flies and the last sample time: a different ship or a clock gap means a new battle.
    player: usize,
    last: f64,
    /// Flying bodies seen on the previous frame (pointer -> last state): a new pointer is a birth, a vanished one a death.
    live: HashMap<usize, Live>,
    statics_done: bool,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Session {
    fn open(now: f64) -> Option<Session> {
        let stamp = unix_secs();
        let mut main = File::create(format!("Modloader/logs/hf_ally_telemetry_{stamp}.csv")).ok()?;
        let mut parts = File::create(format!("Modloader/logs/hf_ally_parts_{stamp}.csv")).ok()?;
        let mut bodies = File::create(format!("Modloader/logs/hf_ally_bodies_{stamp}.csv")).ok()?;
        let level = unsafe { crate::level_name() };
        let _ = writeln!(
            main,
            "# hf_ally v{} telemetry, level={level}, game clock at start {now:.2}. kinds: P=player ship, E=other ship, M=marker. \
             side: negative = enemy AI, positive = player's side. id: P is always 0. target_id: -1 none/unknown, 0 = the player. \
             f0..f4 = creature control bytes +0xbbc..+0xbc0 (f0 = group-0 guns, LMB; f1 = group-1 guns, RMB; f4 = flares). boost: 2.0 = Shift held. mx,my = the game's own mouse \
             (per-frame deltas, whole numbers); amx,amy = their sum since the last sample. vc0..vc7 = floats at 0x1403c0dc0 \
             (candidate virtual cursor). rp_* = first entry of the ship's part list (position, velocity, angle), cr_* = \
             creature +0x130/+0x134/+0x138 raw. hp_sum = sum of part health of live parts, hp_max0 = sum of max health when \
             first seen. Velocity/acceleration: derive from x,y,t.",
            env!("CARGO_PKG_VERSION"),
        );
        let _ = writeln!(main, "{}", COLUMNS.join(","));
        let _ = writeln!(parts, "# hf_ally v{} parts/damage log. kinds: S=part seen (snapshot), H=health or state changed, L=part left the ship's list, N=text strings found in the ship object (offset:text). ship = id as in the telemetry file (0 = player). part = part number within the ship. state: 0 = dead/dropped.", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(parts, "t,kind,ship,part,oid,stats,state,hp,hp_max,lx,ly");
        let _ = writeln!(bodies, "# hf_ally v{} flying bodies. N = a body is born (first frame its pointer is seen), X = it is gone (last position and velocity read on the previous frame, roughly the impact or expiry point), K = a body of a constant kind (logic 0/200/400/910/3506, level props?) listed once at session start, C = counts by logic id every sample as id:count, B = 10 Hz track of a body with a target (missiles). owner_id / owner2_id: the ship behind the body link +0xb8 / the launcher link +0x4b0 (0 = the player, -1 unknown). vx,vy are units per second.", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(bodies, "t,kind,logic,stats,body,x,y,vx,vy,hp,ttl,target_id,owner_id,owner2_id,oid,counts");
        log::info!("TELEMETRY writing hf_ally_{{telemetry,parts,bodies}}_{stamp}.csv");
        Some(Session {
            main,
            parts,
            bodies,
            ids: Vec::new(),
            tracks: HashMap::new(),
            next: now,
            next_others: now,
            marks: 0,
            mark_down: false,
            acc: (0.0, 0.0),
            rows: 0,
            reported_bad: false,
            player: 0,
            last: now,
            live: HashMap::new(),
            statics_done: false,
        })
    }

    fn id(&mut self, ship: usize) -> i32 {
        match self.ids.iter().position(|&p| p == ship) {
            Some(i) => i as i32 + 1,
            None => {
                self.ids.push(ship);
                self.ids.len() as i32
            }
        }
    }
}

/// Every live ship of the level except `player`.
unsafe fn other_ships(player: usize) -> Vec<usize> {
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    if cb == 0 || ce < cb || (ce - cb) / 0x18 > 512 {
        return Vec::new();
    }
    let mut ships = Vec::new();
    for a in (cb..ce).step_by(0x18) {
        let p: usize = read(a + 8);
        if p != player && live_ship(p) && !ships.contains(&p) {
            ships.push(p);
            if ships.len() >= MAX_SHIPS {
                break;
            }
        }
    }
    ships
}

/// A std::string at `addr` (MSVC layout), if it holds 1..=max printable ASCII characters.
unsafe fn read_string(addr: usize, max: usize) -> Option<String> {
    let len = rd::<usize>(addr + 0x10)?;
    let cap = rd::<usize>(addr + 0x18)?;
    if len == 0 || len > max || cap < len || cap > 0x1000 {
        return None;
    }
    let data = if cap > 0xf { rd::<usize>(addr)? } else { addr };
    if !plausible(data) && cap > 0xf {
        return None;
    }
    let mut s = String::with_capacity(len);
    for i in 0..len {
        let b = rd::<u8>(data + i)?;
        if !(0x20..0x7f).contains(&b) {
            return None;
        }
        s.push(if b == b',' || b == b'"' { ';' } else { b as char });
    }
    Some(s)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Walks the ship's part list, writes S/H/L rows, returns (parts in list, parts alive, hp sum of live parts, hp_max0).
unsafe fn track_parts(s: &mut Session, now: f64, ship: usize, ship_id: i32) -> (usize, usize, f32, f32) {
    let (Some(begin), Some(end)) = (rd::<usize>(ship + OFF_PARTS.0), rd::<usize>(ship + OFF_PARTS.1)) else {
        return (0, 0, 0.0, 0.0);
    };
    if begin == 0 || end < begin || (end - begin) / 0x18 > MAX_PARTS {
        return (0, 0, 0.0, 0.0);
    }
    let first_time = !s.tracks.contains_key(&ship);
    if first_time {
        s.tracks.insert(ship, ShipTrack { parts: HashMap::new(), next_part: 0, hp_max0: 0.0 });
        // every text string inside the ship object: the ship's design name is among them
        let mut found = String::new();
        for off in (0x8..0x1a40).step_by(8) {
            if let Some(text) = read_string(ship + off, 40) {
                let _ = write!(found, "{off:#x}:{text}|");
            }
        }
        let _ = writeln!(s.parts, "{now:.3},N,{ship_id},,{found},,,,,,");
    }
    let (mut n, mut alive, mut hp_sum) = (0usize, 0usize, 0.0f32);
    let mut seen: Vec<usize> = Vec::new();
    let mut new_max = 0.0f32;
    let mut out = String::new();
    for entry in (begin..end).step_by(0x18) {
        let Some(node) = rd::<usize>(entry + 8) else { continue };
        if node == 0 || !plausible(node) {
            continue;
        }
        let Some(code) = rd::<u64>(node + PART_CODE) else { continue };
        if code & 0xf != 0xf {
            continue;
        }
        let (Some(state), Some(hp), Some(hp_max)) =
            (rd::<i32>(node + PART_STATE), rd::<f32>(node + PART_HP), rd::<f32>(node + PART_HP + 4))
        else {
            continue;
        };
        n += 1;
        seen.push(node);
        let is_alive = state != 0 && hp > 0.0;
        if is_alive {
            alive += 1;
            hp_sum += hp;
        }
        let track = s.tracks.get_mut(&ship).unwrap();
        match track.parts.get_mut(&node) {
            None => {
                let number = track.next_part;
                track.next_part += 1;
                track.parts.insert(node, (number, hp, state));
                new_max += hp_max;
                let stats = rd::<i32>(node + PART_STATS).unwrap_or(-1);
                let oid = read_string(node + PART_OID, 48).unwrap_or_default();
                let _ = writeln!(
                    out,
                    "{now:.3},S,{ship_id},{number},{oid},{stats},{state},{hp:.2},{hp_max:.2},{:.1},{:.1}",
                    rd::<f32>(node + PART_LX).unwrap_or(f32::NAN),
                    rd::<f32>(node + PART_LY).unwrap_or(f32::NAN),
                );
            }
            Some(entry) => {
                if (hp - entry.1).abs() > 0.005 || state != entry.2 {
                    let _ = writeln!(
                        out,
                        "{now:.3},H,{ship_id},{},,,{state},{hp:.2},{hp_max:.2},{:.1},{:.1}",
                        entry.0,
                        rd::<f32>(node + PART_LX).unwrap_or(f32::NAN),
                        rd::<f32>(node + PART_LY).unwrap_or(f32::NAN),
                    );
                    entry.1 = hp;
                    entry.2 = state;
                }
            }
        }
    }
    let track = s.tracks.get_mut(&ship).unwrap();
    track.hp_max0 += new_max;
    // parts that vanished from the list since the last sample (never dereferenced: their objects may be gone)
    let gone: Vec<usize> = track.parts.keys().copied().filter(|p| !seen.contains(p)).collect();
    for p in gone {
        if let Some((number, hp, _)) = track.parts.remove(&p) {
            let _ = writeln!(out, "{now:.3},L,{ship_id},{number},,,,{hp:.2},,,");
        }
    }
    let hp_max0 = track.hp_max0;
    let _ = s.parts.write_all(out.as_bytes());
    (n, alive, hp_sum, hp_max0)
}

unsafe fn ship_row(s: &mut Session, now: f64, kind: char, ship: usize, player: usize) -> Vec<String> {
    let id = if kind == 'P' { 0 } else { s.id(ship) };
    let target: usize = read(ship + OFF_TARGET);
    let target_id = if !plausible(target) {
        -1
    } else if target == player || rd::<usize>(target + 0xb8) == Some(player) {
        0
    } else {
        let owner = creature_of(target);
        s.ids.iter().position(|&p| p == owner).map_or(-1, |i| i as i32 + 1)
    };
    let (gn, gl, gr, _) = part_list(ship, OFF_GUNS);
    let (zn, zl, zr, _) = part_list(ship, OFF_ZENITH);
    let (fn_, fl, fr, _) = part_list(ship, OFF_FLARES);
    let (parts_n, parts_alive, hp_sum, hp_max0) = track_parts(s, now, ship, id);
    let flags: [u8; 5] = read(ship + OFF_FLAGS);
    // first entry of the part list: raw position / velocity / angle (what the ship "is" physically)
    let rp: [f32; 5] = rd::<usize>(ship + OFF_PARTS.0)
        .and_then(|b| rd::<usize>(b + 8))
        .filter(|n| plausible(*n))
        .and_then(|n| Some([rd::<f32>(n + 0x128)?, rd::<f32>(n + 0x12c)?, rd::<f32>(n + 0x130)?, rd::<f32>(n + 0x134)?, rd::<f32>(n + 0x138)?]))
        .unwrap_or([f32::NAN; 5]);
    let mut v: Vec<String> = vec![
        format!("{now:.3}"),
        kind.to_string(),
        id.to_string(),
        format!("{:.2}", read::<f32>(ship + OFF_SIDE)),
        read::<i32>(ship + OFF_STATE).to_string(),
        format!("{:.1}", read::<f32>(ship + OFF_POS_X)),
        format!("{:.1}", read::<f32>(ship + OFF_POS_Y)),
        target_id.to_string(),
        gn.to_string(),
        format!("{gl:.0}"),
        gr.to_string(),
        zn.to_string(),
        format!("{zl:.0}"),
        zr.to_string(),
        fn_.to_string(),
        format!("{fl:.0}"),
        fr.to_string(),
        parts_n.to_string(),
        parts_alive.to_string(),
        format!("{hp_sum:.1}"),
        format!("{hp_max0:.1}"),
        format!("{:.0}", read::<f64>(ship + OFF_FUEL)),
        format!("{:.0}", read::<f64>(ship + OFF_FUEL + 8)),
        read::<i32>(ship + OFF_CREW).to_string(),
        read::<i32>(ship + OFF_CREW + 4).to_string(),
        format!("{:.2}", read::<f32>(ship + OFF_AGGRESSION)),
        format!("{:.1}", read::<f32>(ship + OFF_RETARGET)),
        format!("{:.1}", read::<f32>(ship + OFF_WAYPOINT_X)),
        format!("{:.1}", read::<f32>(ship + OFF_WAYPOINT_Y)),
        format!("{:.2}", read::<f32>(ship + OFF_LAUNCH_CD)),
        read::<i32>(ship + OFF_MODE).to_string(),
        read::<i32>(ship + OFF_MODE_D70).to_string(),
        read::<u8>(ship + OFF_AI_FLAG).to_string(),
        format!("{:.0}", read::<f32>(ship + OFF_REACH)),
        format!("{:.0}", read::<f32>(ship + OFF_REACH + 4)),
    ];
    for x in rp {
        v.push(format!("{x:.3}"));
    }
    for off in [0x130usize, 0x134, 0x138] {
        v.push(format!("{:.3}", read::<f32>(ship + off)));
    }
    for f in flags {
        v.push(f.to_string());
    }
    v.push(read_string(ship + OFF_NAME, 40).unwrap_or_default());
    v.push(gun_ammo(ship));
    v
}

const AMMO_LIST_BEGIN: usize = 0x143a13be0; // pointer to the first AmmoDefList entry (stride 0x188, item_name std::string at +8)
const OFF_PART_AMMO: usize = 0x850; // m_weapon_ammo: index into AmmoDefList
const OFF_PART_AMMO_EXTRA: usize = 0x854; // m_weapon_ammo_extra

/// The ammo each live gun is set to, as `index:item_name/extra_index` joined by `|` (`-1` = out of range).
unsafe fn gun_ammo(ship: usize) -> String {
    let (b, e): (usize, usize) = (read(ship + OFF_GUNS.0), read(ship + OFF_GUNS.1));
    if b == 0 || e < b || (e - b) / 0x18 > 64 {
        return String::new();
    }
    let table: usize = read(AMMO_LIST_BEGIN);
    let mut out = Vec::new();
    for a in (b..e).step_by(0x18) {
        let part: usize = read(a + 8);
        if part == 0 || !plausible(part) {
            continue;
        }
        let (idx, extra) = (read::<i32>(part + OFF_PART_AMMO), read::<i32>(part + OFF_PART_AMMO_EXTRA));
        let name = if table != 0 && (0..64).contains(&idx) {
            read_string(table + idx as usize * 0x188 + 8, 40).unwrap_or_default()
        } else {
            String::new()
        };
        out.push(format!("{idx}:{name}/{extra}"));
    }
    out.join("|")
}

unsafe fn mouse() -> (f32, f32) {
    let hge: usize = read(HGE_PTR);
    if !plausible(hge) {
        return (f32::NAN, f32::NAN);
    }
    (rd(hge + HGE_MOUSE_X).unwrap_or(f32::NAN), rd(hge + HGE_MOUSE_Y).unwrap_or(f32::NAN))
}

const STATIC_LOGIC: [i32; 5] = [0, 200, 400, 910, 3506];

/// The ship id behind a link target (a ship or one of its parts); 0 = the player, -1 unknown.
unsafe fn ship_id_of(s: &mut Session, node: usize, player: usize) -> i32 {
    if !plausible(node) {
        return -1;
    }
    let ship = if rd::<u64>(node + PART_CODE).map_or(false, |c| c & 0x2f == 0x2f) { node } else { rd::<usize>(node + 0xb8).unwrap_or(0) };
    if ship == player {
        0
    } else if ship != 0 && live_ship(ship) {
        s.id(ship)
    } else {
        -1
    }
}

/// Every frame: births and deaths of flying bodies (shells, rockets, missiles, flares). Plain reads for the bodies of the
/// array this frame (the game walks the same array every frame); page-checked reads for anything they point at.
unsafe fn scan_bodies(s: &mut Session, now: f64, player: usize) {
    let count = read::<u32>(BODY_COUNT) as usize;
    if count == 0 || count > 4000 {
        return;
    }
    let mut out = String::new();
    let mut present: Vec<usize> = Vec::with_capacity(count);
    for i in 0..count {
        let body: usize = read(BODIES + i * 8);
        if !plausible(body) || read::<u64>(body + PART_CODE) & 0xf != 0xf {
            continue;
        }
        let logic = read::<i32>(body + BODY_LOGIC);
        let stats = read::<i32>(body + PART_STATS);
        let state = (
            read::<f32>(body + PART_LX),
            read::<f32>(body + PART_LY),
            read::<f32>(body + 0x130),
            read::<f32>(body + 0x134),
            read::<f32>(body + BODY_TTL),
        );
        if STATIC_LOGIC.contains(&logic) {
            if !s.statics_done {
                let _ = writeln!(out, "{now:.3},K,{logic},{stats},{body:#x},{:.1},{:.1},,,,,,,,,", state.0, state.1);
            }
            continue;
        }
        present.push(body);
        match s.live.get_mut(&body) {
            Some(l) if l.logic == logic && l.stats == stats => l.last = state,
            other => {
                // an address reused for a different body: close the old one first
                if let Some(l) = other {
                    let _ = writeln!(out, "{now:.3},X,{},{},{body:#x},{:.1},{:.1},{:.1},{:.1},,{:.2},,,,,", l.logic, l.stats, l.last.0, l.last.1, l.last.2, l.last.3, l.last.4);
                }
                let target = read::<usize>(body + BODY_TARGET);
                let target_id = if target == 0 { -1 } else { ship_id_of(s, target, player) };
                let owner1 = ship_id_of(s, read::<usize>(body + 0xb8), player);
                let owner2 = ship_id_of(s, rd::<usize>(body + 0x4b0).unwrap_or(0), player);
                let oid = read_string(body + PART_OID, 48).unwrap_or_default();
                let _ = writeln!(
                    out,
                    "{now:.3},N,{logic},{stats},{body:#x},{:.1},{:.1},{:.1},{:.1},{:.1},{:.2},{target_id},{owner1},{owner2},{oid},",
                    state.0,
                    state.1,
                    state.2,
                    state.3,
                    read::<f32>(body + PART_HP),
                    state.4,
                );
                s.live.insert(body, Live { logic, stats, last: state });
            }
        }
    }
    s.statics_done = true;
    // gone since the previous frame
    let gone: Vec<usize> = s.live.keys().copied().filter(|b| !present.contains(b)).collect();
    for b in gone {
        if let Some(l) = s.live.remove(&b) {
            let _ = writeln!(out, "{now:.3},X,{},{},{b:#x},{:.1},{:.1},{:.1},{:.1},,{:.2},,,,,", l.logic, l.stats, l.last.0, l.last.1, l.last.2, l.last.3, l.last.4);
        }
    }
    if !out.is_empty() {
        let _ = s.bodies.write_all(out.as_bytes());
    }
}

/// Writes one `C` row (counts by logic id) and a `B` row for each targeted body.
unsafe fn write_bodies(s: &mut Session, now: f64, player: usize) {
    let count = rd::<u32>(BODY_COUNT).unwrap_or(0) as usize;
    if count == 0 || count > 4000 {
        return;
    }
    let mut counts: std::collections::BTreeMap<i32, u32> = std::collections::BTreeMap::new();
    let mut out = String::new();
    let mut rows = 0;
    for i in 0..count {
        let Some(body) = rd::<usize>(BODIES + i * 8) else { continue };
        if !plausible(body) {
            continue;
        }
        let Some(code) = rd::<u64>(body + PART_CODE) else { continue };
        if code & 0xf != 0xf {
            continue;
        }
        let logic = rd::<i32>(body + BODY_LOGIC).unwrap_or(-1);
        *counts.entry(logic).or_default() += 1;
        let target = rd::<usize>(body + BODY_TARGET).unwrap_or(0);
        if !plausible(target) || rows >= MAX_BODY_ROWS {
            continue;
        }
        // the link points at a ship or at one of its parts (whose owner is the ship)
        let owner = if rd::<u64>(target + PART_CODE).map_or(false, |c| c & 0x2f == 0x2f) {
            target
        } else {
            rd::<usize>(target + 0xb8).unwrap_or(0)
        };
        let target_id = if owner == player { 0 } else { s.ids.iter().position(|&p| p == owner).map_or(-1, |i| i as i32 + 1) };
        let _ = writeln!(
            out,
            "{now:.3},B,{logic},{},{body:#x},{:.1},{:.1},{:.1},{:.1},{:.1},{:.2},{target_id},{},{},,",
            rd::<i32>(body + PART_STATS).unwrap_or(-1),
            rd::<f32>(body + PART_LX).unwrap_or(f32::NAN),
            rd::<f32>(body + PART_LY).unwrap_or(f32::NAN),
            rd::<f32>(body + 0x130).unwrap_or(f32::NAN),
            rd::<f32>(body + 0x134).unwrap_or(f32::NAN),
            rd::<f32>(body + PART_HP).unwrap_or(f32::NAN),
            rd::<f32>(body + BODY_TTL).unwrap_or(f32::NAN),
            -1,
            -1,
        );
        rows += 1;
    }
    let list: Vec<String> = counts.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    let _ = writeln!(s.bodies, "{now:.3},C,,,,,,,,,,,,,,{}", list.join(" "));
    let _ = s.bodies.write_all(out.as_bytes());
}

/// Called every frame from the battle manager hook. Cheap when disabled or when it is not yet time for a sample.
pub unsafe fn tick() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut guard) = SESSION.try_lock() else { return };
    let now = read::<f64>(CLOCK);
    let ships = roster();
    let player = ships.iter().copied().find(|&p| live_ship(p) && read::<i32>(p + OFF_MODE_D70) == 1 && read::<u8>(p + OFF_AI_FLAG) == 0);

    if ships.is_empty() {
        if let Some(s) = guard.take() {
            log::info!("TELEMETRY battle over, {} rows", s.rows);
        }
        return;
    }
    let Some(player) = player else { return };
    // a different player ship, or the clock jumping, is a new battle even when the roster never emptied
    if guard.as_ref().map_or(false, |s| s.player != player || now < s.last || now - s.last > 2.0) {
        if let Some(old) = guard.take() {
            log::info!("TELEMETRY new battle detected, closing session ({} rows)", old.rows);
        }
    }
    if guard.is_none() {
        *guard = Session::open(now);
    }
    let Some(s) = guard.as_mut() else { return };
    s.player = player;
    s.last = now;

    let (mx, my) = mouse();
    if mx.is_finite() && my.is_finite() {
        s.acc = (s.acc.0 + mx, s.acc.1 + my);
    }
    let down = GetAsyncKeyState(VK_F9) as u16 & 0x8000 != 0;
    if down && !s.mark_down {
        s.marks += 1;
        let _ = writeln!(s.main, "{now:.3},M,{}{}", s.marks, ",".repeat(COLUMNS.len() - 3));
    }
    s.mark_down = down;

    scan_bodies(s, now, player);

    if now < s.next {
        return;
    }
    // a clock that jumped (new battle, pause, load) restarts the cadence instead of writing a burst
    s.next = if now - s.next > 1.0 { now + PERIOD_PLAYER } else { s.next + PERIOD_PLAYER };
    let with_others = now >= s.next_others;
    if with_others {
        s.next_others = if now - s.next_others > 1.0 { now + PERIOD } else { s.next_others + PERIOD };
    }

    let others = if with_others { other_ships(player) } else { Vec::new() };
    // ids first, so a target that is another ship gets its id in the same sample
    for &p in &others {
        s.id(p);
    }
    let mut out = String::with_capacity(512 * 8);
    let emit = |out: &mut String, v: Vec<String>, s: &mut Session| {
        if v.len() != COLUMNS.len() && !s.reported_bad {
            s.reported_bad = true;
            log::info!("TELEMETRY BUG: row has {} fields, header has {}", v.len(), COLUMNS.len());
        }
        out.push_str(&v.join(","));
        out.push('\n');
    };

    let mut p = ship_row(s, now, 'P', player, player);
    let lmb = GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0;
    let rmb = GetAsyncKeyState(VK_RBUTTON) as u16 & 0x8000 != 0;
    p.push(format!("{:.1}", read::<f32>(player + OFF_BOOST)));
    p.push((lmb as u8).to_string());
    p.push((rmb as u8).to_string());
    p.push(format!("{mx:.3}"));
    p.push(format!("{my:.3}"));
    p.push(format!("{:.3}", s.acc.0));
    p.push(format!("{:.3}", s.acc.1));
    p.push(read::<u32>(ZOOM_LEVEL).to_string());
    for i in 0..8 {
        p.push(format!("{:.3}", rd::<f32>(VC_BASE + i * 4).unwrap_or(f32::NAN)));
    }
    p.push(format!("{:.1}", read::<f32>(WAVE_TIMER)));
    p.push(rd::<[u8; 64]>(player + OFF_CTL).map(|b| hex(&b)).unwrap_or_default());
    s.acc = (0.0, 0.0);
    emit(&mut out, p, s);
    for ship in others {
        let mut v = ship_row(s, now, 'E', ship, player);
        v.resize(COLUMNS.len(), String::new());
        emit(&mut out, v, s);
    }
    if s.main.write_all(out.as_bytes()).is_ok() {
        s.rows += out.lines().count() as u64;
    }
    if with_others {
        write_bodies(s, now, player);
    }
}

// ---- dead-node diagnostic (read-only) ----------------------------------------------------------------------------
// The game's per-frame collector (0x1401be250) frees every dead node whose master link (+0x88) is empty. A dead node
// that still has a master makes it call the fatal logger ("ERR: Collect_dead node with m_master = N.", then INT3), which
// crashed the game on 2026-10-18 and left only a stack dump (no heap, so the node's class was lost). These two hooks
// run just before the master test, read the node and, only when it WILL trip the assert, write what it is to
// `Modloader/logs/hf_ally_deadnode.log` (flushed at once) and to the log. They change nothing: the game then aborts as before.
const DEAD_NODE_SITE: usize = 0x1_401b_e274; // cmp qword ptr [rdi+0x88], 0
const DEAD_NODE_BYTES: [u8; 8] = [0x48, 0x83, 0xbf, 0x88, 0x00, 0x00, 0x00, 0x00];
const DEAD_JOINT_SITE: usize = 0x1_401b_e2f8; // cmp qword ptr [rdi+0x58], 0
const DEAD_JOINT_BYTES: [u8; 5] = [0x48, 0x83, 0x7f, 0x58, 0x00];
static DEAD_REPORTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Adds the two detours to `session`; checks the bytes first.
pub unsafe fn install_dead_node_hooks(session: &mut PatchSession) -> Result<(), String> {
    for (site, bytes, hook) in [
        (DEAD_NODE_SITE, &DEAD_NODE_BYTES[..], on_dead_node as *const ()),
        (DEAD_JOINT_SITE, &DEAD_JOINT_BYTES[..], on_dead_joint as *const ()),
    ] {
        let found = std::slice::from_raw_parts(site as *const u8, bytes.len());
        if found != bytes {
            return Err(format!("dead-node site {site:#x} has unexpected bytes {found:02x?}"));
        }
        let mut trampoline = Trampoline::new();
        trampoline.preserved_call(hook, &[0x48, 0x8b, 0xcf], &[], ReturnType::None); // mov rcx, rdi
        trampoline.bytes(bytes);
        trampoline.relative_jump(site + bytes.len());
        session.detour_trampoline(site, bytes.len(), trampoline).map_err(|e| e.to_string())?;
    }
    Ok(())
}

unsafe extern "C" fn on_dead_node(node: usize) {
    if plausible(node) && rd::<usize>(node + 0x88).unwrap_or(0) != 0 {
        report_dead("node", node, 0x88, 0x90);
    }
}

unsafe extern "C" fn on_dead_joint(node: usize) {
    if plausible(node) && rd::<usize>(node + 0x58).unwrap_or(0) != 0 {
        report_dead("joint", node, 0x58, 0x60);
    }
}

/// The class name of an object from its vtable (MSVC x64 RTTI: vtable[-1] = complete object locator).
unsafe fn class_name(object: usize) -> String {
    let Some(vtable) = rd::<usize>(object).filter(|v| plausible(*v)) else { return "?".into() };
    let Some(locator) = rd::<usize>(vtable - 8).filter(|v| plausible(*v)) else { return format!("vtable {vtable:#x} (no rtti)") };
    let Some(type_rva) = rd::<u32>(locator + 0xc) else { return format!("vtable {vtable:#x}") };
    let mut name = String::new();
    for i in 0..80 {
        match rd::<u8>(0x1_4000_0000 + type_rva as usize + 0x10 + i) {
            Some(b) if (0x20..0x7f).contains(&b) => name.push(b as char),
            _ => break,
        }
    }
    format!("{name} (vtable {vtable:#x}, rva {:#x})", vtable.wrapping_sub(0x1_4000_0000))
}

unsafe fn report_dead(kind: &str, node: usize, master_off: usize, master_id_off: usize) {
    if DEAD_REPORTS.fetch_add(1, Ordering::Relaxed) >= 20 {
        return;
    }
    let master = rd::<usize>(node + master_off).unwrap_or(0);
    let mut text = String::new();
    let _ = writeln!(text, "DEADNODE {kind} {node:#x} t={:.2} v{}", read::<f64>(CLOCK), env!("CARGO_PKG_VERSION"));
    let _ = writeln!(text, "  class: {}", class_name(node));
    let _ = writeln!(
        text,
        "  state(+0x48)={:?} m_id(+0x78)={:?} master ptr={master:#x} master id={:?}",
        rd::<i32>(node + 0x48),
        rd::<i64>(node + 0x78),
        rd::<i64>(node + master_id_off)
    );
    let _ = writeln!(text, "  master class: {}", if plausible(master) { class_name(master) } else { "?".into() });
    let _ = writeln!(text, "  name(+0xc8)={:?}", read_string(node + 0xc8, 40));
    // is it a part of a ship in the roster (index in the telemetry numbering is not known here: roster order)?
    for (i, ship) in roster().into_iter().enumerate() {
        let (b, e): (usize, usize) = (read(ship + OFF_PARTS.0), read(ship + OFF_PARTS.1));
        if b == 0 || e < b || (e - b) / 0x18 > 4000 {
            continue;
        }
        for (k, a) in (b..e).step_by(0x18).enumerate() {
            if rd::<usize>(a + 8) == Some(node) {
                let _ = writeln!(text, "  IS part {k} of roster ship {i} ({ship:#x}, name {:?})", read_string(ship + OFF_NAME, 40));
            }
        }
    }
    if let Some(raw) = rd::<[u8; 256]>(node) {
        for (i, chunk) in raw.chunks(32).enumerate() {
            let _ = writeln!(text, "  +{:#05x}: {}", i * 32, hex(chunk));
        }
    }
    log::error!("{}", text.trim_end());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open("Modloader/logs/hf_ally_deadnode.log") {
        let _ = file.write_all(text.as_bytes());
        let _ = file.sync_all();
    }
}
