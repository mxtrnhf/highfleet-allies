//! Carrier plane armament chosen on the pre-combat screen (button `PLANE_LOADOUT_n`, drawn by `lib.rs`).
//!
//! Facts (hf_notes/docs/findings/carrier-plane-loadouts.md, read in Ghidra, not all yet seen in play):
//! * Planes are parts (logic id 500) of the carrier creature: Link vector `+0x14c8..+0x14d0` (stride 0x18, the part at
//!   `+8`), count `+0x17dc`. Per plane part: craft name `+0x190`, loadout name `+0x3f8` (a MSVC string).
//! * Every launch (the player's key, AI carriers via AiShipTick) enters `0x14002e930(plane, owner, ..)`, which looks the
//!   loadout name up in the game's table. Writing a valid `LOADOUT_<craft>_<arm>` name before that makes the plane carry
//!   that armament. Vanilla consumes no inventory for it; this mod charges the profile inventory (`Profile +0x540`,
//!   a `std::map<string,int>`, the int at node `+0x40`) once per launch, and falls back to the gun-only loadout when the
//!   stock is short.
//! * The loadout table (`PLANE_LOADOUTS`) and the craft list (`PLANE_TLL`) are filled once at startup, from code
//!   (findings section 14). The armaments offered here are read from the live tables the first time they are needed, so
//!   loadouts that another mod (or the qol `planes` option) added or re-priced follow; the vanilla list below is only the
//!   fallback when the tables cannot be read. The crafts are read the same way (vanilla has T-7 and LA-29; qol `planes` can
//!   register more, up to `MAX_CRAFTS`), and an item's class (bomb, rocket, K-13) comes from the game's ammo list.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex, OnceLock,
    },
};

use patchy::{PatchSession, ReturnType, Trampoline};

const ARM_FN: usize = 0x1_4002_e930;
/// `mov rax,rsp; push rbp,rbx,rsi,rdi,r12,r13,r14,r15` (15 bytes, whole instructions, no RIP-relative operand).
const ARM_BYTES: [u8; 15] = [0x48, 0x8b, 0xc4, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57];
/// `std::string::operator=(const string&)`: RCX = destination, RDX = source, returns the destination.
const COPY_ASSIGN: usize = 0x1_4001_7e20;
const GET_PROFILE: usize = 0x1_4029_2a90;

const OFF_CRAFT: usize = 0x190;
const OFF_LOADOUT: usize = 0x3f8;
/// The plane's AircraftLogic; its +0x128 / +0x138 are the bomb / rocket entry counts, +0x190 its state.
const OFF_LOGIC: usize = 0xa20;
const CAMPAIGN_BYTES: (usize, usize) = (0x1_47fc_6f80, 0x1_47fc_6f81);
const OFF_PLANES: (usize, usize) = (0x14c8, 0x14d0);
/// Profile API (win64 fastcall, RCX = profile): owned count / remove, by item name ("ITEM_FAB100"). The ammo shop uses the same.
const PROFILE_OWNED: usize = 0x1_4028_c460;
const PROFILE_REMOVE: usize = 0x1_4029_3ad0;
/// `GetPropertyLinkedObject(Property*)` -> the ship creature (what `precombatOk` does with a card's link).
const LINKED_OBJECT: usize = 0x1_4021_b070;

/// What a loadout does once armed; decides which of this module's behaviours it gets.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bombs,
    Rockets,
    /// carries an air-to-air missile (ammo class 5)
    Aa,
    /// parts this module knows nothing about: armed and charged, the game's own flight script
    Other,
}

/// One armament: a loadout of the game's table with parts (the guns-only loadouts have none).
pub struct Arm {
    /// the loadout name (`LOADOUT_T7_FAB100`)
    pub key: String,
    /// the craft whose vector lists it (`CRAFT_T7`)
    craft: String,
    pub label: String,
    /// (item name, count) of one plane
    parts: Vec<(String, i32)>,
    kind: Kind,
}

/// A plane type of the game's craft table (`PLANE_TLL`): vanilla T-7 and LA-29, and whatever a mod registered (qol `planes`).
pub struct Craft {
    /// the game name (`CRAFT_SB15`)
    pub name: String,
    /// the name on the button label (`SB-15`)
    pub label: String,
    /// the loadout that arms the plain gun: the craft's own loadout without parts, else a name the game does not know
    /// (an unknown name arms the plain 37 mm gun, findings section 14.4)
    guns_only: String,
}

/// Config `plane_k13`: offer the K-13 air-to-air choice on the carrier buttons (off by default, it is janky).
pub static K13_ON: AtomicBool = AtomicBool::new(false);

/// Plane types that get a button per card: one column of full-size buttons, six rows fit under the ALLY row.
pub const MAX_CRAFTS: usize = 6;

/// `PLANE_TLL`: `{ head*, size }` of an MSVC `std::map` CRAFT_* -> Plane (findings section 14).
const PLANE_TLL: usize = 0x1_43a1_3c50;
/// Craft node: key at `+0x20`, the loadout pointer vector (each points at a `GameLoadout`) at `+0x48/+0x50`.
const TLL_VEC: (usize, usize) = (0x48, 0x50);
/// `GameLoadout`: oid string `+0`, `vec_parts` begin / end `+0x40` / `+0x48`, stride 0x28 (name `+0`, count `+0x20`).
const LO_PARTS: (usize, usize) = (0x40, 0x48);
const PART_STRIDE: usize = 0x28;
const PART_COUNT: usize = 0x20;
/// The game's ammo list (qol `plane_guns.rs`, 1.163): `{ begin*, end* }` of `Ammo` records of 0x188 bytes, `reticle` (the
/// class: 2 bombs, 3 rockets, 4 gun, 5 K-13) as the first int, the item name string at `+8`.
const AMMO_LIST: usize = 0x1_43a1_3be0;
const AMMO_SIZE: usize = 0x188;

/// The vanilla table, used when the live one cannot be read: (loadout suffix, item, T-7 count, LA-29 count).
const VANILLA: [(&str, &str, i32, i32); 5] = [
    ("FAB100", "ITEM_FAB100", 4, 2),
    ("FAB250", "ITEM_FAB250", 2, 1),
    ("NURS122", "ITEM_NAR122", 12, 6),
    ("NURS340", "ITEM_NAR340", 2, 0),
    ("K13", "ITEM_K13", 2, 0),
];

/// Button sprites (round icons of the old order menu): guns only / an armament chosen.
pub const SPRITE_DEFAULT: &[u8] = b"order_unset\0";
pub const SPRITE_CHOSEN: &[u8] = b"order_bombing_on\0";

/// The ammo class of an item: from the game's ammo list, else guessed from the vanilla names.
fn class_of(item: &str, classes: &HashMap<String, i32>) -> Option<i32> {
    classes.get(item).copied().or_else(|| {
        if item.starts_with("ITEM_FAB") {
            Some(2)
        } else if item.starts_with("ITEM_NAR") {
            Some(3)
        } else if item == "ITEM_K13" {
            Some(5)
        } else {
            None
        }
    })
}

fn classify(parts: &[(String, i32)], classes: &HashMap<String, i32>) -> Kind {
    let has = |class: i32| parts.iter().any(|(n, _)| class_of(n, classes) == Some(class));
    if has(5) {
        Kind::Aa
    } else if has(2) {
        Kind::Bombs
    } else if has(3) {
        Kind::Rockets
    } else {
        Kind::Other
    }
}

/// "ITEM_FAB250" -> "BOMBS 250", "ITEM_NAR122" -> "ROCKETS 122", "ITEM_K13" -> "AA K-13", else the name without "ITEM_".
fn item_label(item: &str) -> String {
    if let Some(n) = item.strip_prefix("ITEM_FAB") {
        format!("BOMBS {n}")
    } else if let Some(n) = item.strip_prefix("ITEM_NAR") {
        format!("ROCKETS {n}")
    } else if item == "ITEM_K13" {
        "AA K-13".to_string()
    } else {
        item.strip_prefix("ITEM_AMMO_").or_else(|| item.strip_prefix("ITEM_")).unwrap_or(item).to_string()
    }
}

/// "CRAFT_T7" -> "T-7", "CRAFT_LA29" -> "LA-29", "CRAFT_SB15" -> "SB-15", "CRAFT_X" -> "X".
fn craft_label(name: &str) -> String {
    let short = name.strip_prefix("CRAFT_").unwrap_or(name);
    match short.find(|c: char| c.is_ascii_digit()) {
        Some(i) if i > 0 && short[..i].chars().all(|c| c.is_ascii_alphabetic()) => format!("{}-{}", &short[..i], &short[i..]),
        _ => short.to_string(),
    }
}

fn make_arm(craft: &str, key: String, parts: Vec<(String, i32)>, classes: &HashMap<String, i32>) -> Arm {
    let label = parts.iter().map(|(n, _)| item_label(n)).collect::<Vec<_>>().join(" + ");
    Arm { kind: classify(&parts, classes), key, craft: craft.to_string(), label, parts }
}

