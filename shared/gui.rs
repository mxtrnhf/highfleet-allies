//! Native GUI registry and control primitives for Steam 1.163.

use crate::{
    native::{plausible, read, GUI_ROOT, OFF_ACTIVE},
    native_layout::{Gui, Link, Node},
};
use std::{
    mem::{offset_of, size_of},
    sync::atomic::Ordering,
};

pub const GUI_LIST: usize = 0x1_43a1_3ac8;
pub const GUI_COUNT: usize = 0x1_43a1_3ad0;
pub const GUI_NAME: usize = offset_of!(Gui, identifier);

// --- GUI element recipe (read from the PRECOMBAT_OK block of 0x1402d7620, see precombat-screen.md) ---
pub const GUI_NEW: usize = 0x1_4033_4094; // operator new(size)
pub const GUI_NEW_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
pub const GUI_CTOR: usize = 0x1_4012_a910; // GUI_2(mem, 0)
pub const GUI_CTOR_PROLOGUE: [u8; 10] =
    [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08];
pub const GUI_SETPOS: usize = 0x1_4005_20b0; // setXposYpos(obj, Vec2*)
pub const GUI_SETPOS_PROLOGUE: [u8; 10] =
    [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec, 0xb0];

/// The element list is walked to its own count plus this slack, capped so a corrupt count cannot hang the
/// game thread. A walk that reaches the cap is reported as incomplete and is never used to hide our elements.
const GUI_SCAN_SLACK: usize = 16;
const GUI_SCAN_MIN: usize = 256;
const GUI_SCAN_MAX: usize = 4096;

/// A NUL-terminated ASCII name at `address`, at most 48 bytes, printable only.
pub const GUI_NAME_READ_LIMIT: usize = 48;

pub unsafe fn cstr(address: usize) -> String {
    (0..GUI_NAME_READ_LIMIT)
        .map(|i| read::<u8>(address + i))
        .take_while(|&b| (0x20..0x7f).contains(&b))
        .map(|b| b as char)
        .collect()
}

/// Compare a fixed GUI identifier with the same 48-byte printable-ASCII boundary as `cstr`,
/// without allocating a name for every element in a per-frame scan.
pub unsafe fn cstr_is(address: usize, expected: &[u8]) -> bool {
    if expected.len() > GUI_NAME_READ_LIMIT {
        return false;
    }
    for (index, &byte) in expected.iter().enumerate() {
        if read::<u8>(address + index) != byte {
            return false;
        }
    }
    expected.len() == GUI_NAME_READ_LIMIT
        || !(0x20..0x7f).contains(&read::<u8>(address + expected.len()))
}

type NewFn = unsafe extern "C" fn(usize) -> *mut u8;
type CtorFn = unsafe extern "C" fn(*mut u8, i32) -> *mut u8;
pub type GuiInitFn = unsafe extern "C" fn(
    *mut u8,
    *const u8,
    f32,
    f32,
    f32,
    f32,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
    f32,
);

type NodeAddFn = unsafe extern "C" fn(*mut u8, *mut u8, usize, usize);

pub type ResetFn = unsafe extern "C" fn(*mut u8);
type SetPosFn = unsafe extern "C" fn(*mut u8, *const f32);

pub struct GuiScan {
    pub elements: Vec<usize>,
    /// False when the walk hit the cap before the sentinel: the list is only partially known.
    pub complete: bool,
    pub visited: usize,
    pub limit: usize,
}

pub unsafe fn scan_gui_list() -> GuiScan {
    let head: usize = read(GUI_LIST);
    let count: usize = read(GUI_COUNT);
    let limit = count
        .saturating_add(GUI_SCAN_SLACK)
        .clamp(GUI_SCAN_MIN, GUI_SCAN_MAX);
    let mut elements = Vec::new();
    let mut visited = 0;
    let mut complete = false;
    if plausible(head) {
        let mut node: usize = read(head);
        for _ in 0..limit {
            if node == head {
                break;
            }
            if !plausible(node) {
                break;
            }
            visited += 1;
            let element: usize = read(node + 0x10);
            if !plausible(element) {
                break;
            }
            elements.push(element);
            node = read(node);
        }
        complete = node == head;
    }
    GuiScan {
        elements,
        complete,
        visited,
        limit,
    }
}

/// Writes the game's shown (2) / unused (0) state to one of our elements.
pub unsafe fn set_gui_active(gui: usize, active: bool) {
    if gui == 0 {
        return;
    }
    (gui as *mut i32)
        .byte_add(OFF_ACTIVE)
        .write_unaligned(if active { 2 } else { 0 });
}

/// Parent node of an element (`Node.master` at 0x80, its `Link.target` at +8 => 0x88 in 1.163). A live element
/// placed by `make_gui` is a child of the global root.
pub unsafe fn master_node(gui: usize) -> usize {
    read(gui + offset_of!(Node, master) + offset_of!(Link, target))
}

/// Cell of the `GUI_ELEMENTS` registry entry (+0xba8, see docs/ally-gui-v0.14.5-tester.md). Non-zero means the element is
/// registered and can still be found by name; it says nothing about the dead-node queue.
pub unsafe fn registry_cell(gui: usize) -> usize {
    read(gui + 0xba8)
}

/// Checks registry/root fields of an element already obtained from the current native list.
/// Callers must prove current membership BEFORE calling this function: it dereferences the pointer and is
/// not itself a lifetime check for a cached address.
pub unsafe fn gui_live_in_root(gui: usize) -> bool {
    if !plausible(gui) || registry_cell(gui) == 0 {
        return false;
    }
    let root: usize = read(GUI_ROOT);
    plausible(root) && master_node(gui) == root
}

