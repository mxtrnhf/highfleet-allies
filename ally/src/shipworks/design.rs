//! Native volume serialization/loading, mirroring the FIGHT and design-save branches.
//! No game-owned container is ever copied into an owning Rust value.

use std::{
    ffi::{CStr, CString},
    mem::{offset_of, size_of},
};

use highfleet::general::SentinelListNode;

use crate::{
    diag,
    native::{read, LINK_STRIDE, OFF_CODE, OFF_SIDE},
    native_layout::{Body, Creature, Link, Node, SentinelList, NODE_OWNER},
};

use super::{native_string, write, Entry};

const NEW: usize = 0x1_4033_4094;
const NODE_CTOR: usize = 0x1_4027_f200;
const SAVE_VOLUME: usize = 0x1_4024_ecc0;
const LOAD_SERIA: usize = 0x1_4022_5930;
const LINK_CTOR: usize = 0x1_4019_4bd0;
const LINK_DTOR: usize = 0x1_4019_4d20;
const RANDOM_IDS: usize = 0x1_4028_0840;
const PROPERTY_SHIP: usize = 0x1_4021_b070;
const CALCULATE_STATS: usize = 0x1_4007_fcf0;
const HAS_THRUST: usize = 0x1_4007_3450;
const PART_HAS_FLAG: usize = 0x1_4004_9470;
const BODY_ARRAY: usize = 0x1_43a3_3c90;
const BODY_COUNT: usize = 0x1_43a5_1150;
const BODY_LIMIT: usize = 15_000;
const PART_LIMIT: usize = 4096;
const CODE_BODY: u64 = 0xf;
const CODE_FRAME: u64 = 0x1f;
const CODE_CREATURE: u64 = 0x2f;
/// Uncovered Creature fields, verified in the FIGHT/save branch (see feature notes).
const SHIP_NAME: usize = 0xa90;
const INITIAL_MORALE: usize = 0x1a20;
const SNAPSHOT_DIR: &str = "Modloader/cache/hf_ally_shipworks";

const _: () = {
    assert!(size_of::<Node>() == 0x128);
    assert!(size_of::<[u64; 3]>() == size_of::<Link>());
    assert!(offset_of!(Node, children) == 0x100);
    assert!(offset_of!(SentinelList<usize>, length) == 8);
    assert!(offset_of!(SentinelListNode<usize>, value) == 0x10);
    assert!(offset_of!(Node, master) + offset_of!(Link, target) == 0x88);
};

/// Verify every additional native call entry before the feature can be used.
pub(super) unsafe fn verify() -> Result<(), String> {
    for (address, expected) in [
        (NEW, &b"\x40\x53\x48\x83\xec\x20\x48\x8b\xd9"[..]),
        (NODE_CTOR, &b"\x48\x89\x5c\x24\x10\x48\x89\x4c\x24\x08"[..]),
        (SAVE_VOLUME, &b"\x48\x89\x5c\x24\x08\x55\x56\x57"[..]),
        (LOAD_SERIA, &b"\x48\x89\x5c\x24\x08\x48\x89\x74\x24\x10"[..]),
        (LINK_CTOR, &b"\x40\x53\x48\x83\xec\x20\x48\x8b\xd9"[..]),
        (
            LINK_DTOR,
            &b"\x4c\x8b\x41\x08\x48\x8d\x05\xe5\xd5\x1b\x00"[..],
        ),
        (RANDOM_IDS, &b"\x48\x89\x5c\x24\x08\x48\x89\x6c\x24\x10"[..]),
        (
            PROPERTY_SHIP,
            &b"\x48\x85\xc9\x74\x20\x48\x83\xb9\x08\x01\x00\x00\x00"[..],
        ),
        (
            CALCULATE_STATS,
            &b"\x48\x8b\xc4\x44\x88\x40\x18\x88\x50\x10"[..],
        ),
        (HAS_THRUST, &b"\x48\x83\xb9\xa8\x0a\x00\x00\x10"[..]),
        (
            PART_HAS_FLAG,
            &b"\x40\x53\x48\x83\xec\x20\x8b\x89\x88\x01\x00\x00"[..],
        ),
    ] {
        if std::slice::from_raw_parts(address as *const u8, expected.len()) != expected {
            return Err(format!(
                "Shipworks design: unexpected native bytes at {address:#x}"
            ));
        }
    }
    Ok(())
}

struct EditorShip {
    creature: usize,
    root: usize,
}

type PartHasFlag = unsafe extern "C" fn(usize, u32) -> u8;