fn make_craft(name: &str, guns_only: Option<String>) -> Craft {
    let guns_only = guns_only.unwrap_or_else(|| format!("LOADOUT_{}_GUNS", name.strip_prefix("CRAFT_").unwrap_or(name)));
    Craft { name: name.to_string(), label: craft_label(name), guns_only }
}

/// What the game's own vanilla table holds, used when the live one cannot be read.
fn vanilla_table() -> Table {
    let classes = HashMap::new();
    let (mut crafts, mut arms) = (Vec::new(), Vec::new());
    for (craft, short) in [("CRAFT_T7", "T7"), ("CRAFT_LA29", "LA29")] {
        crafts.push(make_craft(craft, Some(format!("LOADOUT_{short}_GUN37"))));
        for (suffix, item, t7, la29) in VANILLA {
            let count = if craft == "CRAFT_T7" { t7 } else { la29 };
            if count > 0 {
                arms.push(make_arm(craft, format!("LOADOUT_{short}_{suffix}"), vec![(item.to_string(), count)], &classes));
            }
        }
    }
    Table { crafts, arms }
}

/// The in-order nodes of an MSVC `std::map` global (`{ head*, size }`); empty when it looks unfilled or unreadable.
unsafe fn map_nodes(global: usize) -> Vec<usize> {
    let (head, size): (usize, usize) = (rd(global), rd(global + 8));
    if size == 0 || size > 4096 || !crate::plausible(head) {
        return Vec::new();
    }
    let isnil = |n: usize| rd::<u8>(n + 0x19) != 0;
    let mut out = Vec::new();
    let mut node: usize = rd(head); // head->left = the leftmost node
    while node != head && out.len() <= size {
        if !crate::plausible(node) {
            return Vec::new();
        }
        out.push(node);
        let right: usize = rd(node + 0x10);
        if crate::plausible(right) && !isnil(right) {
            node = right;
            loop {
                let left: usize = rd(node);
                if !crate::plausible(left) || isnil(left) {
                    break;
                }
                node = left;
            }
        } else {
            let mut parent: usize = rd(node + 8);
            while crate::plausible(parent) && !isnil(parent) && node == rd::<usize>(parent + 0x10) {
                node = parent;
                parent = rd(node + 8);
            }
            node = parent;
        }
    }
    if out.len() > size {
        return Vec::new();
    }
    out
}

/// item name -> ammo class, from the game's ammo list (empty when it cannot be read: then the vanilla names decide).
unsafe fn ammo_classes() -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let (b, e): (usize, usize) = (rd(AMMO_LIST), rd(AMMO_LIST + 8));
    if b < 0x10000 || e < b || (e - b) % AMMO_SIZE != 0 || (e - b) / AMMO_SIZE > 10_000 || !crate::plausible(b) {
        return out;
    }
    for rec in (b..e).step_by(AMMO_SIZE) {
        if let Some(name) = string_at(rec + 8) {
            out.insert(name, rd::<i32>(rec));
        }
    }
    out
}

/// The crafts and armaments read from the live tables: every craft in `PLANE_TLL` (the two vanilla ones first, so their
/// button places stay as they were) and every loadout in its vector that has parts. `None` when the craft map is empty
/// or unreadable (the tables are filled by `PrepareMainLoop`, and qol's `planes` option replaces them after that).
unsafe fn read_table() -> Option<Table> {
    let nodes = map_nodes(PLANE_TLL);
    if nodes.is_empty() {
        return None;
    }
    let classes = ammo_classes();
    let mut named: Vec<(String, usize)> = nodes.iter().filter_map(|&n| Some((string_at(n + 0x20)?, n))).collect();
    named.sort_by_key(|(name, _)| match name.as_str() {
        "CRAFT_T7" => 0,
        "CRAFT_LA29" => 1,
        _ => 2,
    });
    named.truncate(MAX_CRAFTS);
    let (mut crafts, mut arms) = (Vec::new(), Vec::new());
    for (craft, node) in named {
        let (b, e): (usize, usize) = (rd(node + TLL_VEC.0), rd(node + TLL_VEC.1));
        if b < 0x10000 || e < b || (e - b) / 8 > 256 {
            return None;
        }
        let mut guns_only: Option<String> = None;
        for lo in (b..e).step_by(8).map(|a| rd::<usize>(a)) {
            if !crate::plausible(lo) {
                return None;
            }
            let (pb, pe): (usize, usize) = (rd(lo + LO_PARTS.0), rd(lo + LO_PARTS.1));
            let key = string_at(lo).filter(|_| pe >= pb && (pe - pb) / PART_STRIDE <= 16)?;
            let parts: Vec<(String, i32)> = (pb..pe)
                .step_by(PART_STRIDE)
                .filter_map(|p| Some((string_at(p)?, rd::<i32>(p + PART_COUNT))))
                .filter(|(n, c)| !n.is_empty() && *c > 0)
                .collect();
            if parts.is_empty() {
                if guns_only.is_none() || key.ends_with("GUN37") {
                    guns_only = Some(key);
                }
            } else {
                arms.push(make_arm(&craft, key, parts, &classes));
            }
        }
        crafts.push(make_craft(&craft, guns_only));
    }
    Some(Table { crafts, arms })
}

struct Table {
    crafts: Vec<Craft>,
    arms: Vec<Arm>,
}

static LIVE_TABLE: OnceLock<Table> = OnceLock::new();
static VANILLA_TABLE: OnceLock<Table> = OnceLock::new();

/// The crafts and armaments, indexed by the `usize` choices of this module. The live table is read once, the first time
/// it is readable (a choice index must not move afterwards); until then, and if it never is, the vanilla list.
fn table() -> &'static Table {
    if let Some(t) = LIVE_TABLE.get() {
        return t;
    }
    if let Some(t) = unsafe { read_table() } {
        let t = LIVE_TABLE.get_or_init(|| t);
        log::info!(
            "PLANE table read: crafts {:?}, armaments {:?}",
            t.crafts.iter().map(|c| (c.name.as_str(), c.guns_only.as_str())).collect::<Vec<_>>(),
            t.arms.iter().map(|x| (x.key.as_str(), x.kind)).collect::<Vec<_>>()
        );
        return t;
    }
    VANILLA_TABLE.get_or_init(vanilla_table)
}

pub fn arms() -> &'static [Arm] {
    &table().arms
}

pub fn crafts() -> &'static [Craft] {
    &table().crafts
}

/// The label drawn beside the button of plane type `craft`: what is chosen and how many of it the player still owns.
pub unsafe fn label_for(craft: usize, choice: Option<usize>) -> String {
    match choice {
        None => format!("{}: GUNS ONLY", crafts()[craft].label),
        Some(a) => {
            let arm = &arms()[a];
            let stock = arm.parts.iter().map(|(n, _)| owned(n).to_string()).collect::<Vec<_>>().join("/");
            format!("{}: {} [{}]", crafts()[craft].label, arm.label, stock)
        }
    }
}

impl Arm {
    /// This armament is offered for planes of `craft` (the game name).
    fn offered(&self, craft: &str) -> bool {
        if self.craft != craft {
            return false;
        }
        !(self.kind == Kind::Aa && !K13_ON.load(Ordering::Relaxed)) // `plane_k13` off: the K-13 choice is not offered
    }

    /// The player's stock covers one plane.
    unsafe fn affordable(&self) -> bool {
        self.parts.iter().all(|(n, c)| owned(n) >= *c)
    }

    fn short_text(&self) -> String {
        self.parts.iter().map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>().join(", ")
    }
}

/// (ship creature, index into `crafts()`, index into `arms()`) chosen on the pre-combat screen.
static CHOICE: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());
/// (plane part, game time of its last charge): the same launch must not be charged twice.
static CHARGED: Mutex<Vec<(usize, f64)>> = Mutex::new(Vec::new());
/// (plane, game time to look again, name we wrote): the log shows whether the game left our name alone.
static VERIFY: Mutex<Vec<(usize, f64, String)>> = Mutex::new(Vec::new());
/// Seconds after the arming before a bomb / rocket plane starts its attack runs (`plane_attack_delay`): they take off and fly
/// the game's own script first. Without it LA-29 bombers dived straight into the carrier's deck.
pub static ATTACK_DELAY_S: AtomicU32 = AtomicU32::new(6.0f32.to_bits());
/// `extra_gunpod` config: in a normal tactical battle only, add the 37 mm gun to bomb / rocket planes and keep them
/// in the fight with it after the payload is gone.
pub static EXTRA_GUNPOD: AtomicBool = AtomicBool::new(false);
/// `stateSelectorAircraft_` (i32): 4 = the normal tactical battle (`Combat.seria`) mode 6; 2/3 = the scripted
/// bomber / tutorial scenes (`Bombers.seria` included), 0/1 = the crafts-vs-* scenes.
const STATE_SELECTOR_AIRCRAFT: usize = 0x1_403b_d008;
/// `BOOL_147fc6fdb`: set by the scripted bomber / tutorial setup, 0 in a normal tactical battle.
const SCENARIO_AIRCRAFT: usize = 0x1_47fc_6fdb;
/// Planes armed with a chosen bomb / rocket loadout: (plane, true = bombs, gunpod eligible, list had entries once, game
/// time of the arming). When both lists are empty the plane is sent out (state 5), or with `extra_gunpod` switches to the
/// gun until it is dry.
static WATCH: Mutex<Vec<(usize, bool, bool, bool, f64)>> = Mutex::new(Vec::new());
/// The aircraft state setter `FUN_14000ebb0(logic, state)`; 5 = TO_OUT_OF_SCREEN.
const SET_STATE: usize = 0x1_4000_ebb0;
/// Plane parts armed with the K-13 choice. The normal battle mode (`logic+0x118 == 6`) never calls the air-to-air launcher
/// `FUN_14000f5b0`, so `on_update` calls it for these planes (notes: carrier-plane-loadouts.md section 13).
static AA: Mutex<Vec<(usize, usize, f64)>> = Mutex::new(Vec::new());
const UPDATE_FN: usize = 0x1_4000_b370;
const UPDATE_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20];
const AA_LAUNCH: usize = 0x1_4000_f5b0;
static AA_LAST: Mutex<Vec<(usize, i32)>> = Mutex::new(Vec::new());
static AA_TICKS: AtomicU32 = AtomicU32::new(0);
static AA_LOGS: AtomicU32 = AtomicU32::new(0);
static ARM_CALLS: AtomicU32 = AtomicU32::new(0);
static CHARGES: AtomicU32 = AtomicU32::new(0);
/// Launches closer together than this (game seconds) on one plane count as one.
const SAME_LAUNCH_S: f64 = 3.0;

