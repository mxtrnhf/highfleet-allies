//! Immediate control handoff at the start of the stock combat manager tick.
//!
//! Native contracts: hf_notes/docs/findings/battle-ally.md (control links/AI list/roles).
//! Input fields and TAB were rechecked against live 1.163 on 2026-11-03; see
//! docs/ship-switching.md. Shared Link, Node and Creature offsets come from
//! highfleet-rs; the boost and bomb-held words stay local RE offsets.

use std::mem::{offset_of, MaybeUninit};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::native_layout::Link;

pub static ENABLED: AtomicBool = AtomicBool::new(false);
static HELD_KEYS: AtomicU8 = AtomicU8::new(0b111);
const KEYS: [i32; 3] = [0x31, 0x32, 0x09]; // 1, 2, TAB
const IN_COMBAT: usize = 0x1_47fc_6fb7;
const FRAME_DT: usize = 0x1_403c_126c;
const ACTIVE_DIALOG: usize = 0x1_43a1_3e08;
const OFF_BOOST: usize = 0xba8;
const OFF_BOMB_HELD: usize = 0xce8;
/// Native AI ship vector (`CVec<Link>`): SomethingAITask visits only entries here.
const AI_VECTOR: usize = 0x1_43fa_a608;
/// Native `Link` helpers used by Creature_RegisterInWorldLists for exactly this append.
const LINK_CONSTRUCT: usize = 0x1_4019_4bd0;
const LINK_CONSTRUCT_BYTES: [u8; 12] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0xc2,
];
const LINK_COPY: usize = 0x1_4019_4b80;
const LINK_COPY_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20];
const LINK_UNLINK: usize = 0x1_4019_4d20;
const LINK_UNLINK_BYTES: [u8; 10] = [0x4c, 0x8b, 0x41, 0x08, 0x48, 0x8d, 0x05, 0xe5, 0xd5, 0x1b];
const LINK_VECTOR_GROW: usize = 0x1_4000_4b00;
const LINK_VECTOR_GROW_BYTES: [u8; 10] =
    [0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x56, 0x57, 0x41, 0x54];

type LinkConstructFn = unsafe extern "win64" fn(*mut Link, usize) -> *mut Link;
type LinkCopyFn = unsafe extern "win64" fn(*mut Link, *const Link) -> *mut Link;
type LinkUnlinkFn = unsafe extern "win64" fn(*mut Link) -> *mut Link;
type LinkVecGrowFn = unsafe extern "win64" fn(usize, *mut Link, *const Link) -> *mut Link;

/// Verify the native AI-vector append/remove path before any battle hook is installed.
pub fn verify_native() -> Result<(), String> {
    for (name, address, expected) in [
        ("link_construct", LINK_CONSTRUCT, &LINK_CONSTRUCT_BYTES[..]),
        ("link_copy", LINK_COPY, &LINK_COPY_BYTES[..]),
        ("link_unlink", LINK_UNLINK, &LINK_UNLINK_BYTES[..]),
        (
            "link_vector_grow",
            LINK_VECTOR_GROW,
            &LINK_VECTOR_GROW_BYTES[..],
        ),
    ] {
        let found = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
        if found != expected {
            return Err(format!(
                "{name} at {address:#x} has unexpected bytes {found:02x?}"
            ));
        }
    }
    Ok(())
}

struct AiVector {
    address: usize,
    begin: usize,
    end: usize,
    capacity_end: usize,
    stride: usize,
}

/// Validate the three raw vector pointers and derive a bounded length.
unsafe fn ai_vector() -> Option<AiVector> {
    let address = AI_VECTOR;
    let begin = crate::read::<usize>(address);
    let end = crate::read::<usize>(address + 8);
    let capacity_end = crate::read::<usize>(address + 16);
    let stride = std::mem::size_of::<Link>();
    if begin < 0x10000 || end < begin || capacity_end < end {
        return None;
    }
    if !(end - begin).is_multiple_of(stride) {
        return None;
    }
    let capacity = capacity_end - begin;
    if !capacity.is_multiple_of(stride) || capacity / stride > crate::CREATURE_LIST_LIMIT {
        return None;
    }
    Some(AiVector {
        address,
        begin,
        end,
        capacity_end,
        stride,
    })
}

impl AiVector {
    unsafe fn contains(&self, ship: usize) -> bool {
        let mut entry = self.begin;
        while entry < self.end {
            if crate::read::<usize>(entry + offset_of!(Link, target)) == ship {
                return true;
            }
            entry += self.stride;
        }
        false
    }

