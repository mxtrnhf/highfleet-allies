//! Read owned aircraft and loadout descriptions from the live native tables.

use crate::{
    memory,
    native::read as rd,
    native_layout::{Ammo, Loadout, Munition, Plane, PlaneNode},
};
use std::{
    collections::HashMap,
    mem::{offset_of, size_of},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Bombs,
    Rockets,
    /// carries an air-to-air missile (ammo class 5)
    Aa,
    /// a gun-only loadout (no parts, the game's `has_gun37mm` flag, a qol `gun_ammo` entry)
    Gun,
    /// parts this module knows nothing about: armed and charged, the game's own flight script
    Other,
}

/// One armament choice: a loadout of the game's table. Gun-only loadouts carry no parts and cost
/// nothing; the rest are (item name, count of one plane) pairs charged from the profile.
pub struct Arm {
    /// the loadout name (`LOADOUT_T7_FAB100`)
    pub key: String,
    /// the craft whose vector lists it (`CRAFT_T7`)
    pub craft: String,
    pub label: String,
    /// (item name, count) of one plane
    pub parts: Vec<(String, i32)>,
    pub kind: Kind,
}

impl Arm {
    pub fn short_text(&self) -> String {
        self.parts
            .iter()
            .map(|(name, count)| format!("{name} x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Shipworks has no inventory display. Show the actual per-plane payload
    /// quantities so variants with the same ordnance type remain distinguishable.
    pub fn airstrike_label(&self) -> String {
        if self.parts.is_empty() {
            return self.label.clone();
        }
        self.parts
            .iter()
            .map(|(name, count)| format!("{count}x {}", item_label(name)))
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// A plane type of the game's craft table (`PLANE_TLL`): vanilla T-7 and LA-29, and whatever a mod registered (qol `planes`).
pub struct Craft {
    /// the game name (`CRAFT_SB15`)
    pub name: String,
    /// the name on the button label (`SB-15`)
    pub label: String,
    /// the loadout that arms the plain gun: the craft's own loadout without parts, else a name the game does not know
    /// (an unknown name arms the plain 37 mm gun, findings section 14.4)
    pub guns_only: String,
}

/// `PLANE_TLL`: `{ head*, size }` of an MSVC `std::map` CRAFT_* -> Plane (findings section 14).
const PLANE_TLL: usize = 0x1_43a1_3c50;
/// Craft node: key at `+0x20`, the loadout pointer vector (each points at a `GameLoadout`) at `+0x48/+0x50`.
const TLL_VECTOR: usize = offset_of!(PlaneNode, data) + offset_of!(Plane, loadouts);
const TLL_VEC: (usize, usize) = (TLL_VECTOR, TLL_VECTOR + size_of::<usize>());
/// `GameLoadout`: oid string `+0`, `vec_parts` begin / end `+0x40` / `+0x48`, stride 0x28 (name `+0`, count `+0x20`).
const LO_PARTS: (usize, usize) = (
    offset_of!(Loadout, parts),
    offset_of!(Loadout, parts) + size_of::<usize>(),
);
const PART_STRIDE: usize = size_of::<Munition>();
const PART_COUNT: usize = offset_of!(Munition, count);
/// The game's ammo list (qol `plane_guns.rs`, 1.163): `{ begin*, end* }` of `Ammo` records of 0x188 bytes, `reticle` (the
/// class: 2 bombs, 3 rockets, 4 gun, 5 K-13) as the first int, the item name string at `+8`.
pub const AMMO_LIST: usize = 0x1_43a1_3be0;
pub const AMMO_SIZE: usize = size_of::<Ammo>();
/// Ceiling on the records read from the ammo list; a longer list means the header is corrupt.
const AMMO_LIST_LIMIT: usize = 10_000;
/// Ceiling on the nodes read from a `std::map` global and on the loadouts of one craft.
const MAP_NODE_LIMIT: usize = 4096;
const CRAFT_LOADOUT_LIMIT: usize = 256;
/// Ceiling on the parts of one loadout; a longer vector means a corrupt header.
const LOADOUT_PART_LIMIT: usize = 16;
/// Ceiling on the battle body array walked by the interceptors.

/// The vanilla table, used when the live one cannot be read: (loadout suffix, item, T-7 count, LA-29 count).
const VANILLA: [(&str, &str, i32, i32); 5] = [
    ("FAB100", "ITEM_FAB100", 4, 2),
    ("FAB250", "ITEM_FAB250", 2, 1),
    ("NURS122", "ITEM_NAR122", 12, 6),
    ("NURS340", "ITEM_NAR340", 2, 0),
    ("K13", "ITEM_K13", 2, 0),
];

/// Button sprites (round icons of the old order menu): guns only / an armament chosen.

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
    let has = |class: i32| {
        parts
            .iter()
            .any(|(n, _)| class_of(n, classes) == Some(class))
    };
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
        item.strip_prefix("ITEM_AMMO_")
            .or_else(|| item.strip_prefix("ITEM_"))
            .unwrap_or(item)
            .to_string()
    }
}

/// "CRAFT_T7" -> "T-7", "CRAFT_LA29" -> "LA-29", "CRAFT_SB15" -> "SB-15", "CRAFT_X" -> "X".
fn craft_label(name: &str) -> String {
    let short = name.strip_prefix("CRAFT_").unwrap_or(name);
    match short.find(|c: char| c.is_ascii_digit()) {
        Some(i) if i > 0 && short[..i].chars().all(|c| c.is_ascii_alphabetic()) => {
            format!("{}-{}", &short[..i], &short[i..])
        }
        _ => short.to_string(),
    }
}

fn make_arm(
    craft: &str,
    key: String,
    parts: Vec<(String, i32)>,
    classes: &HashMap<String, i32>,
) -> Arm {
    let label = parts
        .iter()
        .map(|(n, _)| item_label(n))
        .collect::<Vec<_>>()
        .join(" + ");
    Arm {
        kind: classify(&parts, classes),
        key,
        craft: craft.to_string(),
        label,
        parts,
    }
}

/// The label of a gun-only loadout name: "LOADOUT_MB110_GUN57" -> "57MM GUN",
/// "LOADOUT_LA29_GUN20_2" -> "20MM GUN 2"; anything else the loadout's last name segment.
pub fn gun_label(key: &str) -> String {
    let suffix = key.rsplit_once('_').map_or(key, |(_, last)| last);
    let Some(rest) = suffix.strip_prefix("GUN") else {
        return suffix.to_string();
    };
    let digits: &str = rest
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or(rest);
    if digits.is_empty() {
        return suffix.to_string();
    }
    let tail = rest[digits.len()..].trim_start_matches('_');
    let mut label = format!("{digits}MM GUN");
    if !tail.is_empty() {
        label.push(' ');
        label.push_str(tail);
    }
    label
}

/// A gun-only loadout as an armament: nothing to charge, no inventory read while arming.
fn make_gun_arm(craft: &str, key: String) -> Arm {
    Arm {
        key: key.clone(),
        craft: craft.to_string(),
        label: gun_label(&key),
        parts: Vec::new(),
        kind: Kind::Gun,
    }
}

fn make_craft(name: &str, guns_only: Option<String>) -> Craft {
    let guns_only = guns_only.unwrap_or_else(|| {
        format!(
            "LOADOUT_{}_GUNS",
            name.strip_prefix("CRAFT_").unwrap_or(name)
        )
    });
    Craft {
        name: name.to_string(),
        label: craft_label(name),
        guns_only,
    }
}

/// What the game's own vanilla table holds, used when the live one cannot be read.
pub fn vanilla_table() -> Table {
    let classes = HashMap::new();
    let (mut crafts, mut arms) = (Vec::new(), Vec::new());
    for (craft, short) in [("CRAFT_T7", "T7"), ("CRAFT_LA29", "LA29")] {
        let gun = format!("LOADOUT_{short}_GUN37");
        crafts.push(make_craft(craft, Some(gun.clone())));
        arms.push(make_gun_arm(craft, gun));
        for (suffix, item, t7, la29) in VANILLA {
            let count = if craft == "CRAFT_T7" { t7 } else { la29 };
            if count > 0 {
                arms.push(make_arm(
                    craft,
                    format!("LOADOUT_{short}_{suffix}"),
                    vec![(item.to_string(), count)],
                    &classes,
                ));
            }
        }
    }
    Table { crafts, arms }
}

/// The in-order nodes of an MSVC `std::map` global (`{ head*, size }`); empty when it looks unfilled or unreadable.
unsafe fn map_nodes(global: usize) -> Vec<usize> {
    let (head, size): (usize, usize) = (rd(global), rd(global + 8));
    if size == 0 || size > MAP_NODE_LIMIT || !memory::readable_span(head, 0x20) {
        return Vec::new();
    }
    let is_node = |n: usize| {
        n != head && memory::rd::<u8>(n.wrapping_add(offset_of!(PlaneNode, is_sentinel))) == Some(0)
    };
    let mut out = Vec::new();
    let mut node: usize = rd(head); // head->left = the leftmost node
    while node != head && out.len() < size {
        if !is_node(node) || !memory::readable_span(node, size_of::<PlaneNode>()) {
            return Vec::new();
        }
        out.push(node);
        let right: usize = rd(node + offset_of!(PlaneNode, right));
        // A valid tree cannot ascend or descend more than the number of nodes.
        if is_node(right) {
            node = right;
            for depth in 0..size {
                let Some(left) = memory::rd::<usize>(node) else {
                    return Vec::new();
                };
                if !is_node(left) {
                    break;
                }
                if depth + 1 == size {
                    return Vec::new();
                }
                node = left;
            }
        } else {
            let mut parent: usize = rd(node + offset_of!(PlaneNode, parent));
            for depth in 0..size {
                if !is_node(parent)
                    || memory::rd::<usize>(parent + offset_of!(PlaneNode, right)) != Some(node)
                {
                    break;
                }
                if depth + 1 == size {
                    return Vec::new();
                }
                node = parent;
                let Some(next) = memory::rd::<usize>(node + offset_of!(PlaneNode, parent)) else {
                    return Vec::new();
                };
                parent = next;
            }
            node = parent;
        }
    }
    if node != head || out.len() != size {
        return Vec::new();
    }
    out
}

/// item name -> ammo class, from the game's ammo list (empty when it cannot be read: then the vanilla names decide).
unsafe fn ammo_classes() -> HashMap<String, i32> {
    let mut out = HashMap::new();
    let (b, e): (usize, usize) = (rd(AMMO_LIST), rd(AMMO_LIST + 8));
    if b < 0x10000 || !crate::native::plausible(b) || e < b {
        return out;
    }
    let list_bytes = e - b;
    if !list_bytes.is_multiple_of(AMMO_SIZE)
        || list_bytes / AMMO_SIZE > AMMO_LIST_LIMIT
        || !memory::readable_span(b, list_bytes)
    {
        return out;
    }
    for rec in (b..e).step_by(AMMO_SIZE) {
        if let Some(name) = string_at(rec + offset_of!(Ammo, item_name)) {
            out.insert(name, rd::<i32>(rec + offset_of!(Ammo, reticle)));
        }
    }
    out
}

/// The crafts and armaments read from the live tables: every craft in `PLANE_TLL` (the two vanilla ones first, so their
/// button places stay as they were) and every loadout in its vector: the gun-only ones (no parts, the game's
/// `has_gun37mm` flag) as zero-cost gun armaments with the craft's default first, then the part-carrying ones. `None`
/// when the craft map is empty or unreadable (the tables are filled by `PrepareMainLoop`, and qol's `planes` option
/// replaces them after that).
pub unsafe fn read_table(craft_limit: usize, include_unarmed: bool) -> Option<Table> {
    let nodes = map_nodes(PLANE_TLL);
    if nodes.is_empty() {
        return None;
    }
    let classes = ammo_classes();
    let mut named: Vec<(String, usize)> = nodes
        .iter()
        .filter_map(|&n| Some((string_at(n + offset_of!(PlaneNode, key))?, n)))
        .collect();
    named.sort_by_key(|(name, _)| match name.as_str() {
        "CRAFT_T7" => 0,
        "CRAFT_LA29" => 1,
        _ => 2,
    });
    named.truncate(craft_limit);
    let (mut crafts, mut arms) = (Vec::new(), Vec::new());
    for (craft, node) in named {
        let (b, e): (usize, usize) = (rd(node + TLL_VEC.0), rd(node + TLL_VEC.1));
        let bytes = e.checked_sub(b)?;
        if !bytes.is_multiple_of(8)
            || bytes / 8 > CRAFT_LOADOUT_LIMIT
            || (bytes != 0 && !memory::readable_span(b, bytes))
        {
            return None;
        }
        let mut guns_only: Option<String> = None;
        let mut gun_arms: Vec<Arm> = Vec::new();
        let mut part_arms: Vec<Arm> = Vec::new();
        for lo in (b..e).step_by(8).map(|a| rd::<usize>(a)) {
            if !memory::readable_span(lo, size_of::<Loadout>()) {
                return None;
            }
            let (pb, pe): (usize, usize) = (rd(lo + LO_PARTS.0), rd(lo + LO_PARTS.1));
            let part_bytes = pe.checked_sub(pb)?;
            if !part_bytes.is_multiple_of(PART_STRIDE)
                || part_bytes / PART_STRIDE > LOADOUT_PART_LIMIT
                || (part_bytes != 0 && !memory::readable_span(pb, part_bytes))
            {
                return None;
            }
            let key = string_at(lo).filter(|key| !key.is_empty())?;
            let has_gun = rd::<u8>(lo + offset_of!(Loadout, has_gun37mm)) != 0;
            let parts: Vec<(String, i32)> = (pb..pe)
                .step_by(PART_STRIDE)
                .filter_map(|p| Some((string_at(p)?, rd::<i32>(p + PART_COUNT))))
                .filter(|(n, c)| !n.is_empty() && *c > 0)
                .collect();
            if parts.is_empty() {
                // a gun-only loadout the game itself arms through the `has_gun37mm` flag (qol's `gun_ammo`
                // entries swap in the selected cannon); one without the flag arms nothing at all
                if has_gun {
                    if guns_only.is_none() || key.ends_with("GUN37") {
                        guns_only = Some(key.clone());
                    }
                    gun_arms.push(make_gun_arm(&craft, key));
                } else if include_unarmed {
                    part_arms.push(Arm {
                        label: format!("UNARMED ({})", gun_label(&key)),
                        key,
                        craft: craft.clone(),
                        parts,
                        kind: Kind::Other,
                    });
                }
            } else {
                part_arms.push(make_arm(&craft, key, parts, &classes));
            }
        }
        // the craft's default (gunpod) loadout first, so the choice cycle starts at it (vanilla: the 37 mm)
        if let Some(default) = &guns_only {
            gun_arms.sort_by_key(|arm| arm.key != *default);
        }
        arms.extend(gun_arms);
        arms.extend(part_arms);
        crafts.push(make_craft(&craft, guns_only));
    }
    Some(Table { crafts, arms })
}

pub struct Table {
    pub crafts: Vec<Craft>,
    pub arms: Vec<Arm>,
}

/// stock fallback or campaign inventory filters.
///
/// # Safety
/// Call after the game has initialized its aircraft tables, on the game thread
/// while their owners are not mutating them. The result owns only Rust strings.
pub unsafe fn airstrike_table() -> Option<(Vec<Craft>, Vec<Arm>)> {
    let table = read_table(MAP_NODE_LIMIT, true)?;
    Some((table.crafts, table.arms))
}

/// A MSVC string at `addr` (inline when capacity < 16), `None` when it does not look like one.
pub unsafe fn string_at(addr: usize) -> Option<String> {
    let (len, cap): (usize, usize) = (rd(addr + 0x10), rd(addr + 0x18));
    if len > 200 || cap < len {
        return None;
    }
    let data = if cap < 16 {
        addr
    } else {
        let p: usize = rd(addr);
        if !crate::native::plausible(p) {
            return None;
        }
        p
    };
    if len == 0 {
        return Some(String::new());
    }
    if !memory::readable_span(data, len) {
        return None;
    }
    Some(String::from_utf8_lossy(std::slice::from_raw_parts(data as *const u8, len)).into_owned())
}