unsafe fn rd<T: Copy>(a: usize) -> T {
    (a as *const T).read_unaligned()
}

/// A MSVC string at `addr` (inline when capacity < 16), `None` when it does not look like one.
unsafe fn string_at(addr: usize) -> Option<String> {
    let (len, cap): (usize, usize) = (rd(addr + 0x10), rd(addr + 0x18));
    if len > 200 || cap < len {
        return None;
    }
    let data = if cap < 16 {
        addr
    } else {
        let p: usize = rd(addr);
        if !crate::plausible(p) {
            return None;
        }
        p
    };
    Some(String::from_utf8_lossy(std::slice::from_raw_parts(data as *const u8, len)).into_owned())
}

/// The plane parts of a carrier creature.
unsafe fn planes_of(ship: usize) -> Vec<usize> {
    // a creature (class code with bits 0x2f), not any pointer a stale card link may hold
    if !crate::plausible(ship) || rd::<u8>(ship + 0x28) & 0x2f != 0x2f {
        return Vec::new();
    }
    let (b, e): (usize, usize) = (rd(ship + OFF_PLANES.0), rd(ship + OFF_PLANES.1));
    if b < 0x10000 || e < b || (e - b) / 0x18 > 64 {
        return Vec::new();
    }
    (b..e).step_by(0x18).map(|a| rd::<usize>(a + 8)).filter(|p| crate::plausible(*p)).collect()
}

unsafe fn profile() -> usize {
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(GET_PROFILE);
    get_profile()
}

/// Items of `item` the player owns (0 when none).
pub unsafe fn owned(item: &str) -> i32 {
    let profile = profile();
    if !crate::plausible(profile) {
        return 0;
    }
    let mut name = item.as_bytes().to_vec();
    name.push(0);
    let f: unsafe extern "win64" fn(usize, *const u8) -> i32 = std::mem::transmute(PROFILE_OWNED);
    f(profile, name.as_ptr())
}

/// Takes `count` of `item` from the player's stock (the shop's own call).
unsafe fn take(item: &str, count: i32) {
    let profile = profile();
    if !crate::plausible(profile) {
        return;
    }
    let mut name = item.as_bytes().to_vec();
    name.push(0);
    let f: unsafe extern "win64" fn(usize, *const u8, i32) = std::mem::transmute(PROFILE_REMOVE);
    f(profile, name.as_ptr(), count);
}

/// The ship creature behind a card's link value (`PRECOMBAT_A +0xbc0`, a Property pointer), 0 when it is not one.
pub unsafe fn ship_of_link(link: usize) -> usize {
    if (link >> 47) != 0 && (link >> 47) != 0x1ffff || !crate::plausible(link) {
        return 0;
    }
    let f: unsafe extern "win64" fn(usize) -> usize = std::mem::transmute(LINKED_OBJECT);
    f(link)
}

/// Armaments (indices into `arms()`) the player can afford for one plane of type `craft` (index into `crafts()`) of `ship`.
pub unsafe fn available_arms(ship: usize, craft: usize) -> Vec<usize> {
    if !has_craft(ship, craft) {
        return Vec::new();
    }
    let arms = arms();
    (0..arms.len()).filter(|&i| arms[i].offered(&crafts()[craft].name) && arms[i].affordable()).collect()
}

/// The ship carries at least one plane of type `craft`.
pub unsafe fn has_craft(ship: usize, craft: usize) -> bool {
    planes_of(ship).iter().any(|p| string_at(*p + OFF_CRAFT).as_deref() == Some(crafts()[craft].name.as_str()))
}

/// One line for the log: why the button is or is not shown for `ship` (plane parts and crafts, the profile inventory).
pub unsafe fn describe(ship: usize) -> String {
    if !crate::plausible(ship) || rd::<u8>(ship + 0x28) & 0x2f != 0x2f {
        return format!("card link gives {ship:#x}, not a creature");
    }
    let planes = planes_of(ship);
    let on_ship: Vec<String> = planes.iter().filter_map(|p| string_at(*p + OFF_CRAFT)).collect();
    let available: Vec<String> = (0..crafts().len())
        .map(|c| format!("{} {:?}", crafts()[c].label, available_arms(ship, c).iter().map(|&a| arms()[a].key.as_str()).collect::<Vec<_>>()))
        .collect();
    let (count, begin, end): (u32, usize, usize) = (rd(ship + 0x17dc), rd(ship + OFF_PLANES.0), rd(ship + OFF_PLANES.1));
    let inv: Vec<String> = arms().iter().flat_map(|a| a.parts.iter().map(|(n, _)| format!("{n}={}", owned(n)))).collect();
    format!(
        "ship {ship:#x} class {:#x} plane_count(+0x17dc)={count} vector {begin:#x}..{end:#x} parts {} crafts {:?} available {} | stock [{}]",
        rd::<u8>(ship + 0x28),
        planes.len(),
        on_ship,
        available.join(", "),
        inv.join(" ")
    )
}

pub unsafe fn has_planes(ship: usize) -> bool {
    !planes_of(ship).is_empty()
}

pub fn choice(ship: usize, craft: usize) -> Option<usize> {
    CHOICE.lock().ok()?.iter().find(|(s, c, _)| *s == ship && *c == craft).map(|(_, _, a)| *a)
}

/// Any plane type of `ship` has a choice.
pub fn any_choice(ship: usize) -> bool {
    CHOICE.lock().map_or(false, |c| c.iter().any(|(s, _, _)| *s == ship))
}

pub fn set_choice(ship: usize, craft: usize, arm: Option<usize>) {
    if let Ok(mut c) = CHOICE.lock() {
        c.retain(|(s, k, _)| !(*s == ship && *k == craft));
        if let Some(a) = arm {
            c.push((ship, craft, a));
        }
    }
}

pub fn clear() {
    if let Ok(mut c) = AA.lock() {
        c.clear();
    }
    if let Ok(mut c) = WATCH.lock() {
        c.clear();
    }
    if let Ok(mut c) = CHOICE.lock() {
        c.clear();
    }
    if let Ok(mut c) = CHARGED.lock() {
        c.clear();
    }
    if let Ok(mut c) = NO_DRAW.lock() {
        c.clear();
    }
}

/// The next choice when the button is clicked: none -> first affordable -> ... -> none.
pub unsafe fn next_choice(ship: usize, craft: usize) -> Option<usize> {
    let available = available_arms(ship, craft);
    match choice(ship, craft) {
        Some(current) => available.into_iter().find(|&a| a > current),
        None => available.first().copied(),
    }
}

/// Writes `name` into the plane's loadout string with the game's own copy-assign (its allocator, its buffer rules).
unsafe fn write_loadout(plane: usize, name: &str) {
    let mut text = name.as_bytes().to_vec();
    text.push(0);
    // a string object in MSVC layout that only the game reads: inline up to 15 bytes, else a pointer to `text`
    let mut src = [0u8; 0x20];
    if name.len() < 16 {
        src[..text.len()].copy_from_slice(&text);
        src[0x10..0x18].copy_from_slice(&(name.len() as u64).to_le_bytes());
        src[0x18..0x20].copy_from_slice(&15u64.to_le_bytes());
    } else {
        src[..8].copy_from_slice(&(text.as_ptr() as u64).to_le_bytes());
        src[0x10..0x18].copy_from_slice(&(name.len() as u64).to_le_bytes());
        src[0x18..0x20].copy_from_slice(&(name.len() as u64).to_le_bytes());
    }
    let assign: unsafe extern "win64" fn(*mut u8, *const u8) -> *mut u8 = std::mem::transmute(COPY_ASSIGN);
    assign((plane + OFF_LOADOUT) as *mut u8, src.as_ptr());
}

