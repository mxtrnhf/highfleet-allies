//! Vanilla KAZ first-target scan: cheap rejection before native virtual/distance calls.
//! The second detonation pass and all firing/effects code remain native.
//! Evidence and limits: docs/frame-profiling.md and docs/kaz-fast-scan.md.

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

const LOOP: usize = 0x14002f690;
const LOOP_LEN: usize = 0x24e;
const LOOP_FINGERPRINT: u32 = 0x9241709e;
const ENTRY_BYTES: [u8; 9] = [0x48, 0x85, 0xed, 0x0f, 0x85, 0x45, 0x02, 0x00, 0x00];
const FOUND: usize = 0x14002f8de;
const NO_TARGET: usize = 0x14002ff0a;
const GET_STATS: usize = 0x140281e00;
const DISTANCE: usize = 0x14020fb80;
const NORMALIZE: usize = 0x140018ce0;
const ATAN2: usize = 0x1403365e8;
const BODY_ARRAY: usize = 0x143a33c90;
const BODY_COUNT: usize = 0x143a51150;
const DAMAGE_THRESHOLD: usize = 0x1403ef040;
const MAX_BODIES: usize = 15_000;
// Exact native f32 bit patterns, not rounded source-level approximations.
const TARGET_CONE: f32 = f32::from_bits(0x3f490fdb);
const APPROACH_CONE: f32 = f32::from_bits(0x3fc90fdb);

const STATS_BYTES: [u8; 16] = [
    0x83, 0xf9, 0xff, 0x74, 0x22, 0x48, 0x8b, 0x15, 0xe4, 0x1d, 0x7b, 0x03, 0x48, 0x63, 0xc1, 0x48,
];
const DISTANCE_BYTES: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xda, 0x48, 0x85, 0xc9, 0x0f, 0x84, 0xd1, 0x00,
];
const NORMALIZE_BYTES: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x40, 0x0f, 0x29, 0x74, 0x24, 0x30, 0x48, 0x8b, 0xda, 0x0f, 0x29,
];
const ATAN2_BYTES: [u8; 16] = [
    0xff, 0x25, 0x82, 0xb1, 0x01, 0x00, 0xff, 0x25, 0x74, 0xb1, 0x01, 0x00, 0xff, 0x25, 0x4e, 0xb1,
];

type StatsFn = unsafe extern "win64" fn(i32) -> *const u8;
type DistanceFn = unsafe extern "win64" fn(*const u8, *const u8) -> f32;
type NormalizeFn = unsafe extern "win64" fn(*const [f32; 2], *mut [f32; 2]) -> *mut [f32; 2];
type Atan2Fn = unsafe extern "win64" fn(f32, f32) -> f32;
type ActiveFn = unsafe extern "win64" fn(*const u8) -> u8;

/// Check mapped executable bytes during startup; no runtime game objects are read.
pub unsafe fn validate() -> Result<(), String> {
    // SAFETY: fixed bounded ranges in the validated main image.
    let code = unsafe { std::slice::from_raw_parts(LOOP as *const u8, LOOP_LEN) };
    let fingerprint = code.iter().fold(0x811c9dc5_u32, |hash, &byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
    });
    if fingerprint != LOOP_FINGERPRINT {
        return Err("KAZ target-loop fingerprint differs".into());
    }
    for (address, expected) in [
        (GET_STATS, STATS_BYTES.as_slice()),
        (DISTANCE, DISTANCE_BYTES.as_slice()),
        (NORMALIZE, NORMALIZE_BYTES.as_slice()),
        (ATAN2, ATAN2_BYTES.as_slice()),
    ] {
        // SAFETY: same fixed bounded executable ranges.
        let found = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
        if found != expected {
            return Err(format!("KAZ helper bytes differ at {address:#x}"));
        }
    }
    Ok(())
}

/// Replace only the first target loop. Native code resumes with RBP=target or
/// returns zero when no target exists, exactly as the original loop's exits do.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    let mut trampoline = Trampoline::new();
    let found = trampoline.new_label();
    // Native RSP is 16-byte aligned at this mid-function call site. patchy saves
    // 0xc0 bytes of volatile state. RCX=RDI weapon; XMM0/XMM1=XMM10/XMM11
    // direction; original [RSP+0x138/+0x13c] position becomes +0x1f8/+0x1fc.
    let arguments = [
        0x48, 0x89, 0xf9, // mov rcx,rdi
        0x41, 0x0f, 0x28, 0xc2, // movaps xmm0,xmm10
        0x41, 0x0f, 0x28, 0xcb, // movaps xmm1,xmm11
        0xf3, 0x0f, 0x10, 0x94, 0x24, 0xf8, 0x01, 0x00, 0x00, // movss xmm2,[rsp+1f8]
        0xf3, 0x0f, 0x10, 0x9c, 0x24, 0xfc, 0x01, 0x00, 0x00, // movss xmm3,[rsp+1fc]
    ];
    trampoline.preserved_call(
        select_target as *const (),
        &arguments,
        &[0x48, 0x89, 0xc5, 0x48, 0x85, 0xc0], // mov rbp,rax; test rax,rax
        ReturnType::Rax,
    );
    trampoline.jump_if(Condition::NotEqual, found);
    trampoline.relative_jump(NO_TARGET);
    trampoline.bind(found).map_err(|error| error.to_string())?;
    trampoline.relative_jump(FOUND);
    // SAFETY: nine complete bytes at the loop head are replaced. Every exit
    // bypasses rather than replays the old loop; nonvolatile native state and
    // XMM6-XMM15 remain live for the untouched firing/detonation continuation.
    unsafe { session.detour_trampoline(LOOP, ENTRY_BYTES.len(), trampoline) }
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[inline]
unsafe fn read<T: Copy>(address: usize) -> T {
    // SAFETY: callers use scalar fields of live native objects/array entries on
    // the game thread, under the same lifetime preconditions as the original loop.
    unsafe { (address as *const T).read_unaligned() }
}

