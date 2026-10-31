//! Steam 1.151 gun/KAZ firing-arc rules ported from hf_qol_fork.
//!
//! Evidence: hf_notes/docs/findings/elevation-rules-1151-vs-1163.md.
//! Classic removes the candidate-floor +1 for free-standing parts, widens the KAZ
//! target half-angle to 1.2 radians and uses type-3 rear coverage with threshold 0.5.
//! ClassicKaz additionally skips MDL_FERMA trusses for KAZ ray scans only.
//! Vanilla/modern do not install any of these patches. Every patch checks its bytes.

use std::sync::atomic::{AtomicBool, Ordering};

use patchy::{Condition, PatchSession, Result, ReturnType, Trampoline};
use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleA,
    Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
};

use crate::config::ElevationRules;

const IMAGE_BASE: usize = 0x1_4000_0000;
// World_RaycastBodies: lea edx,[rax+1]; cmovz edx,eax. Replace only cmovz.
const CANDIDATE_SITE: usize = 0x14023a100;
const CANDIDATE_ORIGINAL: [u8; 6] = [0x8d, 0x50, 0x01, 0x0f, 0x44, 0xd0];
const CANDIDATE_PATCH: [u8; 3] = [0x89, 0xc2, 0x90]; // mov edx,eax; nop
                                                     // RCX is the shooter, RSP is aligned; preserve MOV R10,[RCX+0x88].
const SHOOTER_SITE: usize = 0x140239f5b;
const SHOOTER_ORIGINAL: [u8; 7] = [0x4c, 0x8b, 0x91, 0x88, 0x00, 0x00, 0x00];
const SHOOTER_RETURN: usize = 0x140239f62;
// Candidate loop: MOV RCX,RBX; CALL GetPartFlagsWord. The candidate is in RBX.
const CANDIDATE_LOOP_SITE: usize = 0x14023a120;
const CANDIDATE_LOOP_ORIGINAL: [u8; 8] = [0x48, 0x8b, 0xcb, 0xe8, 0xa8, 0xb6, 0xe0, 0xff];
const GET_PART_FLAGS_WORD: usize = 0x1400457d0;
const CANDIDATE_SKIP: usize = 0x14023a1bc;
const CANDIDATE_CONTINUE: usize = 0x14023a128;
const GET_STATS: usize = 0x140281e00;

// Reviewed upstream Body has these fields, but this crate's locked highfleet
// revision 3f6b54b3 does not. Keep the existing dependency unchanged for this port.
// Native Body/std::string offsets and Stats category: findings linked above.
const BODY_PART_INDEX: usize = 0x188;
const BODY_OID: usize = 0x190;
const BODY_OID_LEN: usize = 0x1a0;
const BODY_OID_CAP: usize = 0x1a8;
const STATS_CATEGORY: usize = 0xb0;
const KAZ_CATEGORY: u32 = 1 << 25;
const TRUSS_PREFIX: &[u8; 9] = b"MDL_FERMA";
const MAX_OID_CAPACITY: usize = 0x1000;

// The native ray scan and both callbacks run sequentially on the game thread.
// This flag is scoped by the shooter hook at the start of every native scan;
// it does not confer thread safety or ownership on any game object.
static SHOOTER_IS_KAZ: AtomicBool = AtomicBool::new(false);

// Two movss xmm1,[rip+disp] loads in Weapon_FlakInterceptScan.
const CONE_SITES: [(usize, [u8; 8]); 2] = [
    (
        0x14002f864,
        [0xf3, 0x0f, 0x10, 0x0d, 0x40, 0x2f, 0x32, 0x00],
    ),
    (
        0x14002faf8,
        [0xf3, 0x0f, 0x10, 0x0d, 0xac, 0x2c, 0x32, 0x00],
    ),
];
const PI_OVER_4_ADDRESS: usize = 0x1403527ac;
const CONE_1151_ADDRESS: usize = 0x140354758;
// UpdateEngineCovered type-5 branch, ending before its shared conditional jump.
const COVERED_SITE: usize = 0x14001c68a;
const COVERED_ORIGINAL: [u8; 17] = [
    0x41, 0x0f, 0x2f, 0xf6, 0x72, 0x36, 0xf3, 0x0f, 0x10, 0x05, 0x34, 0x81, 0x33, 0x00, 0x0f, 0x2f,
    0xc6,
];
const COVERED_TYPE_3: usize = 0x14001c65f;
const HALF_ADDRESS: usize = 0x1403527a4;