    /// Append exactly as `Creature_RegisterInWorldLists` does: construct a temporary Link, copy it
    /// into spare capacity or grow the vector natively, then release the temporary registration.
    unsafe fn add(&mut self, ship: usize) -> bool {
        if self.contains(ship) {
            return true;
        }
        let construct: LinkConstructFn = std::mem::transmute(LINK_CONSTRUCT);
        let copy: LinkCopyFn = std::mem::transmute(LINK_COPY);
        let unlink: LinkUnlinkFn = std::mem::transmute(LINK_UNLINK);
        let grow: LinkVecGrowFn = std::mem::transmute(LINK_VECTOR_GROW);
        let mut link = MaybeUninit::<Link>::uninit();
        construct(link.as_mut_ptr(), ship);
        if self.end == self.capacity_end {
            grow(self.address, self.end as *mut Link, link.as_ptr());
        } else {
            copy(self.end as *mut Link, link.as_ptr());
            (self.address as *mut usize)
                .byte_add(8)
                .write_unaligned(self.end + self.stride);
        }
        unlink(link.as_mut_ptr());
        if let Some(vector) = ai_vector() {
            *self = vector;
        }
        self.contains(ship)
    }

    /// Remove every Link to `ship`, keeping each Link's native node registration consistent while
    /// the remaining entries shift down into the vacated slots.
    unsafe fn remove(&mut self, ship: usize) -> bool {
        let copy: LinkCopyFn = std::mem::transmute(LINK_COPY);
        let unlink: LinkUnlinkFn = std::mem::transmute(LINK_UNLINK);
        let mut removed = false;
        loop {
            let length = (self.end - self.begin) / self.stride;
            let mut index = None;
            for candidate in 0..length {
                let entry = self.begin + candidate * self.stride;
                if crate::read::<usize>(entry + offset_of!(Link, target)) == ship {
                    index = Some(candidate);
                    break;
                }
            }
            let Some(index) = index else {
                break;
            };
            for slot in index..length - 1 {
                let destination = self.begin + slot * self.stride;
                let source = destination + self.stride;
                unlink(destination as *mut Link);
                copy(destination as *mut Link, source as *const Link);
            }
            let last = self.begin + (length - 1) * self.stride;
            unlink(last as *mut Link);
            self.end = last;
            (self.address as *mut usize)
                .byte_add(8)
                .write_unaligned(self.end);
            removed = true;
        }
        removed
    }
}

/// The AI list holds exactly the AI-run ships: add the demoted ship before promotion, and remove the
/// promoted ally so SomethingAITask cannot fight the player's input on the newly flown ship.
unsafe fn rebalance_ai_list(demoted: usize, promoted: usize) -> bool {
    let Some(mut vector) = ai_vector() else {
        log::warn!("ALLY switch ignored: native AI vector at {AI_VECTOR:#x} is unreadable");
        return false;
    };
    if !vector.add(demoted) {
        log::warn!(
            "ALLY switch ignored: c={demoted:#x} could not be registered in the native AI list"
        );
        return false;
    }
    if !vector.remove(promoted) {
        log::warn!("ALLY switch: c={promoted:#x} was not registered in the native AI list");
    }
    true
}

/// Only current roster members may reach this function; they remain game-owned for this tick.
unsafe fn available(ship: usize) -> bool {
    crate::live_ship(ship)
        && crate::read::<f32>(ship + crate::OFF_SIDE) > 0.0
        && matches!(crate::read::<i32>(ship + crate::OFF_STATE), 0 | 1)
        && crate::read::<i32>(ship + crate::OFF_MODE_D70) != crate::ORDER_BOMBING
}

/// Move latched player input, not the ally's last AI fire command, to the newly controlled ship.
unsafe fn transfer_input(previous: usize, selected: usize) {
    // SAFETY: Called on the game thread with two distinct current live roster members. Only verified
    // scalar input fields are borrowed; no owning native values/references are constructed. The
    // native Input_KeyDown/Up routines own these fields (0x1401896a0 / 0x14018a530).
    for index in 0..6 {
        let offset = crate::OFF_FLAGS + index;
        let held = crate::read::<u8>(previous + offset);
        (previous as *mut u8).add(offset).write(0);
        // Bomb release is a one-shot command; never carry it across a handoff.
        (selected as *mut u8)
            .add(offset)
            .write(if index == 2 { 0 } else { held });
    }
    let boost = crate::read::<f32>(previous + OFF_BOOST);
    (previous as *mut f32)
        .byte_add(OFF_BOOST)
        .write_unaligned(1.0);
    (selected as *mut f32)
        .byte_add(OFF_BOOST)
        .write_unaligned(boost);
    (previous as *mut u8).add(OFF_BOMB_HELD).write(0);
    (selected as *mut u8).add(OFF_BOMB_HELD).write(0);
}