/// Entry hook of the plane arming function (RCX = plane part, RDX = owner ship). Runs for every launch of every side.
pub unsafe extern "C" fn on_arm(plane: usize, owner: usize) {
    let _hook = crate::diag::scope("plane_arm");
    let n = ARM_CALLS.fetch_add(1, Ordering::Relaxed);
    let Ok(choices) = CHOICE.try_lock() else { return };
    let craft_name = string_at(plane + OFF_CRAFT);
    let picked = choices
        .iter()
        .find(|(s, c, _)| *s == owner && craft_name.as_deref() == Some(crafts()[*c].name.as_str()))
        .map(|(_, _, a)| *a);
    if n < 40 || picked.is_some() {
        log::info!(
            "PLANE arm #{n}: plane={plane:#x} owner={owner:#x} craft={:?} loadout now={:?} chosen={:?}",
            string_at(plane + OFF_CRAFT),
            string_at(plane + OFF_LOADOUT),
            picked.map(|a| arms()[a].key.as_str())
        );
    }
    let Some(arm_i) = picked else {
        drop(choices);
        // no choice = "GUNS ONLY": the game would otherwise roll a loadout from the weights in `qol.json` (bombs, rockets, K-13)
        if crate::ARMED.load(Ordering::Relaxed) && crate::plausible(owner) && rd::<f32>(owner + 0xad4) > 0.0 && rd::<usize>(plane + OFF_LOGIC) == 0 {
            if let Some(c) = crafts().iter().find(|c| Some(c.name.as_str()) == craft_name.as_deref()) {
                write_loadout(plane, &c.guns_only);
                no_rockets(plane);
                log::info!("PLANE arm #{n}: no choice, {} set to {} (guns only)", c.name, c.guns_only);
            }
        }
        return;
    };
    drop(choices);
    // the game arms a plane only while it has no AircraftLogic (+0xa20); otherwise the loadout name is ignored
    if rd::<usize>(plane + OFF_LOGIC) != 0 {
        // still keep the vanilla class-3 stock draw off it: the chosen loadout is what it carries
        no_rockets(plane);
        log::info!("PLANE arm: plane {plane:#x} already has its logic, loadout ignored by the game: nothing charged");
        return;
    }
    if !crate::ARMED.load(Ordering::Relaxed) {
        return;
    }
    let Some(craft) = string_at(plane + OFF_CRAFT) else { return };
    let arm = &arms()[arm_i];
    if !arm.offered(&craft) {
        return; // this craft has no such loadout: it keeps its own
    }
    let Some(fallback) = crafts().iter().find(|c| c.name == craft).map(|c| c.guns_only.clone()) else { return };
    let wanted = arm.key.clone();
    let now = crate::read::<f64>(crate::CLOCK);
    let Ok(mut charged) = CHARGED.try_lock() else { return };
    let repeat = charged.iter().any(|(p, t)| *p == plane && (now - *t).abs() < SAME_LAUNCH_S);
    if !repeat {
        if !arm.affordable() {
            log::info!("PLANE arm: {wanted} short (needs {}), {craft} keeps guns only", arm.short_text());
            write_loadout(plane, &fallback);
            no_rockets(plane);
            return;
        }
        let before: Vec<i32> = arm.parts.iter().map(|(n, _)| owned(n)).collect();
        for (n, c) in &arm.parts {
            take(n, *c);
        }
        charged.retain(|(_, t)| now - *t < 60.0);
        charged.push((plane, now));
        let total = CHARGES.fetch_add(1, Ordering::Relaxed) + 1;
        let after: Vec<i32> = arm.parts.iter().map(|(n, _)| owned(n)).collect();
        log::info!("PLANE arm: {craft} takes {}: stock {before:?} -> {after:?} (charge #{total})", arm.short_text());
    }
    write_loadout(plane, &wanted);
    // The chosen loadout is the whole armament: stop the vanilla class-3 stock draw from topping a FAB loadout up with
    // NAR-122 from the craft's other loadout (seen in a Shipworks test fight: FAB100 plane also carried 6 rockets).
    no_rockets(plane);
    if arm.kind == Kind::Aa {
        if let Ok(mut a) = AA.try_lock() {
            a.retain(|p| p.0 != plane);
            a.push((plane, owner, now));
        }
    }
    if arm.kind != Kind::Aa {
        if let Ok(mut w) = WATCH.try_lock() {
            w.retain(|e| e.0 != plane);
            // `extra_gunpod` is for the two vanilla carrier planes only. Custom crafts (SB-15 etc.) keep their own
            // loadout: if a custom plane was not given a gun in its loadout, it must not suddenly get one.
            let gunpod = matches!(arm.kind, Kind::Bombs | Kind::Rockets)
                && matches!(craft.as_str(), "CRAFT_T7" | "CRAFT_LA29");
            w.push((plane, arm.kind == Kind::Bombs, gunpod, false, now));
        }
    }
    if let Ok(mut v) = VERIFY.try_lock() {
        v.push((plane, now + 1.0, wanted));
    }
}

/// Entry hook of the aircraft update `0x14000b370` (RCX = logic, RDX = body block): K-13 planes in battle mode 6 fire their
/// air-to-air missile through the launcher the other modes use. The launcher checks target, range, timer and angle itself.
/// Bomb and rocket planes: battle mode 6 never starts an attack by itself (a plane goes to state 1 and flies away, notes
/// section 17), so after `plane_attack_delay` seconds a plane armed with a chosen loadout is put into mode 3 (the bomber
/// variant `0x14000bd50`: attack runs with its own pass loop, drops bombs and fires rockets) once it has a target. The state
/// must be set in the same step. The delay lets the plane take off and leave the carrier first.
unsafe fn attackers_to_mode_3(logic: usize) {
    let Ok(w) = WATCH.try_lock() else { return };
    let Some(t0) = w.iter().find(|(p, _, _, _, _)| crate::plausible(*p) && rd::<usize>(*p + OFF_LOGIC) == logic).map(|e| e.4) else { return };
    drop(w);
    if rd::<i32>(logic + 0x118) != 6 || rd::<usize>(logic + 0x158) == 0 || (rd::<i32>(logic + 0x128) <= 0 && rd::<i32>(logic + 0x138) <= 0) {
        return;
    }
    if crate::read::<f64>(crate::CLOCK) - t0 < f32::from_bits(ATTACK_DELAY_S.load(Ordering::Relaxed)) as f64 {
        return;
    }
    (logic as *mut i32).byte_add(0x118).write_unaligned(3);
    let set: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
    set(logic, 3);
    log::info!(
        "PLANE attack: logic {logic:#x} switched to mode 3 / state 3, bombs {} rockets {} target {:#x}",
        rd::<i32>(logic + 0x128),
        rd::<i32>(logic + 0x138),
        rd::<usize>(logic + 0x158)
    );
}

/// True only in the normal tactical battle (`Combat.seria`): mode selector 4 and not the scripted bomber / tutorial
/// flag. `extra_gunpod` must be on as well. The strategic bomber scene (`Bombers.seria`) uses selector 2/3 and the
/// scenario flag, so it is excluded by both tests.
unsafe fn combat_seria() -> bool {
    EXTRA_GUNPOD.load(Ordering::Relaxed) && rd::<i32>(STATE_SELECTOR_AIRCRAFT) == 4 && rd::<u8>(SCENARIO_AIRCRAFT) == 0
}

/// Gives the plane the vanilla 37 mm gun (`ITEM_GUN37`) if it has none. Same fields `configure_plane_gun` `0x14000ea20`
/// writes; called only for our bomb / rocket planes in a normal tactical battle.
unsafe fn install_gunpod(logic: usize) -> bool {
    if rd::<i32>(logic + 0x1a4) != 0 {
        return true;
    }
    let (begin, end): (usize, usize) = (rd(AMMO_LIST), rd(AMMO_LIST + 8));
    if begin < 0x10000 || end <= begin || (end - begin) / AMMO_SIZE > 64 {
        return false;
    }
    // Prefer the vanilla 37 mm. A mod may replace/rename it, so fall back to the first class-4 (aircraft gun) ammo.
    let mut pick: Option<(usize, i32, String)> = None;
    for rec in (begin..end).step_by(AMMO_SIZE) {
        let name = string_at(rec + 8).unwrap_or_default();
        if name == "ITEM_GUN37" {
            pick = Some((rec, 4, name));
            break;
        }
        if pick.is_none() && rd::<i32>(rec) == 4 {
            pick = Some((rec, 4, name));
        }
    }
    let Some((rec, class, name)) = pick else {
        log::warn!("PLANE gunpod: ITEM_GUN37 and no class-4 gun in the ammo list; logic {logic:#x} stays without a gun");
        return false;
    };
    let index = ((rec - begin) / AMMO_SIZE) as i32;
    let delay = rd::<f32>(rec + 0x17c);
    let magazine = rd::<i32>(rec + 0x180).clamp(1, 999);
    let reserve = (50 - magazine).max(0);
    (logic as *mut i32).byte_add(0x1a4).write_unaligned(index);
    (logic as *mut f32).byte_add(0xa0).write_unaligned(2.0);
    (logic as *mut f32).byte_add(0xa8).write_unaligned(delay);
    (logic as *mut i32).byte_add(0xb0).write_unaligned(magazine);
    (logic as *mut i32).byte_add(0xac).write_unaligned(magazine);
    (logic as *mut i32).byte_add(0xb4).write_unaligned(reserve);
    log::info!(
        "PLANE gunpod: logic {logic:#x} armed with {name} (class {class}, index {index}, magazine {magazine}, reserve {reserve}) in Combat.seria"
    );
    true
}

