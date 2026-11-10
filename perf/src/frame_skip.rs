//! Restore the render-skip decision that 1.163 computes and discards.
//!
//! `_FrameFunc` computes the boolean and passes it in `ECX` to
//! `SomethingVideoTask` `0x140316e20`; the callee never reads it. We wrap the
//! entry, and when the boolean is false we set the game's own one-shot skip
//! flag `DAT_143a11963`. The renderer then takes its existing epilogue path
//! (`0x140316eac`), so the rest of the frame is untouched.

use std::{
    slice,
    sync::atomic::{AtomicBool, Ordering},
};

use patchy::{PatchSession, Trampoline};

const VIDEO_TASK: usize = 0x1_4000_0000 + 0x316e20;
const SKIP_FLAG: usize = 0x1_4000_0000 + 0x3a11963;
// MOV RAX,RSP ; MOV [RAX+8],RBX. Both complete instructions; the next
// instruction starts at VIDEO_TASK + 7.
const VIDEO_PROLOGUE: [u8; 7] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08];

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Validate the fixed entry bytes before installing the wrapper.
///
/// # Safety
///
/// The caller must be in the startup-only installation path with the supported
/// 1.163 image mapped at the fixed base. This reads executable bytes only.
pub unsafe fn validate() -> Result<(), String> {
    let found = unsafe { slice::from_raw_parts(VIDEO_TASK as *const u8, VIDEO_PROLOGUE.len()) };
    if found != VIDEO_PROLOGUE {
        return Err(format!(
            "video-task prologue differs at {VIDEO_TASK:#x}; refusing frame-skip hook"
        ));
    }
    Ok(())
}

/// Prepare the entry detour. The trampoline calls the hook, restores the
/// Win64 stack alignment, replays the two stolen instructions, and resumes at
/// `VIDEO_TASK + 7`.
///
/// # Safety
///
/// Same startup-only, fixed-image, single-game-thread contract as the other
/// hf_perf patches. The caller must retain the DLL for process lifetime.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    unsafe { validate()? };

    let mut trampoline = Trampoline::new();
    // At function entry RSP % 16 == 8. Win64 requires 32 bytes of shadow space
    // for the callee plus 8 bytes to land the call at RSP % 16 == 8.
    trampoline.bytes(&[0x48, 0x83, 0xec, 0x28]); // sub rsp, 0x28
    trampoline.absolute_call(video_entry as *const ());
    trampoline.bytes(&[0x48, 0x83, 0xc4, 0x28]); // add rsp, 0x28
    trampoline.bytes(&VIDEO_PROLOGUE);
    trampoline.absolute_jump(VIDEO_TASK + VIDEO_PROLOGUE.len());

    // SAFETY: the source range is the validated fixed-image prologue; the
    // trampoline preserves the entry stack/register contract and resumes at a
    // complete instruction boundary.
    unsafe {
        session
            .detour_trampoline(VIDEO_TASK, VIDEO_PROLOGUE.len(), trampoline)
            .map_err(|error| error.to_string())?;
    }
    ENABLED.store(true, Ordering::Release);
    Ok(())
}

/// Entry hook. `render` is the boolean from `_FrameFunc` (`0` means the stock
/// pacing heuristic asked to skip this render). When it is zero, set the game's
/// existing one-shot skip flag; the native video task then takes its own early
/// out. No other game state is touched.
unsafe extern "win64" fn video_entry(render: u32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // Diagnostic counters: calls vs decisions, so a zero skip count can be
    // distinguished from a hook that never ran.
    crate::profile::count(9);
    if render == 0 {
        // SAFETY: fixed image .bss byte, written on the game thread only.
        unsafe { (SKIP_FLAG as *mut u8).write_volatile(1) };
        crate::profile::count(8);
        crate::profile::count(10);
    }
}