/// Select the bridge's owner AND the bridge's master, not the Creature's master.
/// Native save: 0x14024d9b0..0x14024da02 searches frame children too; 0x14024dad5
/// takes the bridge master; 0x14024db3a takes its owner.
unsafe fn editor_ship() -> Result<EditorShip, &'static str> {
    let count = diag::rd::<u32>(BODY_COUNT).ok_or("unreadable editor body count")? as usize;
    if count == 0 || count > BODY_LIMIT {
        return Err("editor body count outside supported bounds");
    }
    let has_flag: PartHasFlag = std::mem::transmute(PART_HAS_FLAG);
    for index in 0..count {
        let body = diag::rd::<usize>(BODY_ARRAY + index * size_of::<usize>())
            .ok_or("unreadable editor body array")?;
        if body == 0 {
            continue;
        }
        let Some(bridge) = find_bridge(body, has_flag)? else {
            continue;
        };
        let root = diag::rd::<usize>(bridge + offset_of!(Node, master) + offset_of!(Link, target))
            .ok_or("unreadable bridge master")?;
        if !diag::readable_span(root, size_of::<Body>())
            || read::<u64>(root + OFF_CODE) & CODE_BODY != CODE_BODY
        {
            return Err("bridge has no attached design root");
        }
        let creature = diag::rd::<usize>(bridge + NODE_OWNER).ok_or("unreadable bridge owner")?;
        if !diag::readable_span(creature, size_of::<Creature>())
            || read::<u64>(creature + OFF_CODE) != CODE_CREATURE
        {
            return Err("bridge owner is not a valid Creature");
        }
        return Ok(EditorShip { creature, root });
    }
    Err("no bridge found in editor bodies or frame children")
}

/// Frame parts are not necessarily separate members of the global Body array.
/// Traverse only a validated frame's immediate children, as the native save routine does.
unsafe fn find_bridge(body: usize, has_flag: PartHasFlag) -> Result<Option<usize>, &'static str> {
    if !diag::readable_span(body, size_of::<Body>()) {
        return Err("unreadable editor body");
    }
    // SAFETY: body is a current Body-array member (or a child of the current frame), alive
    // on the input thread. Its complete Body span is readable before calling the native query.
    if has_flag(body, 0x40) != 0 {
        return Ok(Some(body));
    }
    if read::<u64>(body + OFF_CODE) & CODE_FRAME != CODE_FRAME {
        return Ok(None);
    }
    let list = body + offset_of!(Node, children);
    let sentinel = diag::rd::<usize>(list).ok_or("unreadable frame child sentinel")?;
    let count = diag::rd::<usize>(list + offset_of!(SentinelList<usize>, length))
        .ok_or("unreadable frame child count")?;
    if count > PART_LIMIT {
        return Err("frame child count outside supported bounds");
    }
    let mut cell = diag::rd::<usize>(sentinel).ok_or("unreadable frame child list")?;
    for _ in 0..count {
        if cell == sentinel {
            return Err("frame child list ended before its count");
        }
        let child = diag::rd::<usize>(cell + offset_of!(SentinelListNode<usize>, value))
            .ok_or("unreadable frame child")?;
        if !diag::readable_span(child, size_of::<Body>()) {
            return Err("unreadable frame child body");
        }
        if has_flag(child, 0x40) != 0 {
            return Ok(Some(child));
        }
        cell = diag::rd::<usize>(cell).ok_or("unreadable next frame child")?;
    }
    if cell != sentinel {
        return Err("frame child list exceeds its count");
    }
    Ok(None)
}

/// Collect top-level ship parts plus the root, as ShipDesign_LoadFromFile/SaveVolume does.
unsafe fn volume_roots(selected: &EditorShip) -> Option<Vec<usize>> {
    let header = selected.creature + offset_of!(Creature, parts);
    let begin = diag::rd::<usize>(header)?;
    let end = diag::rd::<usize>(header + size_of::<usize>())?;
    let bytes = end.checked_sub(begin)?;
    if bytes == 0 || !bytes.is_multiple_of(LINK_STRIDE) || bytes / LINK_STRIDE > PART_LIMIT {
        return None;
    }
    if !diag::readable_span(begin, bytes) {
        return None;
    }
    let mut roots = Vec::new();
    for index in 0..bytes / LINK_STRIDE {
        let part: usize = read(begin + index * LINK_STRIDE + offset_of!(Link, target));
        if part == 0 {
            continue;
        }
        let parent = diag::rd::<usize>(part + offset_of!(Node, master) + offset_of!(Link, target))?;
        if diag::rd::<u64>(parent + OFF_CODE)? & CODE_BODY != CODE_BODY {
            roots.push(part);
        }
    }
    if !roots.contains(&selected.root) {
        roots.push(selected.root);
    }
    Some(roots)
}