/// Adds the gunpod to a watched bomb / rocket plane, if this is the tactical battle and it still has no gun.
unsafe fn ensure_gunpod(logic: usize) {
    if !combat_seria() || rd::<i32>(logic + 0x1a4) != 0 {
        return;
    }
    let Ok(w) = WATCH.try_lock() else { return };
    let applies = w.iter().any(|(p, _, gunpod, _, _)| *gunpod && crate::plausible(*p) && rd::<usize>(*p + OFF_LOGIC) == logic);
    drop(w);
    if applies {
        install_gunpod(logic);
    }
}

pub unsafe extern "C" fn on_update(logic: usize, body: usize) {
    if crate::plausible(logic) {
        ensure_gunpod(logic);
        attackers_to_mode_3(logic);
    }
    census(crate::read::<f64>(crate::CLOCK)); // the body array is only stable inside the aircraft update
    let Ok(aa) = AA.try_lock() else { return };
    if aa.is_empty() || !crate::plausible(logic) || !crate::plausible(body) {
        return;
    }
    let Some((plane, owner, armed_at)) = aa.iter().find(|p| crate::plausible(p.0) && rd::<usize>(p.0 + OFF_LOGIC) == logic).copied() else { return };
    // like the bombers (`plane_attack_delay`): a fresh interceptor flies the game's own script until the delay has passed,
    // else it dives off the deck into the carrier (reported 2026-10-25); the same delay hides it from the hunt below
    let armed_for = crate::read::<f64>(crate::CLOCK) - armed_at;
    let too_early = armed_for < f32::from_bits(AA_DELAY_S.load(Ordering::Relaxed)) as f64;
    drop(aa);
    if rd::<i32>(logic + 0x148) <= 0 {
        // the K-13 are gone: leave the fight (mode 2 never leaves by itself, notes section 18e)
        let state = rd::<i32>(logic + 0x190);
        if !matches!(state, 5..=7) {
            (logic as *mut i32).byte_add(0x118).write_unaligned(6);
            let set_state: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
            set_state(logic, 5);
            if AA_LOGS.fetch_add(1, Ordering::Relaxed) < 200 {
                log::info!("PLANE AA: logic {logic:#x} used up its K-13, leaving (state {state} -> 5)");
            }
        }
        return;
    }
    let n = AA_TICKS.fetch_add(1, Ordering::Relaxed);
    if n % 150 == 0 && n < 20000 {
        log::info!(
            "PLANE AA watch: logic {logic:#x} mode {} state {} AA list {} target {:#x} distance to carrier {:.0}",
            rd::<i32>(logic + 0x118),
            rd::<i32>(logic + 0x190),
            rd::<i32>(logic + 0x148),
            rd::<usize>(logic + 0x158),
            ((rd::<f32>(plane + 0x128) - rd::<f32>(owner + 0x1840)).powi(2) + (rd::<f32>(plane + 0x12c) - rd::<f32>(owner + 0x1844)).powi(2)).sqrt()
        );
    }
    let (mode, state, target) = (rd::<i32>(logic + 0x118), rd::<i32>(logic + 0x190), rd::<usize>(logic + 0x158));
    // diagnostics for the engagement: a launch shows as the K-13 list shrinking; while engaged, the numbers the launcher checks
    let list = rd::<i32>(logic + 0x148);
    if let Ok(mut last) = AA_LAST.try_lock() {
        match last.iter_mut().find(|e| e.0 == logic) {
            Some(e) => {
                if list < e.1 && AA_LOGS.fetch_add(1, Ordering::Relaxed) < 200 {
                    log::info!("PLANE AA missile: logic {logic:#x} K-13 list {} -> {list}, target {target:#x}", e.1);
                }
                e.1 = list;
            }
            None => {
                last.push((logic, list));
                let factor = f32::from_bits(AA_SPEED.load(Ordering::Relaxed));
                if factor != 1.0 && (0.1..=10.0).contains(&factor) {
                    let before = (rd::<f32>(logic + 0x64), rd::<f32>(logic + 0x68), rd::<f32>(logic + 0x6c));
                    for off in [0x64usize, 0x68, 0x6c] {
                        (logic as *mut f32).byte_add(off).write_unaligned(rd::<f32>(logic + off) * factor);
                    }
                    log::info!("PLANE AA: logic {logic:#x} nominal speed fields {before:?} x {factor}");
                }
            }
        }
    }
    if mode == 0 && target != 0 && n % 15 == 0 && AA_LOGS.load(Ordering::Relaxed) < 200 {
        log::info!(
            "PLANE AA engaged: logic {logic:#x} state {state} target {target:#x} id {} dist(body+0x2c) {:.0} timer(+0xb8) {:.2} K-13 list {list} plane speed {:.0}",
            rd::<i32>(target + 0x3c4),
            rd::<f32>(body + 0x2c),
            rd::<f32>(logic + 0xb8),
            rd::<f32>(plane + 0x130).hypot(rd::<f32>(plane + 0x134))
        );
    }
    let set_state: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
    let hunt = true; // every frame: a Zenith is only in the air for a few seconds
    // Interceptors (notes sections 14, 17, 18): idle in mode 6 state 0 (a standing turn near the carrier, no target needed);
    // with a target (the carrier's, or a foe Zenith found here) mode 0 state 3 flies at it and fires the K-13 and the gun.
    match mode {
        6 if !too_early && target != 0 && matches!(rd::<i32>(target + 0x3c4), 1 | 500) && rd::<f32>(target + 0x248) * rd::<f32>(owner + 0xad4) <= 0.0 => {
            // a target came with the launch and the K-13 can take it (a plane or a missile): engage it. A SHIP target (the
            // carrier's locked enemy) is not one: the mode-0 code drops it at once (rel.30 flapped between the modes).
            (logic as *mut i32).byte_add(0x118).write_unaligned(2);
            set_state(logic, 3);
            if AA_LOGS.fetch_add(1, Ordering::Relaxed) < 40 {
                log::info!("PLANE AA: logic {logic:#x} engages its launch target {target:#x} (mode 2, state {state} -> 3)");
            }
        }
        6 => {
            if hunt && !too_early && engage_zenith(logic, owner) {
                return;
            }
            // the game sends a plane that cleared the carrier to retire (1); a plane that reaches 2..4 would fire the stock-
            // supplied rockets (a K-13 plane gets 12 NAR from the player's stock in this mode, rel.32 log): back to the circle
            if matches!(state, 1..=4) {
                set_state(logic, 0);
            }
        }
        // mode 2 picks the FIRST missile it finds when it has no target, friendly ones too: never let it run without one
        2 if target == 0 => {
            if engage_zenith(logic, owner) {
                return;
            }
            (logic as *mut i32).byte_add(0x118).write_unaligned(6);
            set_state(logic, 0);
            if AA_LOGS.fetch_add(1, Ordering::Relaxed) < 40 {
                log::info!("PLANE AA: logic {logic:#x} lost its target, back to the circle (mode 6, state 0)");
            }
        }
        _ => {}
    }
}

const SET_LINK: usize = 0x1_4019_4de0;
/// The flat array of all battle bodies (planes, missiles, shells), rebuilt every frame: `array[0..count]` of Body*
/// (notes section 18c, the array Weapon_FlakInterceptScan walks), count a u32 at `BODY_COUNT` (cap 15000).
const BODY_ARRAY: usize = 0x1_43a3_3c90;
const BODY_COUNT: usize = 0x1_43a5_1150;

/// All active bodies: (body, logic id `+0x3c4`, side float `+0x248`, x, y).
unsafe fn bodies() -> Vec<(usize, i32, f32, f32, f32)> {
    let count = rd::<u32>(BODY_COUNT).min(15000) as usize;
    let mut out = Vec::new();
    for i in 0..count {
        let body: usize = rd(BODY_ARRAY + i * 8);
        if !crate::plausible(body) || rd::<usize>(body + 0x1f0) == 0 {
            continue;
        }
        let vt: usize = rd(body);
        if !crate::plausible(vt) {
            continue;
        }
        let active: unsafe extern "win64" fn(usize) -> i32 = std::mem::transmute(rd::<usize>(vt + 0x78));
        if active(body) & 0xff == 0 {
            continue;
        }
        out.push((body, rd(body + 0x3c4), rd(body + 0x248), rd(body + 0x128), rd(body + 0x12c)));
    }
    out
}