/// Page-checked borrowed scalar read; never copy owning native containers.
///
/// # Safety
/// The caller establishes native object lifetime and game-thread access. Page
/// checks only reject inaccessible addresses, not stale/reused object identities.
unsafe fn read<T: Copy>(address: usize) -> Option<T> {
    let end = address.checked_add(std::mem::size_of::<T>())?;
    if address < 0x10000 {
        return None;
    }
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(address as *const _, &mut info, std::mem::size_of_val(&info)) == 0
        || info.State != MEM_COMMIT
        || info.Protect == 0
        || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
        || end > (info.BaseAddress as usize).checked_add(info.RegionSize)?
    {
        return None;
    }
    // SAFETY: caller guarantees lifetime/threading; the entire scalar is in a
    // readable region, and unaligned access takes no reference or ownership.
    Some((address as *const T).read_unaligned())
}

/// movss xmm1,[rip+disp32] at `at`, reading `target`.
fn movss_xmm1(at: usize, target: usize) -> [u8; 8] {
    let displacement = (target as i64 - (at as i64 + 8)) as i32;
    let mut bytes = [0xf3, 0x0f, 0x10, 0x0d, 0, 0, 0, 0];
    bytes[4..].copy_from_slice(&displacement.to_le_bytes());
    bytes
}

/// movss xmm10,[0.5f]; jmp type3; nop padding.
fn covered_patch() -> [u8; 17] {
    let mut bytes = [0x90; 17];
    let displacement = (HALF_ADDRESS as i64 - (COVERED_SITE as i64 + 9)) as i32;
    bytes[..5].copy_from_slice(&[0xf3, 0x44, 0x0f, 0x10, 0x15]);
    bytes[5..9].copy_from_slice(&displacement.to_le_bytes());
    bytes[9] = 0xeb;
    bytes[10] = (COVERED_TYPE_3 as i64 - (COVERED_SITE as i64 + 11)) as i8 as u8;
    bytes
}

unsafe extern "C" fn note_shooter(shooter: usize) {
    // SAFETY: byte-validated entry site supplies the live native shooter on the
    // game thread. GetStats borrows the definition indexed by the scalar field;
    // it range-checks the index and returns null when no definition exists.
    let is_kaz = read::<i32>(shooter + BODY_PART_INDEX).is_some_and(|index| {
        let get_stats: unsafe extern "C" fn(i32) -> usize = std::mem::transmute(GET_STATS);
        let stats = get_stats(index);
        stats != 0 && read::<u32>(stats + STATS_CATEGORY) == Some(KAZ_CATEGORY)
    });
    SHOOTER_IS_KAZ.store(is_kaz, Ordering::Relaxed);
}

unsafe extern "C" fn skip_candidate(candidate: usize) -> bool {
    if !SHOOTER_IS_KAZ.load(Ordering::Relaxed) {
        return false;
    }
    // SAFETY: the native candidate loop owns the live body during this callback.
    // Read only bounded bytes of its borrowed MSVC string, never an owning copy.
    let (Some(length), Some(capacity)) = (
        read::<usize>(candidate + BODY_OID_LEN),
        read::<usize>(candidate + BODY_OID_CAP),
    ) else {
        return false;
    };
    if length < TRUSS_PREFIX.len() || length > capacity || capacity > MAX_OID_CAPACITY {
        return false;
    }
    let data = if capacity < 0x10 {
        candidate + BODY_OID
    } else {
        let Some(data) = read::<usize>(candidate + BODY_OID) else {
            return false;
        };
        data
    };
    read::<[u8; 9]>(data) == Some(*TRUSS_PREFIX)
}

