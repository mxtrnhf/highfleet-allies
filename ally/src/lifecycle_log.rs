//! Spawn and Creature lifecycle logging: the SPAWN and ADD/REMOVE entry hooks, their line
//! budgets, the pending level name and the scene tag.
//!
//! The SPAWN trampoline calls [`crate::battle::on_spawn`], which owns battle-start resets and
//! asks this module for the current log line through [`log_spawn`]. No generic event layer is
//! involved: battle state resets and logging stay explicit.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Mutex,
};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};

use crate::diag;
use crate::native::{
    count, read, CLOCK, CONTROLLED_LINK, CREATURES_BEGIN, CREATURES_END, IMAGE_BASE,
    OFF_AGGRESSION, OFF_AI_FLAG, OFF_MODE, OFF_SIDE, OFF_STATE, OFF_TARGET, SCENE_FLAGS,
    SCENE_SELECTOR,
};

/// `FUN_14006bf60(creature, removing)`: the Creature add-to-level / remove routine (vtable entry).
const ADD_RVA: usize = 0x6bf60;
/// `mov [rsp+8],rbx; push rdi; sub rsp,0x40` (10 bytes, whole instructions).
const ADD_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40];

/// `0x1402672c0(template, &pos, 0, radio)`: spawns a Creature from a ship template node.
const SPAWN_RVA: usize = 0x2672c0;
/// `mov [rsp+0x10],rbx; mov [rsp+0x18],rsi` (10 bytes, rsp-relative, safe to replay first).
const SPAWN_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18];
/// The pending level name, a game `std::string` (MSVC layout: buffer or pointer at +0, size at
/// +0x10, capacity at +0x18); written by the level loader `0x14025afd0` through `CopyString`.
const LEVEL_NAME: usize = 0x1_403c_1390;

/// Vector of creatures the AI task runs (same entry layout): begin / end.
const AI_BEGIN: usize = 0x1_43fa_a608;
const AI_END: usize = 0x1_43fa_a610;
/// Side buckets (vectors of raw 8-byte pointers): begin / end.
const NEG_BEGIN: usize = 0x1_43fa_a5f0;
const NEG_END: usize = 0x1_43fa_a5f8;
const ZERO_BEGIN: usize = 0x1_43fa_a650;
const ZERO_END: usize = 0x1_43fa_a658;
const POS_BEGIN: usize = 0x1_43fa_a638;
const POS_END: usize = 0x1_43fa_a640;
/// Order index, `i32`, given by the Missile scenario.
const OFF_ORDER: usize = 0xc98;

const MAX_LINES: u32 = 600;
const DEBUG_MAX_LINES: u32 = 1_000_000;
static LINES: AtomicU32 = AtomicU32::new(0);
static SPAWN_LINES: AtomicU32 = AtomicU32::new(0);
static LAST_LEVEL: Mutex<String> = Mutex::new(String::new());
/// Signature of the last ADD/REMOVE line and how many identical ones were swallowed since.
static LAST_SIG: Mutex<(usize, u8, u32, u32, usize)> = Mutex::new((0, 0, 0, 0, 0));

/// Addresses verified before the sound check and the patch session is created.
pub(crate) struct LifecycleHooks {
    add: usize,
    spawn: usize,
}