/// The side float of the ship that launched `body` (0 when unknown). rel.32 log: a Zenith's own `+0x248` is 0, so the owner
/// has to be asked. The owner node pointer is at `+0x4b0` (section 18d; the older +0xc0 guesses stay as fallbacks), and a candidate only counts when it looks like a creature (class code bits `0x2f`) with a non-zero side.
unsafe fn owner_side(body: usize) -> f32 {
    // notes section 18d: the launch writes SetLink(body+0x4a8, launching ship): the owner node pointer is at +0x4b0
    let owner: usize = rd(body + 0x4b0);
    let direct: usize = rd(body + 0xc0 + 8);
    let link: usize = rd(body + 0xc0);
    let indirect: usize = if crate::plausible(link) { rd(link + 8) } else { 0 };
    for cand in [owner, direct, indirect] {
        if crate::plausible(cand) && rd::<u8>(cand + 0x28) & 0x2f == 0x2f {
            let v = rd::<f32>(cand + 0xad4);
            if v.is_finite() && v != 0.0 && v.abs() < 1.0e6 {
                return v;
            }
        }
    }
    0.0
}

static OWNER_DUMPS: AtomicU32 = AtomicU32::new(0);

/// Diagnostic: for a Zenith body whose owner could not be read, lists every qword in the body (`+0..+0x600`) that is, or
/// points (at +8) to, something that looks like a creature, with its side float: shows where the owner is stored.
unsafe fn dump_owner_candidates(body: usize) {
    let mut found = Vec::new();
    for off in (0..0x600).step_by(8) {
        let q: usize = rd(body + off);
        if !crate::plausible(q) {
            continue;
        }
        for (kind, cand) in [("direct", q), ("link+8", rd::<usize>(q + 8))] {
            if crate::plausible(cand) && rd::<u8>(cand + 0x28) & 0x2f == 0x2f {
                found.push(format!("+{off:#x} {kind} -> {cand:#x} side {}", rd::<f32>(cand + 0xad4)));
            }
        }
    }
    log::info!("PLANE zenith {body:#x} owner candidates: {found:?}");
}

/// The foe the interceptors go for: the nearest living enemy Zenith-type missile (logic id 1), else the nearest enemy plane
/// (500), measured from the carrier. A foe has the opposite side sign of the owner. 0 when there is none.
unsafe fn find_target(owner: usize, own_side: f32) -> usize {
    let (ox, oy) = (rd::<f32>(owner + 0x1840), rd::<f32>(owner + 0x1844));
    let mut best: Option<(i32, f32, usize)> = None; // (rank 0 = Zenith, 1 = plane; distance^2; body)
    for (body, id, side, x, y) in bodies() {
        let rank = match id {
            1 => 0,
            500 => 1,
            _ => continue,
        };
        let side = if side != 0.0 { side } else { owner_side(body) };
        if side * own_side >= 0.0 {
            continue;
        }
        let d = (x - ox).powi(2) + (y - oy).powi(2);
        if best.map_or(true, |b| (rank, d) < (b.0, b.1)) {
            best = Some((rank, d, body));
        }
    }
    best.map_or(0, |b| b.2)
}

/// Gives the plane a foe Zenith as its target and sends it into mode 0 state 3. True when one was found.
unsafe fn engage_zenith(logic: usize, owner: usize) -> bool {
    let own_side = rd::<f32>(owner + 0xad4);
    if own_side == 0.0 {
        return false;
    }
    let zenith = find_target(owner, own_side);
    if zenith == 0 {
        return false;
    }
    let set_link: unsafe extern "win64" fn(usize, usize) = std::mem::transmute(SET_LINK);
    set_link(logic + 0x150, zenith);
    (logic as *mut i32).byte_add(0x118).write_unaligned(2); // the vanilla interceptor mode: keeps its target, fires K-13 and gun
    let set_state: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
    set_state(logic, 3);
    log::info!("PLANE AA: logic {logic:#x} targets foe body {zenith:#x} id {} (mode 2, state 3), target now {:#x}", rd::<i32>(zenith + 0x3c4), rd::<usize>(logic + 0x158));
    true
}

/// Planes (plane parts) whose logic must not draw rockets from the player's stock: every plane whose armament the
/// pre-combat button chose (guns-only, bombs, rockets, K-13). The chosen loadout is the whole loadout; the vanilla
/// class-3 auto-draw would otherwise add NAR-122 from another loadout of the same craft (e.g. a FAB100 plane with six
/// stock rockets).
static NO_DRAW: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn no_rockets(plane: usize) {
    if let Ok(mut set) = NO_DRAW.lock() {
        if !set.contains(&plane) {
            set.push(plane);
        }
        if set.len() > 64 {
            set.remove(0);
        }
    }
}

/// Called by the stub below with the aircraft logic: non-zero = skip the vanilla class-3 stock draw for it.
unsafe extern "C" fn no_draw(logic: usize) -> u32 {
    let Ok(set) = NO_DRAW.try_lock() else { return 0 };
    set.iter().any(|&p| crate::diag::rd::<usize>(p + OFF_LOGIC) == Some(logic)) as u32
}

/// The gate of the stock draw in `FUN_14000ceb0` (`0x14000d13c`: `movss xmm0,[rdi+0x98]`, RDI = logic; notes
/// `carrier-plane-loadouts.md` section 13). For a plane in `NO_DRAW` the stub zeroes xmm9 (used as zero later) and jumps
/// past the draw block (`0x14000d335`), otherwise it runs the original instruction.
const DRAW_GATE: usize = 0x1_4000_d13c;
const DRAW_GATE_BYTES: [u8; 12] = [0xf3, 0x0f, 0x10, 0x87, 0x98, 0x00, 0x00, 0x00, 0x45, 0x0f, 0x57, 0xc9];
const DRAW_SKIP: usize = 0x1_4000_d335;

pub unsafe fn install_no_rocket_draw(session: &mut PatchSession) {
    let found = std::slice::from_raw_parts(DRAW_GATE as *const u8, DRAW_GATE_BYTES.len());
    if found != DRAW_GATE_BYTES {
        log::warn!("hf_ally: rocket draw gate {DRAW_GATE:#x} has unexpected bytes {found:02x?}: guns-only planes may still draw rockets");
        return;
    }
    let mut t = Trampoline::new();
    t.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xe4, 0xf0]); // push rbp; mov rbp,rsp; and rsp,-16
    t.preserved_call(no_draw as *const (), &[0x48, 0x89, 0xf9], &[], ReturnType::Rax); // mov rcx,rdi
    t.bytes(&[0x48, 0x89, 0xec, 0x5d]); // mov rsp,rbp; pop rbp
    t.bytes(&[0x85, 0xc0]); // test eax,eax
    let normal = t.new_label();
    t.jump_if(patchy::Condition::Equal, normal);
    t.bytes(&[0x45, 0x0f, 0x57, 0xc9]); // xorps xmm9,xmm9
    t.absolute_jump(DRAW_SKIP);
    let _ = t.bind(normal);
    t.bytes(&DRAW_GATE_BYTES[..8]); // movss xmm0,[rdi+0x98]
    t.relative_jump(DRAW_GATE + 8);
    match session.detour_trampoline(DRAW_GATE, 8, t) {
        Ok(_) => log::info!("hf_ally: rocket stock draw gate hooked at {DRAW_GATE:#x}"),
        Err(e) => log::warn!("hf_ally: rocket stock draw gate failed: {e}"),
    }
}

pub unsafe fn install_aa(session: &mut PatchSession) {
    let found = std::slice::from_raw_parts(UPDATE_FN as *const u8, UPDATE_BYTES.len());
    if found != UPDATE_BYTES {
        log::warn!("hf_ally: aircraft update {UPDATE_FN:#x} has unexpected bytes {found:02x?}: K-13 not fired");
        return;
    }
    let mut t = Trampoline::new();
    t.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp,8 (entry RSP is 8 mod 16)
    t.preserved_call(on_update as *const (), &[], &[], ReturnType::None);
    t.bytes(&[0x48, 0x83, 0xc4, 0x08]); // add rsp,8
    t.bytes(&UPDATE_BYTES);
    t.relative_jump(UPDATE_FN + UPDATE_BYTES.len());
    match session.detour_trampoline(UPDATE_FN, UPDATE_BYTES.len(), t) {
        Ok(_) => log::info!("hf_ally: K-13 hook on at {UPDATE_FN:#x}"),
        Err(e) => log::warn!("hf_ally: K-13 hook failed: {e}"),
    }
}

/// Called every battle tick: a plane that has used up the bombs / rockets chosen for it leaves the fight.
/// The global creature list `[begin, end)`, stride 0x18, creature pointer at entry+8 (notes section 18).
const CREATURES: (usize, usize) = (0x1_43fa_a5d8, 0x1_43fa_a5e0);
static CENSUS_T: Mutex<f64> = Mutex::new(-100.0);