#[inline]
unsafe fn normalize(function: NormalizeFn, value: [f32; 2]) -> [f32; 2] {
    let mut result = [0.0; 2];
    // SAFETY: native helper borrowed from the reviewed executable; input/output
    // are live Rust stack vectors for the duration of this synchronous call.
    unsafe { function(&value, &mut result) };
    result
}

/// Return the first native target in original body-array order. The filters are
/// conjunctions; moving pure stats/scalar gates earlier cannot select a body the
/// native conjunction would reject. NaN comparisons intentionally pass just as
/// native COMISS/JA does. No candidate pointers are retained after this call.
unsafe extern "win64" fn select_target(
    weapon: usize,
    axis_x: f32,
    axis_y: f32,
    weapon_x: f32,
    weapon_y: f32,
) -> usize {
    crate::profile::count(5);
    // SAFETY: all native functions/objects are the original caller's live game
    // state. No object ownership, container copy, or cross-callback cache exists.
    unsafe {
        let count = read::<u32>(BODY_COUNT) as usize;
        if count > MAX_BODIES {
            return 0;
        }
        let stats: StatsFn = std::mem::transmute(GET_STATS);
        let distance: DistanceFn = std::mem::transmute(DISTANCE);
        let normalize_vectors: NormalizeFn = std::mem::transmute(NORMALIZE);
        let atan2: Atan2Fn = std::mem::transmute(ATAN2);
        let threshold = read::<f32>(DAMAGE_THRESHOLD);
        let weapon_has_owner = read::<usize>(weapon + 0xb8) != 0;
        let weapon_owner = read::<usize>(weapon + 0xc0);
        let mut prefilter_rejections = 0_u64;

        for index in 0..count {
            let body = read::<usize>(BODY_ARRAY + index * 8);
            if body == 0 {
                continue;
            }
            let stats_record = stats(read::<i32>(body + 0x188));
            let category = if stats_record.is_null() {
                0
            } else {
                read::<u32>(stats_record as usize + 0xb0)
            };
            let excluded =
                matches!(category, 0x4000 | 0x8000000 | 0x200000) || (category >> 10) & 1 != 0;
            let weak = threshold > read::<f32>(body + 0x2e0) + read::<f32>(body + 0x2dc);
            let inactive_layer = read::<i32>(body + 0x1f0) == 0;
            let own_body = weapon_has_owner && read::<usize>(body + 0xc0) == weapon_owner;
            if excluded || weak || inactive_layer || own_body {
                prefilter_rejections += 1;
                continue;
            }

            let active: ActiveFn = std::mem::transmute(read::<usize>(read::<usize>(body) + 0x78));
            if active(body as *const u8) == 0
                || distance(weapon as *const u8, body as *const u8) > 60.0
            {
                continue;
            }

            let delta = normalize(
                normalize_vectors,
                [
                    read::<f32>(body + 0x128) - weapon_x,
                    read::<f32>(body + 0x12c) - weapon_y,
                ],
            );
            let bearing = atan2(
                delta[1] * axis_x - delta[0] * axis_y,
                delta[1] * axis_y + delta[0] * axis_x,
            );
            // Native COMISS/JBE also rejects an unordered/NaN bearing.
            if !matches!(
                (-bearing).abs().partial_cmp(&TARGET_CONE),
                Some(std::cmp::Ordering::Less)
            ) {
                continue;
            }
            let velocity = normalize(
                normalize_vectors,
                [read::<f32>(body + 0x130), read::<f32>(body + 0x134)],
            );
            let approach = atan2(
                velocity[1] * axis_x - velocity[0] * axis_y,
                velocity[1] * axis_y + velocity[0] * axis_x,
            );
            if (-approach).abs() > APPROACH_CONE {
                crate::profile::count(6);
                crate::profile::count_by(7, prefilter_rejections);
                return body;
            }
        }
        crate::profile::count_by(7, prefilter_rejections);
        0
    }
}