/// Prepares classic firing-arc patches; vanilla/modern leave the game untouched.
///
/// # Safety
/// Caller must have verified Steam 1.163 and synchronize native patch installation.
/// Returns patch preparation errors; mismatched native bytes are logged and skipped.
pub unsafe fn patch_elevation_rules(session: &mut PatchSession, rules: ElevationRules) -> Result {
    if !rules.is_classic() {
        log::info!("Elevation rules are the game's own, leaving them alone");
        return Ok(());
    }
    if GetModuleHandleA(std::ptr::null()) as usize != IMAGE_BASE {
        log::error!("Elevation rules: unexpected image base, leaving them alone");
        return Ok(());
    }
    patch_guns(session)?;
    patch_kaz(session)?;
    if rules.exempts_trusses() {
        patch_trusses(session)?;
    }
    Ok(())
}

unsafe fn patch_guns(session: &mut PatchSession) -> Result {
    if read::<[u8; 6]>(CANDIDATE_SITE) == Some(CANDIDATE_ORIGINAL) {
        session.overwrite(CANDIDATE_SITE + 3, &CANDIDATE_PATCH)?;
        log::info!("Elevation rules: free-standing parts no longer block same-level guns");
    } else {
        log::error!("Elevation rules: code at {CANDIDATE_SITE:#x} is not what was expected");
    }
    Ok(())
}

unsafe fn patch_kaz(session: &mut PatchSession) -> Result {
    let cone_ok = read::<f32>(PI_OVER_4_ADDRESS) == Some(std::f32::consts::FRAC_PI_4)
        && read::<f32>(CONE_1151_ADDRESS) == Some(1.2)
        && CONE_SITES
            .iter()
            .all(|&(at, original)| read::<[u8; 8]>(at) == Some(original));
    if cone_ok {
        for (at, _) in CONE_SITES {
            session.overwrite(at, &movss_xmm1(at, CONE_1151_ADDRESS))?;
        }
        log::info!("Elevation rules: KAZ target cone widened to 1.2 rad");
    } else {
        log::error!("Elevation rules: KAZ cone code is not what was expected, leaving it alone");
    }
    if read::<[u8; 17]>(COVERED_SITE) == Some(COVERED_ORIGINAL)
        && read::<f32>(HALF_ADDRESS) == Some(0.5)
    {
        session.overwrite(COVERED_SITE, &covered_patch())?;
        log::info!("Elevation rules: KAZ covered test uses the 1.151 rear cone and 50% threshold");
    } else {
        log::error!("Elevation rules: KAZ covered code is not what was expected, leaving it alone");
    }
    Ok(())
}

unsafe fn patch_trusses(session: &mut PatchSession) -> Result {
    if read::<[u8; 7]>(SHOOTER_SITE) != Some(SHOOTER_ORIGINAL)
        || read::<[u8; 8]>(CANDIDATE_LOOP_SITE) != Some(CANDIDATE_LOOP_ORIGINAL)
    {
        log::error!(
            "Elevation rules: ray scan code is not what was expected, trusses keep blocking"
        );
        return Ok(());
    }
    let mut shooter = Trampoline::new();
    shooter.preserved_call(note_shooter as *const (), &[], &[], ReturnType::None);
    shooter.bytes(&SHOOTER_ORIGINAL);
    shooter.relative_jump(SHOOTER_RETURN);
    session.detour_trampoline(SHOOTER_SITE, SHOOTER_ORIGINAL.len(), shooter)?;

    let mut candidate = Trampoline::new();
    let skip = candidate.new_label();
    candidate.preserved_call(
        skip_candidate as *const (),
        &[0x48, 0x89, 0xd9], // mov rcx,rbx
        &[0x84, 0xc0],       // test al,al
        ReturnType::None,
    );
    candidate.jump_if(Condition::NotEqual, skip);
    candidate.bytes(&CANDIDATE_LOOP_ORIGINAL[..3]);
    candidate.absolute_call(GET_PART_FLAGS_WORD as *const ());
    candidate.relative_jump(CANDIDATE_CONTINUE);
    candidate.bind(skip)?;
    candidate.relative_jump(CANDIDATE_SKIP);
    session.detour_trampoline(
        CANDIDATE_LOOP_SITE,
        CANDIDATE_LOOP_ORIGINAL.len(),
        candidate,
    )?;
    log::info!("Elevation rules: trusses no longer block the KAZ's arc");
    Ok(())
}