/// Diagnostic for the interceptor work: every 5 s while a K-13 plane is up, logs how many living creatures there are per
/// logic id (`+0x3c4`) and side sign, so the id of an enemy plane (500) and of a Zenith missile can be read off the log.
unsafe fn census(now: f64) {
    if AA.try_lock().map_or(true, |a| a.is_empty()) {
        return;
    }
    let Ok(mut last) = CENSUS_T.try_lock() else { return };
    if now - *last < 5.0 {
        return;
    }
    *last = now;
    let (b, e): (usize, usize) = (rd(CREATURES.0), rd(CREATURES.1));
    if b < 0x10000 || e < b || (e - b) / 0x18 > 5000 {
        return;
    }
    let mut seen: Vec<(i32, i32, u32)> = Vec::new(); // (logic id, side sign, count)
    for a in (b..e).step_by(0x18) {
        let c: usize = rd(a + 8);
        if !crate::plausible(c) || rd::<i32>(c + 0x48) == 0 {
            continue;
        }
        let (id, side) = (rd::<i32>(c + 0x3c4), rd::<f32>(c + 0xad4));
        let sign = if side > 0.0 { 1 } else if side < 0.0 { -1 } else { 0 };
        match seen.iter_mut().find(|e| e.0 == id && e.1 == sign) {
            Some(e) => e.2 += 1,
            None => seen.push((id, sign, 1)),
        }
    }
    seen.sort();
    log::info!("PLANE census (logic id, side sign, count): {seen:?}");
    // the same for the flat body array the interceptors hunt in (side = the body's +0x248 sign)
    let mut bodies_seen: Vec<(i32, i32, u32)> = Vec::new();
    for (body, id, side, _, _) in bodies() {
        // a Zenith-type body (id 1) whose own side is 0 is listed as 1000 (owner unknown), 1001 (owner on the player's side)
        // or 1002 (owner on the AI side)
        let id = if id == 1 && side == 0.0 {
            let o = owner_side(body);
            if o == 0.0 && OWNER_DUMPS.fetch_add(1, Ordering::Relaxed) < 3 {
                dump_owner_candidates(body);
            }
            if o > 0.0 { 1001 } else if o < 0.0 { 1002 } else { 1000 }
        } else {
            id
        };
        let sign = if side > 0.0 { 1 } else if side < 0.0 { -1 } else { 0 };
        match bodies_seen.iter_mut().find(|e| e.0 == id && e.1 == sign) {
            Some(e) => e.2 += 1,
            None => bodies_seen.push((id, sign, 1)),
        }
    }
    bodies_seen.sort();
    log::info!("PLANE body census (logic id, +0x248 sign, count): {bodies_seen:?}");
}

pub unsafe fn tick() {
    let Ok(mut w) = WATCH.try_lock() else { return };
    if w.is_empty() {
        return;
    }
    w.retain_mut(|(plane, bombs, gunpod, seen, _)| {
        if !crate::plausible(*plane) || rd::<i32>(*plane + 0x48) == 0 {
            return false; // destroyed or gone
        }
        let logic: usize = rd(*plane + OFF_LOGIC);
        if !crate::plausible(logic) {
            return true;
        }
        let bombs_now = rd::<i32>(logic + 0x128);
        let rockets_now = rd::<i32>(logic + 0x138);
        let state = rd::<i32>(logic + 0x190);
        if bombs_now > 0 || rockets_now > 0 {
            *seen = true;
            return true;
        }
        if !*seen || !(1..=4).contains(&state) {
            return !matches!(state, 5..=7);
        }
        // `extra_gunpod`: stay in the fight and strafe with the 37 mm instead of leaving, but only in the normal
        // tactical battle. The strategic bomber scene (`Bombers.seria`) is mode 3/selector 2-3 and never reaches this.
        if *gunpod && combat_seria() && !matches!(state, 5..=7) {
            if rd::<i32>(logic + 0x1a4) == 0 {
                install_gunpod(logic);
            }
            let rounds = rd::<i32>(logic + 0xac) + rd::<i32>(logic + 0xb4);
            let target = rd::<usize>(logic + 0x158);
            if rd::<i32>(logic + 0x1a4) != 0 && rounds > 0 && target != 0 {
                let mode = rd::<i32>(logic + 0x118);
                let set: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
                if mode != 2 {
                    (logic as *mut i32).byte_add(0x118).write_unaligned(2);
                    set(logic, 3);
                    log::info!(
                        "PLANE gunpod: plane {plane:#x} payload gone (bombs {bombs_now}, rockets {rockets_now}), mode {mode} -> 2 / state 3, 37 mm {rounds} rounds, target {target:#x}"
                    );
                } else if state != 3 {
                    set(logic, 3);
                }
                return true;
            }
        }
        let set: unsafe extern "win64" fn(usize, i32) = std::mem::transmute(SET_STATE);
        set(logic, 5);
        log::info!(
            "PLANE leaves: plane {plane:#x} used up its payload (bombs {bombs_now}, rockets {rockets_now}; watched {}) (state {state} -> 5)",
            if *bombs { "bombs" } else { "rockets" }
        );
        false
    });
}

/// Called every battle tick: one second after an arming, logs what the plane's loadout string says now.
pub unsafe fn verify(now: f64) {
    let Ok(mut v) = VERIFY.try_lock() else { return };
    if v.is_empty() {
        return;
    }
    let due: Vec<_> = v.iter().filter(|e| now >= e.1).cloned().collect();
    v.retain(|e| now < e.1);
    for (plane, _, wanted) in due {
        if crate::plausible(plane) {
            let logic: usize = rd(plane + OFF_LOGIC);
            let counts = if crate::plausible(logic) {
                format!(
                    "logic {logic:#x} bombs(+0x128)={} rockets(+0x138)={} AA(+0x148)={} mode(+0x118)={} state(+0x190)={} target(+0x158)={:#x} gun(+0x1a4)={} mag(+0xac)={} reserve(+0xb4)={}",
                    rd::<i32>(logic + 0x128),
                    rd::<i32>(logic + 0x138),
                    rd::<i32>(logic + 0x148),
                    rd::<i32>(logic + 0x118),
                    rd::<i32>(logic + 0x190),
                    rd::<usize>(logic + 0x158),
                    rd::<i32>(logic + 0x1a4),
                    rd::<i32>(logic + 0xac),
                    rd::<i32>(logic + 0xb4)
                )
            } else {
                format!("logic {logic:#x} (none)")
            };
            log::info!(
                "PLANE check: plane={plane:#x} loadout now={:?} (we wrote {wanted:?}) {counts} campaign bytes {}/{}",
                string_at(plane + OFF_LOADOUT),
                rd::<u8>(CAMPAIGN_BYTES.0),
                rd::<u8>(CAMPAIGN_BYTES.1)
            );
        }
    }
}

/// Installs the entry hook. The stolen prologue is replayed after the call with RSP exactly as at entry (`mov rax,rsp`
/// is the first instruction and the frame is based on it).
pub unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    let found = std::slice::from_raw_parts(ARM_FN as *const u8, ARM_BYTES.len());
    if found != ARM_BYTES {
        return Err(format!("plane arming function {ARM_FN:#x} has unexpected bytes {found:02x?}: plane loadouts not applied (another mod owns it?)"));
    }
    let mut t = Trampoline::new();
    t.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp,8: entry RSP is 8 mod 16, the preserved call wants 0
    t.preserved_call(on_arm as *const (), &[], &[], ReturnType::None);
    t.bytes(&[0x48, 0x83, 0xc4, 0x08]); // add rsp,8
    t.bytes(&ARM_BYTES);
    t.relative_jump(ARM_FN + ARM_BYTES.len());
    session.detour_trampoline(ARM_FN, ARM_BYTES.len(), t).map_err(|e| e.to_string())?;
    log::info!("hf_ally: plane loadout hook on at {ARM_FN:#x}");
    Ok(())
}

/// The gate in the plane state function `0x14000ceb0`: `cmp byte [0x147fc6f80],0 / jnz` (the bytes are the hot-seat
/// flags, not "campaign"), so in a normal battle a plane with bombs never enters the bomber variant `0x14000bd50`.
/// Turning the `jnz` (0x75) into `jmp` (0xeb) sends every plane with a bomb list and a state other than 0 there.
const BOMBER_GATE: usize = 0x1_4000_cf16;
const BOMBER_GATE_CONTEXT: [u8; 11] = [0x75, 0x09, 0x80, 0x3d, 0x62, 0xa0, 0xfb, 0x07, 0x00, 0x74, 0x1d];

pub unsafe fn install_bomber_gate(session: &mut PatchSession) {
    let found = std::slice::from_raw_parts(BOMBER_GATE as *const u8, BOMBER_GATE_CONTEXT.len());
    if found != BOMBER_GATE_CONTEXT {
        log::warn!("hf_ally: bomber gate not applied, bytes at {BOMBER_GATE:#x} are {found:02x?}");
        return;
    }
    match session.overwrite(BOMBER_GATE, &[0xeb]) {
        Ok(_) => log::info!("hf_ally: planes with bombs use the bomber routine ({BOMBER_GATE:#x})"),
        Err(e) => log::warn!("hf_ally: bomber gate failed: {e}"),
    }
}

/// Editor / Shipworks test-fight flag (`DAT_147fc6fb6`, set by `EDC_TEST_FIGHT`; notes section 16).
const TEST_FLAG: u64 = 0x1_47fc_6fb6;
const OWNED_BYTES: [u8; 12] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0xca];
const REMOVE_BYTES: [u8; 16] = [0x48, 0x89, 0x6c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xe9, 0x41, 0x8b, 0xf8];

