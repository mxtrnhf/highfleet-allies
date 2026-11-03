//! Steam 1.163 group-0 range gate only: native barrel alignment, friendly-fire,
//! magazine and burst logic remain untouched. Native latched bursts bypass those
//! checks on subsequent ticks; this is not a guarantee against friendly fire.
//! Evidence: hf_notes/docs/findings/ai/ai-tick-patch-sites.md §5, and live
//! disassembly/bytes rechecked 2026-11-04 at 0x14007ba84..0x14007ba8e.

use std::panic::{catch_unwind, AssertUnwindSafe};

use patchy::{Condition, PatchSession, ProcessModule, ReturnType, Trampoline};

const RANGE_RVA: usize = 0x7ba84;
// comiss xmm8,xmm1; ja 0x14007bc3b
const RANGE_BYTES: [u8; 10] = [0x44, 0x0f, 0x2f, 0xc1, 0x0f, 0x87, 0xad, 0x01, 0x00, 0x00];
const SKIP_RVA: usize = 0x7bc3b;

unsafe extern "C" fn allow_range(creature: usize, distance: f32) -> bool {
    // Rust panics fail closed; this cannot catch native access violations.
    catch_unwind(AssertUnwindSafe(|| {
        crate::tactics::early_fire_allowed(creature, distance)
    }))
    .unwrap_or(false)
}

/// Prepare the range-only detour after validating the complete displaced branch.
/// A mismatch leaves stock firing intact. The caller installs one patch session.
pub unsafe fn prepare(session: &mut PatchSession, module: ProcessModule) -> Result<(), String> {
    let address = module.resolve_rva(RANGE_RVA).map_err(|e| e.to_string())?;
    // SAFETY: fixed-size page-checked scalar-array read from the game image.
    if crate::rd::<[u8; 10]>(address) != Some(RANGE_BYTES) {
        log::error!("hf_ai: early-fire range site differs; keeping stock firing");
        return Ok(());
    }
    let mut code = Trampoline::new();
    let pass = code.new_label();
    let skip = code.new_label();
    // Preserve the stock COMISS outcomes, including unordered (native JA is false).
    code.bytes(&RANGE_BYTES[..4]);
    code.jump_if(Condition::BelowOrEqual, pass);
    // Function prologue leaves RSP 16-byte aligned here. preserved_call saves
    // volatile GPRs/XMMs; nonvolatile RDI (creature) and XMM8 (distance) survive.
    // Windows x64: first argument RCX, second floating argument XMM1.
    code.preserved_call(
        allow_range as *const (),
        &[0x48, 0x89, 0xf9, 0x41, 0x0f, 0x28, 0xc8],
        &[0x84, 0xc0], // test al,al before restoring RAX
        ReturnType::None,
    );
    code.jump_if(Condition::Equal, skip);
    // Restore the original comparison flags on both destinations, too.
    code.bind(pass).map_err(|e| e.to_string())?;
    code.bytes(&RANGE_BYTES[..4]);
    code.absolute_jump(
        module
            .resolve_rva(RANGE_RVA + RANGE_BYTES.len())
            .map_err(|e| e.to_string())?,
    );
    code.bind(skip).map_err(|e| e.to_string())?;
    code.bytes(&RANGE_BYTES[..4]);
    code.absolute_jump(module.resolve_rva(SKIP_RVA).map_err(|e| e.to_string())?);
    session
        .detour_trampoline(address, RANGE_BYTES.len(), code)
        .map_err(|e| e.to_string())?;
    log::info!("hf_ai: group-0 early-fire range gate prepared; native alignment and friendly-fire checks retained");
    Ok(())
}