/// Verifies the ADD/SPAWN entry bytes without installing anything.
///
/// The ADD resolve is unconditional, exactly as before; only its byte check and later hook are
/// conditional on `log_creatures`.
/// Returns resolution errors or mismatched required entry bytes without staging any patches.
///
/// # Safety
/// Call during serialized initialization in the verified Steam 1.163 preferred-base image.
/// The resolved native entries must be mapped readable code, with no racing code modifications.
pub(crate) unsafe fn verify_hooks(
    module: &ProcessModule,
    log_creatures: bool,
) -> Result<LifecycleHooks, String> {
    let add = module.resolve_rva(ADD_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(add as *const u8, ADD_PROLOGUE.len());
    if log_creatures && found != ADD_PROLOGUE {
        return Err(format!(
            "bytes at {add:#x} are {found:02x?}, expected {ADD_PROLOGUE:02x?} (another mod owns this site, or a different build)"
        ));
    }
    let spawn = module.resolve_rva(SPAWN_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(spawn as *const u8, SPAWN_PROLOGUE.len());
    if found != SPAWN_PROLOGUE {
        return Err(format!(
            "bytes at {spawn:#x} are {found:02x?}, expected {SPAWN_PROLOGUE:02x?} (another mod owns this site, or a different build)"
        ));
    }
    Ok(LifecycleHooks { add, spawn })
}

/// Installs the SPAWN detour and, when enabled, the ADD/REMOVE entry hook.
/// Returns patch-construction errors; the caller commits the complete session.
///
/// # Safety
/// `hooks` must be from this initialization pass, with its checked entry bytes unchanged.
/// Call before native callbacks can run concurrently with installation.
pub(crate) unsafe fn install_hooks(
    session: &mut PatchSession,
    hooks: LifecycleHooks,
    log_creatures: bool,
) -> Result<(), String> {
    // At the function entry RSP is 8 mod 16 and [RSP] is the return address. Replay the two
    // rsp-relative stores, pad 8 bytes to align, then call with the return address (now at
    // RSP+0xC8 behind the helper's 7 pushes and 0x88 bytes) in RDX; RCX is still the template.
    let mut trampoline = Trampoline::new();
    trampoline.bytes(&SPAWN_PROLOGUE);
    trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp, 8
    trampoline.preserved_call(
        crate::battle::on_spawn as *const (),
        &[0x48, 0x8b, 0x94, 0x24, 0xc8, 0x00, 0x00, 0x00], // mov rdx, [rsp+0xC8]
        &[],
        ReturnType::None,
    );
    trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]); // add rsp, 8
    trampoline.absolute_jump(hooks.spawn + SPAWN_PROLOGUE.len());
    session
        .detour_trampoline(hooks.spawn, SPAWN_PROLOGUE.len(), trampoline)
        .map_err(|e| e.to_string())?;
    if log_creatures {
        session
            .patch_call(
                hooks.add,
                on_creature as *const (),
                ADD_PROLOGUE.len(),
                true,
                ReturnType::None,
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resets the per-battle SPAWN line budget at the current battle-start location.
pub(crate) fn reset_spawn_budget() {
    SPAWN_LINES.store(0, Ordering::Relaxed);
}

/// `sel=<scenario state> fl=<campaign><battle><pending>` for tagging lines when the name is unreadable.
///
/// # Safety
/// Call on the game thread while the supported image's scalar scene globals are readable.
pub(crate) unsafe fn scene() -> String {
    format!(
        "sel={} fl={}{}{}",
        read::<u32>(SCENE_SELECTOR),
        read::<u8>(SCENE_FLAGS[0]),
        read::<u8>(SCENE_FLAGS[1]),
        read::<u8>(SCENE_FLAGS[2])
    )
}

/// The pending level name, read defensively.
unsafe fn level_name() -> String {
    let capacity: usize = read(LEVEL_NAME + 0x18);
    let size: usize = read(LEVEL_NAME + 0x10);
    if size == 0 || size > capacity || size > 64 {
        return String::from("?");
    }
    let data = if capacity > 0xf {
        read::<usize>(LEVEL_NAME)
    } else {
        LEVEL_NAME
    };
    if data == 0 {
        return String::from("?");
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(data as *const u8, size)).into_owned()
}

/// The `ALLY ADD`/`ALLY REMOVE` and `ALLY SPAWN` line budget. Debug mode removes the practical limit.
fn line_budget() -> u32 {
    if diag::debug() {
        DEBUG_MAX_LINES
    } else {
        MAX_LINES
    }
}

/// Returns the level tag for a new line, or `None` when this level's line budget is spent.
/// A changed level name resets the budget and writes a `LEVEL` line.
unsafe fn next_line() -> Option<String> {
    let level = level_name();
    {
        let mut last = LAST_LEVEL.lock().ok()?;
        if *last != level {
            *last = level.clone();
            LINES.store(0, Ordering::Relaxed);
            log::info!(
                "ALLY LEVEL {level} t={:.2} raw_size={} raw_cap={}",
                read::<f64>(CLOCK),
                read::<usize>(LEVEL_NAME + 0x10),
                read::<usize>(LEVEL_NAME + 0x18)
            );
        }
    }
    if LINES.fetch_add(1, Ordering::Relaxed) >= line_budget() {
        return None;
    }
    Some(level)
}

/// Writes the SPAWN diagnostic and budgeted `ALLY SPAWN` line for one spawn.
///
/// # Safety
/// Call from the native spawner callback on the game thread. A nonzero `template` must be its
/// still-valid template node; the supported image's level-name and scene globals must be readable.
pub(crate) unsafe fn log_spawn(template: usize, caller: usize) {
    crumb!(
        "SPAWN template={template:#x} class={:#x} caller_rva={:#x} {} t={:.2}",
        if template != 0 {
            read::<u64>(template + crate::native::OFF_CODE)
        } else {
            0
        },
        caller.wrapping_sub(IMAGE_BASE),
        scene(),
        read::<f64>(CLOCK)
    );
    diag::snapshot(format_args!(
        "spawn caller_rva={:#x}",
        caller.wrapping_sub(IMAGE_BASE)
    ));
    // spawns are rare and the point of the exercise: their own budget, never shared with ADD lines
    let level = level_name();
    let spawn_n = SPAWN_LINES.fetch_add(1, Ordering::Relaxed);
    let budget = line_budget();
    if spawn_n >= budget {
        if spawn_n == budget {
            log::info!("ALLY SPAWN lines: budget reached for this battle, the rest are not logged");
        }
        return;
    }
    log::info!(
        "ALLY SPAWN level={level} {} t={:.2} template={template:#x} class={:#x} caller_rva={:#x}",
        scene(),
        read::<f64>(CLOCK),
        if template != 0 {
            read::<u64>(template + crate::native::OFF_CODE)
        } else {
            0
        },
        caller.wrapping_sub(IMAGE_BASE),
    );
}

/// Called at the start of `FUN_14006bf60` with its two arguments.
unsafe extern "C" fn on_creature(creature: *const u8, removing: u8) {
    let _hook = diag::scope("on_creature");
    if creature.is_null() {
        return;
    }
    let base = creature as usize;
    let sig = (
        base,
        removing,
        read::<f32>(base + OFF_SIDE).to_bits(),
        read::<f32>(base + OFF_AGGRESSION).to_bits(),
        count(CREATURES_BEGIN, CREATURES_END, 0x18),
    );
    if let Ok(mut last) = LAST_SIG.lock() {
        if (last.0, last.1, last.2, last.3, last.4) == sig {
            return; // the same creature added again with nothing changed: not worth a line
        }
        *last = sig;
    }
    let Some(level) = next_line() else { return };
    let controlled: usize = read(CONTROLLED_LINK + 8);
    log::info!(
        "ALLY {} level={} {} t={:.2} c={:#x} side={} ai_flag={} aggr={} state={} mode={} order={} target={:#x} controlled={} \
         | creatures={} ai={} neg={} zero={} pos={}",
        if removing == 0 { "ADD" } else { "REMOVE" },
        level,
        scene(),
        read::<f64>(CLOCK),
        base,
        read::<f32>(base + OFF_SIDE),
        read::<u8>(base + OFF_AI_FLAG),
        read::<f32>(base + OFF_AGGRESSION),
        read::<i32>(base + OFF_STATE),
        read::<i32>(base + OFF_MODE),
        read::<i32>(base + OFF_ORDER),
        read::<usize>(base + OFF_TARGET),
        controlled == base,
        count(CREATURES_BEGIN, CREATURES_END, 0x18),
        count(AI_BEGIN, AI_END, 0x18),
        count(NEG_BEGIN, NEG_END, 8),
        count(ZERO_BEGIN, ZERO_END, 8),
        count(POS_BEGIN, POS_END, 8),
    );
}