/// Checks a GUI already obtained from the current complete `GUI_ELEMENTS` walk for the expected name,
/// registration and root ownership. Callers must establish membership before calling, especially for cached
/// addresses. Detached or foreign-parented objects are
/// never resurrected: `Node::add` does not clear a dead-node queue entry, so re-adding one could crash the
/// native `CollectDeadNodes` pass.
pub unsafe fn gui_reusable(gui: usize, name: &[u8]) -> bool {
    if !plausible(gui) {
        return false;
    }
    if cstr(gui + GUI_NAME).as_bytes() != &name[..name.len() - 1] {
        return false;
    }
    // only a clean hidden (0) or active (2) object may be restored; 1 = removed/dying
    match read::<i32>(gui + OFF_ACTIVE) {
        0 | 2 => {}
        _ => return false,
    }
    gui_live_in_root(gui)
}

/// True when `gui` is in the scan's GUI list, still carries `name` and is currently shown
/// (active 2). Used to switch our own stale elements off; it never authorises reuse, which needs
/// the registry/root checks of [`gui_reusable`].
///
/// # Safety
/// Use the current game-thread scan with no intervening GUI mutations; `name` must include its NUL.
pub unsafe fn listed_shown(listed: &[usize], gui: usize, name: &[u8]) -> bool {
    listed.contains(&gui)
        && read::<i32>(gui + OFF_ACTIVE) == 2
        && cstr_is(gui + GUI_NAME, &name[..name.len() - 1])
}

/// Calls the element's vtable slot +0x88 (`GUI::Load`): resolves the element's own media names and loads the
/// resources of every sprite in its list, then recurses into children. Only call on a newly constructed GUI
/// or a current registry member validated by `get_or_make`, on the game thread outside rendering.
pub unsafe fn load_gui(gui: usize) {
    if gui == 0 {
        return;
    }
    let vtable: usize = read(gui);
    if !plausible(vtable) {
        return;
    }
    let load: ResetFn = std::mem::transmute(read::<usize>(vtable + 0x88));
    load(gui as *mut u8);
}

/// Allocate/construct native GUI storage, without initializing a control recipe.
///
/// # Safety
/// Game-thread callback with a live global root; caller must initialize and attach the new GUI.
pub unsafe fn allocate_gui() -> *mut u8 {
    let root: usize = read(GUI_ROOT);
    if root == 0 {
        return std::ptr::null_mut();
    }
    let new: NewFn = std::mem::transmute(GUI_NEW);
    let ctor: CtorFn = std::mem::transmute(GUI_CTOR);
    let mem = new(size_of::<Gui>());
    if mem.is_null() {
        return mem;
    }
    ctor(mem, 0)
}

/// Transfer a freshly constructed/initialized control into the native GUI root.
///
/// # Safety
/// The pointer must come from allocate_gui in this callback, never a detached or cached object.
pub unsafe fn attach_gui(gui: *mut u8) {
    let root: usize = read(GUI_ROOT);
    let node_vtable: usize = read(root);
    let add: NodeAddFn = std::mem::transmute(read::<usize>(node_vtable + 0x50));
    add(root as *mut u8, gui, 0, 0);
}

/// Same guarded reuse as get_or_make, with a feature-specific native constructor recipe.
///
/// # Safety
/// The list must belong to this game-thread callback. `create` must return a freshly native-
/// constructed, initialized, root-attached GUI with the requested name, or null on failure.
pub unsafe fn get_or_make_with(
    name: &[u8],
    kept_slot: &std::sync::atomic::AtomicUsize,
    listed: &[usize],
    complete: bool,
    create: impl FnOnce() -> *mut u8,
) -> Option<(usize, bool)> {
    if !complete {
        // an incomplete list cannot prove the kept object is gone; do not create duplicates
        return None;
    }
    let kept = kept_slot.load(Ordering::Relaxed);
    if listed.contains(&kept) && gui_reusable(kept, name) {
        set_gui_active(kept, true);
        return Some((kept, false));
    }
    // The snapshot also includes hidden objects. Never fall back to an unbounded native search after a
    // truncated scan, and never read an unlisted cached pointer merely to compare its old name.
    for &gui in listed {
        if gui != kept && gui_reusable(gui, name) {
            set_gui_active(gui, true);
            kept_slot.store(gui, Ordering::Relaxed);
            return Some((gui, false));
        }
    }
    if kept != 0 {
        log::debug!(
            "GUI: retained {} is no longer reusable; discarding its cached address without reviving it",
            String::from_utf8_lossy(&name[..name.len() - 1])
        );
        // drop the cached number before allocating: a stale pointer must never be read again
        kept_slot.store(0, Ordering::Relaxed);
    }
    let ptr = create();
    if ptr.is_null() {
        return None;
    }
    kept_slot.store(ptr as usize, Ordering::Relaxed);
    Some((ptr as usize, true))
}

/// Move a freshly created or currently validated native GUI and set its rectangular hit box.
///
/// # Safety
/// Same game-thread and live-membership requirements as `load_gui`.
pub unsafe fn move_gui_rect(gui: usize, pos: [f32; 2], size: [f32; 2]) {
    let set_pos: SetPosFn = std::mem::transmute(GUI_SETPOS);
    set_pos(gui as *mut u8, pos.as_ptr());
    (gui as *mut f32).byte_add(0x218).write_unaligned(size[0]);
    (gui as *mut f32).byte_add(0x21c).write_unaligned(size[1]);
}
