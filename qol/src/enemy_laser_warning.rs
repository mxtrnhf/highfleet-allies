//! Optional suppression of enemy pre-fire gun lines in Steam 1.163.
//!
//! In BattleHud_DrawCombatMarkers (0x14029c5c0), these two calls draw
//! group-0 and group-1 gun warnings. RSI is the current creature and XMM8 is
//! zero. The immediately following native code uses alignment at +0xad4 to
//! distinguish hostile ships for the warning sound. Preserve that code.

use patchy::{Condition, PatchError, PatchSession, Result, Trampoline};

const DRAW_WORLD_LINE: usize = 0x140248ba0;
const SITES: [(usize, [u8; 5]); 2] = [
    (0x14029d664, [0xe8, 0x37, 0xb5, 0xfa, 0xff]),
    (0x14029d956, [0xe8, 0x45, 0xb2, 0xfa, 0xff]),
];

pub unsafe fn prepare(session: &mut PatchSession) -> Result {
    for (address, expected) in SITES {
        if !crate::intro::matches_executable_bytes(address, &expected) {
            return Err(PatchError::SourceChanged { address });
        }
    }
    for (address, _) in SITES {
        let mut trampoline = Trampoline::new();
        let skip_draw = trampoline.new_label();
        // COMISS XMM8,[RSI+0xad4]: zero >= alignment means hostile.
        trampoline.bytes(&[0x44, 0x0f, 0x2f, 0x86, 0xd4, 0x0a, 0x00, 0x00]);
        trampoline.jump_if(Condition::AboveOrEqual, skip_draw);
        trampoline.absolute_call(DRAW_WORLD_LINE as *const ());
        trampoline.bind(skip_draw)?;
        trampoline.relative_jump(address + 5);
        // SAFETY: The exact original call is checked above; RSI and XMM8 have
        // the meanings established by the enclosing 1.163 render loop. The
        // original argument registers and stack are untouched before the call.
        unsafe { session.detour_trampoline(address, 5, trampoline)? };
    }
    log::info!("Prepared enemy pre-fire laser warning suppression");
    Ok(())
}
