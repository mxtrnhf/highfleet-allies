//! Native-code adapters only; the spatial algorithm and borrowed view are separate.

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::hud_query;

const WRAPPER_SITE: usize = 0x1_4029_c668;
const WRAPPER_BYTES: [u8; 5] = [0xe8, 0x13, 0x8b, 0xfa, 0xff];
const BODY_ARRAY: usize = 0x1_43a3_3c90;
const LOOPS: [(usize, usize, [u8; 7]); 2] = [
    (
        0x1_4024_54e0,
        0x1_4024_5586,
        [0x4c, 0x8d, 0x35, 0xa9, 0xe7, 0x7e, 0x03],
    ),
    (
        0x1_4024_5baf,
        0x1_4024_5c56,
        [0x4c, 0x8d, 0x35, 0xda, 0xe0, 0x7e, 0x03],
    ),
];

/// Caller guarantees mapped Steam 1.163 executable and startup quiescence.
pub unsafe fn validate() -> Result<(), String> {
    // Whole reviewed function fingerprints include stack-frame input/output,
    // predicates, loop branches and callsites, not merely patch prologues.
    for (address, length, expected) in [
        (hud_query::WARNING, 0x1677, 0x38fbaf20_u32),
        (0x1_4031_98d0, 0x1cf, 0xeff65687),
        (0x1_4031_8b20, 0x356, 0xde36f725),
    ] {
        // SAFETY: Bounded original executable code ranges, no game heap access.
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, length) };
        let fingerprint = bytes.iter().fold(0x811c9dc5_u32, |hash, &byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
        });
        if fingerprint != expected {
            return Err(format!(
                "HUD/shape code fingerprint differs at {address:#x}"
            ));
        }
    }
    for (address, expected) in [
        (WRAPPER_SITE, WRAPPER_BYTES.as_slice()),
        (
            0x1_4000_5e60,
            &[0x83, 0x79, 0x48, 0x02, 0x0f, 0x9d, 0xc0, 0xc3],
        ),
        (LOOPS[0].0, LOOPS[0].2.as_slice()),
        (LOOPS[1].0, LOOPS[1].2.as_slice()),
    ] {
        // SAFETY: Same fixed, bounded mapped executable ranges as above.
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
        if bytes != expected {
            return Err(format!("HUD hook bytes differ at {address:#x}"));
        }
    }
    Ok(())
}

/// Prepare all three disjoint patches; caller installs the session at startup.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    for (fan, &(entry, continuation, expected)) in LOOPS.iter().enumerate() {
        let mut trampoline = Trampoline::new();
        let native = trampoline.new_label();
        // RSP is aligned at this native mid-function callsite. preserved_call
        // saves 7 GPRs and 0x88 bytes (0xc0 total), plus volatile XMM0-XMM5.
        // RCX = [RBP-0x70] input; RDX = original [RSP+0x68] 12-byte scratch;
        // R8D = fan ordinal. Nonvolatile Win64 registers/XMM6-XMM15 stay intact.
        let mut arguments = vec![
            0x48, 0x8d, 0x4d, 0x90, 0x48, 0x8d, 0x94, 0x24, 0x28, 0x01, 0x00, 0x00, 0x41, 0xb8,
        ];
        arguments.extend_from_slice(&(fan as u32).to_le_bytes());
        trampoline.preserved_call(
            hud_query::query_warning as *const (),
            &arguments,
            &[0x84, 0xc0],
            ReturnType::None,
        );
        trampoline.jump_if(Condition::Equal, native);
        // Native loop results: RSI = nearest body; XMM6.low = fraction. All
        // other live caller state stays as it was. Native interpolation, point
        // allocation, fan rotation, flag cleanup and drawing are not replaced.
        trampoline.bytes(&[
            0x48, 0x8b, 0x74, 0x24, 0x68, 0xf3, 0x0f, 0x10, 0x74, 0x24, 0x70,
        ]);
        trampoline.relative_jump(continuation);
        trampoline.bind(native).map_err(|error| error.to_string())?;
        // Replay displaced RIP-relative LEA with equivalent MOV R14,imm64,
        // preserving flags, rather than incorrectly relocating its disp32.
        trampoline.bytes(&[0x49, 0xbe]);
        trampoline.bytes(&BODY_ARRAY.to_le_bytes());
        trampoline.relative_jump(entry + expected.len());
        // SAFETY: Validated complete 7-byte instruction. Both exits preserve
        // native live values; fallback executes the original loop unchanged.
        unsafe { session.detour_trampoline(entry, expected.len(), trampoline) }
            .map_err(|error| error.to_string())?;
    }
    let mut wrapper = Trampoline::new();
    wrapper.absolute_call(hud_query::draw_warning_scope as *const ());
    wrapper.relative_jump(WRAPPER_SITE + WRAPPER_BYTES.len());
    // SAFETY: Replaces a void no-argument CALL with the same ABI; wrapper calls
    // its original target synchronously and always clears callback-local views.
    unsafe { session.detour_trampoline(WRAPPER_SITE, WRAPPER_BYTES.len(), wrapper) }
        .map_err(|error| error.to_string())?;
    Ok(())
}