/// In a Shipworks test fight the profile's stock is unlimited: "owned" answers 1000 and "remove" does nothing, so every
/// plane armament can be picked and nothing is taken from the real campaign stock (the vanilla rocket auto-supply too).
pub unsafe fn install_test_stock(session: &mut PatchSession) {
    for (name, addr, stolen, is_owned) in [("owned", PROFILE_OWNED, &OWNED_BYTES[..], true), ("remove", PROFILE_REMOVE, &REMOVE_BYTES[..], false)] {
        let found = std::slice::from_raw_parts(addr as *const u8, stolen.len());
        if found != stolen {
            log::warn!("hf_ally: test-fight stock ({name}) not hooked, bytes at {addr:#x} are {found:02x?}");
            return;
        }
        let mut t = Trampoline::new();
        t.bytes(&[0x48, 0xb8]); // mov rax, imm64 (volatile, not an argument)
        t.bytes(&TEST_FLAG.to_le_bytes());
        t.bytes(&[0x80, 0x38, 0x00]); // cmp byte [rax], 0
        if is_owned {
            t.bytes(&[0x74, 0x06, 0xb8, 0xe8, 0x03, 0x00, 0x00, 0xc3]); // je orig; mov eax,1000; ret
        } else {
            // je orig; mov eax, r8d; ret: the game uses the return value as "amount removed" (rocket auto-supply pushes that many
            // entries), so the answer must be the requested count (rel.21 left a pointer there: billions of entries, out of memory)
            t.bytes(&[0x74, 0x04, 0x44, 0x89, 0xc0, 0xc3]);
        }
        t.bytes(stolen);
        t.relative_jump(addr + stolen.len());
        if let Err(e) = session.detour_trampoline(addr, stolen.len(), t) {
            log::warn!("hf_ally: test-fight stock ({name}) failed: {e}");
            return;
        }
    }
    log::info!("hf_ally: plane stock is unlimited in Shipworks test fights");
}

/// K-13 launcher limits (notes section 18f), all RIP-relative loads in `FUN_14000f5b0`:
/// * max range `3750.0` at `0x140352848`: only that instruction reads it, so the value is rewritten;
/// * alignment `0.1745` at `0x140352794`: shared by six other functions, so the load at `0x14000f686` is replaced by a load
///   from our own float (`push rax; mov rax, &float; movss xmm2,[rax]; pop rax`).
const MAX_RANGE_ADDR: usize = 0x1_4035_2848;
const MAX_RANGE_BYTES: [u8; 4] = 3750.0f32.to_le_bytes();
const ANGLE_SITE: usize = 0x1_4000_f686;
const ANGLE_BYTES: [u8; 8] = [0xf3, 0x0f, 0x10, 0x15, 0x06, 0x31, 0x34, 0x00];
/// Seconds after the arming before a K-13 plane may engage (`plane_aa_delay`).
pub static AA_DELAY_S: AtomicU32 = AtomicU32::new(3.0f32.to_bits());
pub static AA_ANGLE: AtomicU32 = AtomicU32::new(0.1745f32.to_bits());
/// Speed factor written once into the plane logic's three nominal-speed fields (`+0x64/+0x68/+0x6c`), 1.0 = vanilla.
pub static AA_SPEED: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

pub unsafe fn install_aa_limits(session: &mut PatchSession, range: f32, angle: f32) {
    if range > 3750.0 && range <= 20000.0 {
        let found = std::slice::from_raw_parts(MAX_RANGE_ADDR as *const u8, 4);
        if found == MAX_RANGE_BYTES {
            match session.overwrite(MAX_RANGE_ADDR, &range.to_le_bytes()) {
                Ok(_) => log::info!("hf_ally: K-13 launch range {range:.0} (was 3750)"),
                Err(e) => log::warn!("hf_ally: K-13 range patch failed: {e}"),
            }
        } else {
            log::warn!("hf_ally: K-13 range constant at {MAX_RANGE_ADDR:#x} is {found:02x?}, not patched");
        }
    }
    if angle > 0.1745 && angle <= 1.2 {
        AA_ANGLE.store(angle.to_bits(), Ordering::Relaxed);
        let found = std::slice::from_raw_parts(ANGLE_SITE as *const u8, ANGLE_BYTES.len());
        if found != ANGLE_BYTES {
            log::warn!("hf_ally: K-13 alignment load at {ANGLE_SITE:#x} is {found:02x?}, not patched");
            return;
        }
        let mut t = Trampoline::new();
        t.bytes(&[0x50, 0x48, 0xb8]); // push rax; mov rax, imm64
        t.bytes(&(AA_ANGLE.as_ptr() as u64).to_le_bytes());
        t.bytes(&[0xf3, 0x0f, 0x10, 0x10, 0x58]); // movss xmm2,[rax]; pop rax
        t.relative_jump(ANGLE_SITE + ANGLE_BYTES.len());
        match session.detour_trampoline(ANGLE_SITE, ANGLE_BYTES.len(), t) {
            Ok(_) => log::info!("hf_ally: K-13 alignment tolerance {angle:.3} rad (was 0.1745)"),
            Err(e) => log::warn!("hf_ally: K-13 alignment patch failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_fallback_matches_the_game_table() {
        let t = vanilla_table();
        assert_eq!(t.crafts.iter().map(|c| (c.name.as_str(), c.label.as_str(), c.guns_only.as_str())).collect::<Vec<_>>(), [("CRAFT_T7", "T-7", "LOADOUT_T7_GUN37"), ("CRAFT_LA29", "LA-29", "LOADOUT_LA29_GUN37")]);
        let of = |craft: &str| t.arms.iter().filter(|a| a.craft == craft).map(|a| (a.key.as_str(), a.parts[0].1)).collect::<Vec<_>>();
        assert_eq!(of("CRAFT_T7"), [("LOADOUT_T7_FAB100", 4), ("LOADOUT_T7_FAB250", 2), ("LOADOUT_T7_NURS122", 12), ("LOADOUT_T7_NURS340", 2), ("LOADOUT_T7_K13", 2)]);
        assert_eq!(of("CRAFT_LA29"), [("LOADOUT_LA29_FAB100", 2), ("LOADOUT_LA29_FAB250", 1), ("LOADOUT_LA29_NURS122", 6)]);
    }

    #[test]
    fn labels_and_kinds() {
        let none = HashMap::new();
        let a = make_arm("CRAFT_T7", "LOADOUT_T7_X".into(), vec![("ITEM_FAB250".into(), 1), ("ITEM_NAR122".into(), 6)], &none);
        assert_eq!(a.label, "BOMBS 250 + ROCKETS 122");
        assert_eq!(a.kind, Kind::Bombs);
        assert_eq!(make_arm("CRAFT_T7", "k".into(), vec![("ITEM_K13".into(), 2)], &none).kind, Kind::Aa);
        assert_eq!(make_arm("CRAFT_T7", "k".into(), vec![("ITEM_NEW".into(), 1)], &none).kind, Kind::Other);
        assert_eq!(item_label("ITEM_NEW"), "NEW");
    }

    /// The Hara Vera bomber: its ROCKET loadout is `ITEM_AMMO_300MM_INC`, a rocket (ammo class 3) with no FAB / NAR in the name.
    #[test]
    fn class_comes_from_the_ammo_list_not_the_name() {
        let mut classes = HashMap::new();
        classes.insert("ITEM_AMMO_300MM_INC".to_string(), 3);
        classes.insert("ITEM_SPECIAL_AA".to_string(), 5);
        let rocket = make_arm("CRAFT_SB15", "LOADOUT_SB15_ROCKET".into(), vec![("ITEM_AMMO_300MM_INC".into(), 1)], &classes);
        assert_eq!(rocket.kind, Kind::Rockets);
        assert_eq!(rocket.label, "300MM_INC");
        // the list wins over a misleading name, and an unknown item without a list is Other
        assert_eq!(make_arm("CRAFT_X", "k".into(), vec![("ITEM_SPECIAL_AA".into(), 1)], &classes).kind, Kind::Aa);
        assert_eq!(make_arm("CRAFT_SB15", "k".into(), vec![("ITEM_AMMO_300MM_INC".into(), 1)], &HashMap::new()).kind, Kind::Other);
    }

    #[test]
    fn craft_labels_and_the_guns_only_fallback() {
        assert_eq!(craft_label("CRAFT_T7"), "T-7");
        assert_eq!(craft_label("CRAFT_LA29"), "LA-29");
        assert_eq!(craft_label("CRAFT_SB15"), "SB-15");
        assert_eq!(craft_label("CRAFT_MIG"), "MIG");
        // a craft without a parts-free loadout (the SB-15) gets a name the game does not know: it arms the plain gun
        assert_eq!(make_craft("CRAFT_SB15", None).guns_only, "LOADOUT_SB15_GUNS");
        assert_eq!(make_craft("CRAFT_SB15", Some("LOADOUT_SB15_GUN".into())).guns_only, "LOADOUT_SB15_GUN");
    }
}