/// Capture into a mod-owned, session-temporary file. Serialization is entirely game-native.
///
/// # Safety
/// Input callback on the game thread, outside render/native container traversal. SaveVolume
/// temporarily reparents the validated roots and restores them before returning.
pub(super) unsafe fn snapshot(serial: u64) -> Result<Entry, String> {
    let selected = editor_ship().map_err(str::to_owned)?;
    let roots = volume_roots(&selected).ok_or_else(|| {
        format!(
            "invalid design part/root collection: creature={:#x}, bridge_master={:#x}",
            selected.creature, selected.root,
        )
    })?;
    let name = native_string(selected.creature + SHIP_NAME)
        .ok_or_else(|| "invalid current ship name".to_owned())?;
    // Prevent native text markup / line breaks in a design name from altering the roster layout.
    let display: Vec<u8> = name
        .as_bytes()
        .iter()
        .copied()
        .filter(|&byte| byte >= 0x20 && !matches!(byte, b'@' | b'{' | b'}'))
        .collect();
    let name = if display.is_empty() {
        c"UNNAMED SHIP".into()
    } else {
        CString::new(display).map_err(|error| format!("invalid display name: {error}"))?
    };
    std::fs::create_dir_all(SNAPSHOT_DIR)
        .map_err(|error| format!("cannot create snapshot directory: {error}"))?;
    let path_string = format!("{SNAPSHOT_DIR}/{}_{}.seria", std::process::id(), serial);
    let path = CString::new(path_string.as_bytes())
        .map_err(|error| format!("invalid snapshot path: {error}"))?;

    // Plain storage, NOT copied Link objects. Native construction registers each link at its
    // final stable address; native destruction unregisters it before the Rust allocation is freed.
    let mut links = vec![[0u64; 3]; roots.len()];
    let construct: unsafe extern "C" fn(*mut u64, usize) = std::mem::transmute(LINK_CTOR);
    let destroy: unsafe extern "C" fn(*mut u64) = std::mem::transmute(LINK_DTOR);
    for (storage, root) in links.iter_mut().zip(roots) {
        construct(storage.as_mut_ptr(), root);
    }
    let begin = links.as_mut_ptr() as usize;
    // Borrowed vector header. SaveVolume reads this header; it neither reallocates nor frees it.
    let end = begin + links.len() * LINK_STRIDE;
    let mut header = [begin, end, end];
    let save: unsafe extern "C" fn(*const i8, *const i8, *mut usize) -> u8 =
        std::mem::transmute(SAVE_VOLUME);
    let saved = save(path.as_ptr(), name.as_ptr(), header.as_mut_ptr()) != 0;
    for storage in &mut links {
        destroy(storage.as_mut_ptr());
    }
    if !saved {
        let _ = std::fs::remove_file(&path_string);
        return Err(format!("native SaveVolume failed for {path_string}"));
    }
    let metadata = std::fs::metadata(&path_string)
        .map_err(|error| format!("cannot verify saved snapshot {path_string}: {error}"))?;
    if metadata.len() == 0 {
        let _ = std::fs::remove_file(&path_string);
        return Err(format!(
            "native SaveVolume produced an empty file: {path_string}"
        ));
    }
    Ok(Entry { name, path })
}

/// Deserialize one fresh volume, then transfer ownership to the already-live FIGHT fleet.
/// `side` is written to the ship's `+0xad4`: positive for the player fleet, negative for the
/// enemy fleet.
///
/// # Safety
/// Only the verified FIGHT seams supply `fleet`, on the game thread. The volume's native
/// constructor/deserializer/init/load and deleting destructor retain allocator/Link ownership.
pub(super) unsafe fn append(fleet: usize, path: &CStr, side: f32) -> bool {
    if !diag::readable_span(fleet, 0x618) {
        return false;
    }
    let allocate: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(NEW);
    let construct: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(NODE_CTOR);
    let memory = allocate(size_of::<Node>());
    if memory == 0 {
        return false;
    }
    let volume = construct(memory);
    let load: unsafe extern "C" fn(usize, *const i8) -> u8 = std::mem::transmute(LOAD_SERIA);
    if load(volume, path.as_ptr()) == 0 {
        delete_volume(volume);
        return false;
    }
    let vtable: usize = read(volume);
    let initialize: unsafe extern "C" fn(usize, usize) =
        std::mem::transmute(read::<usize>(vtable + 0x80));
    let randomize: unsafe extern "C" fn(usize, u8) = std::mem::transmute(RANDOM_IDS);
    let load_resources: unsafe extern "C" fn(usize) =
        std::mem::transmute(read::<usize>(vtable + 0x88));
    initialize(volume, 0);
    randomize(volume, 1);
    load_resources(volume);
    let get_ship: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(PROPERTY_SHIP);
    let ship = get_ship(volume);
    if ship == 0 || !diag::readable_span(ship, size_of::<Creature>()) {
        delete_volume(volume);
        return false;
    }
    let stats: unsafe extern "C" fn(usize, u8, u8) = std::mem::transmute(CALCULATE_STATS);
    let has_thrust: unsafe extern "C" fn(usize) -> u8 = std::mem::transmute(HAS_THRUST);
    stats(ship, 0, 0);
    if has_thrust(ship) == 0 {
        delete_volume(volume);
        return false;
    }
    write(ship + INITIAL_MORALE, 10.0f32);
    write(ship + OFF_SIDE, side);
    let fleet_vtable: usize = read(fleet);
    let add: unsafe extern "C" fn(usize, usize, usize, usize) =
        std::mem::transmute(read::<usize>(fleet_vtable + 0x50));
    add(fleet, volume, 0, 0);
    true
}

unsafe fn delete_volume(volume: usize) {
    let vtable: usize = read(volume);
    let destroy: unsafe extern "C" fn(usize, u32) = std::mem::transmute(read::<usize>(vtable));
    destroy(volume, 1);
}