unsafe fn handoff(previous: usize, selected: usize) {
    if previous == selected
        || !available(selected)
        || crate::read::<u8>(selected + crate::OFF_AI_FLAG) == 0
    {
        return;
    }
    let get_profile: crate::GetProfileFn = std::mem::transmute(crate::GET_PROFILE);
    let profile = get_profile();
    if !crate::plausible(profile) {
        log::warn!("ALLY switch ignored: profile pointer {profile:#x} is not readable");
        return;
    }
    let Ok(mut allies) = crate::ALLIES.lock() else {
        return;
    };
    let Some(index) = allies.iter().position(|&ship| ship == selected) else {
        return;
    };
    // Preserve the original player's funded ammo before an AI doctrine can change its shell index.
    let Ok(funding) = crate::AMMO_FUNDING.lock() else {
        return;
    };
    let recorded = funding.iter().any(|entry| entry.0 == previous);
    drop(funding);
    if !recorded {
        crate::record_ammo_funding(previous);
    }
    // No mod mutex is held across calls back into native game code.
    allies[index] = previous;
    drop(allies);
    if !rebalance_ai_list(previous, selected) {
        if let Ok(mut allies) = crate::ALLIES.lock() {
            if let Some(index) = allies.iter().position(|&ship| ship == selected) {
                allies[index] = selected;
            }
        }
        return;
    }

    // SAFETY: This is the version/prologue-checked combat hook, before native manager traversal.
    // Both objects belong to this frame's roster; native SetControlled/SetLink retain ownership and
    // maintain link registration. No game-owned allocation is copied or dropped. SetControlled also
    // rebuilds native battle backgrounds. Role 1 must exist on exactly the controlled roster ship.
    let set_controlled: crate::SetControlledFn = std::mem::transmute(crate::SET_CONTROLLED);
    let set_link: crate::SetLinkFn = std::mem::transmute(crate::SET_LINK);
    (previous as *mut i32)
        .byte_add(crate::OFF_MODE_D70)
        .write_unaligned(crate::ALLY_MODE);
    (selected as *mut i32)
        .byte_add(crate::OFF_MODE_D70)
        .write_unaligned(1);
    set_controlled(profile, selected, 0);
    set_link(crate::CONTROLLED_LINKS, selected);
    transfer_input(previous, selected);
    // The old player occupies the chosen ally's panel, and max_allies is unchanged.
    crate::ally_panels::invalidate();
    let ai_ships = ai_vector().map_or(0, |v| (v.end - v.begin) / v.stride);
    log::info!(
        "ALLY control switched: {previous:#x} -> {selected:#x}; previous ship now native AI (AI list {ai_ships} ships)"
    );
}

/// Poll once per combat-manager tick. Held keys are consumed even while switching is disallowed.
///
/// # Safety
/// `ships` must be the current native roster, read on the game thread before native traversal.
/// Native functions are usable only after the mod's version and prologue checks have succeeded.
pub unsafe fn tick(ships: &[usize]) {
    let mut down = 0u8;
    for (index, key) in KEYS.iter().enumerate() {
        if GetAsyncKeyState(*key) as u16 & 0x8000 != 0 {
            down |= 1 << index;
        }
    }
    let pressed = down & !HELD_KEYS.swap(down, Ordering::Relaxed);
    if ships.is_empty() {
        crate::ally_panels::invalidate();
    }
    if pressed == 0 || !ENABLED.load(Ordering::Relaxed) || !crate::ARMED.load(Ordering::Relaxed) {
        return;
    }
    let mut foreground_process = 0;
    GetWindowThreadProcessId(GetForegroundWindow(), &mut foreground_process);
    let dt = crate::read::<f32>(FRAME_DT);
    if crate::PRECOMBAT_LIVE.load(Ordering::Relaxed)
        || foreground_process != GetCurrentProcessId()
        || crate::read::<u8>(IN_COMBAT) == 0
        || !dt.is_finite()
        || dt <= 0.0
        || crate::read::<usize>(ACTIVE_DIALOG) != 0
    {
        return;
    }
    let previous = crate::read::<usize>(crate::CONTROLLED_LINKS + offset_of!(Link, target));
    if !ships.contains(&previous)
        || !available(previous)
        || crate::read::<i32>(previous + crate::OFF_MODE_D70) != 1
        || crate::read::<u8>(previous + crate::OFF_AI_FLAG) != 0
    {
        return;
    }
    let selected = if pressed & 0b011 != 0 {
        let slot = if pressed & 1 != 0 { 0 } else { 1 };
        crate::ally_panels::displayed_ship(slot)
    } else {
        // Stable native roster order, including the original player and allies beyond the two panels.
        let Some(current_index) = ships.iter().position(|&ship| ship == previous) else {
            return;
        };
        let mut next = None;
        for step in 1..ships.len() {
            let Some(&ship) = ships.get((current_index + step) % ships.len()) else {
                return;
            };
            if available(ship) && crate::read::<u8>(ship + crate::OFF_AI_FLAG) != 0 {
                next = Some(ship);
                break;
            }
        }
        next
    };
    let Some(selected) = selected.filter(|ship| ships.contains(ship)) else {
        return;
    };
    handoff(previous, selected);
}
