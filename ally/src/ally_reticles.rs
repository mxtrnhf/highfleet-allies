//! An ally-coloured target box over every enemy an ally is shooting at.
//!
//! The player's yellow square is the `sign_aim` sprite (global `0x14800eab0`, a 62 x 62 white frame with
//! hotspot 31,31). `afterBattleStars??` `0x14029c5c0` draws it around `playerShip+0xb00` (the target
//! node) at that node's world point (`+0x1840/44` plus `+0x240/244`), projected by `0x1401adfc0` with
//! the battle-camera globals, and tinted lime. `combatEvents` calls that function once per battle frame
//! at `0x14029a8c1`.
//!
//! This module wraps that call: after the game's own boxes, it repeats the same projection and draw for
//! the target (`ally+0xb00`) of every living ally, tinted with the configured ally colour. The ally boxes
//! are drawn at 80% of the native 62 x 62 pixels, so several of them stay readable next to the player's.

use std::sync::atomic::{AtomicU32, Ordering};

use patchy::{PatchSession, ReturnType, Trampoline};

/// `afterBattleStars??(player_ship)` draws the player's target/spawn boxes.
const TARGET_BOX_PASS: usize = 0x1_4029_c5c0;
const CALL_SITE: usize = 0x1_4029_a8c1;
const CALL_SITE_BYTES: [u8; 5] = [0xe8, 0xfa, 0x1c, 0x00, 0x00];
const CALL_RESUME: usize = 0x1_4029_a8c6;
/// The white 62 x 62 target frame (hotspot 31,31), created at setup.
const SIGN_AIM: usize = 0x1_4800_eab0;
/// World point -> screen: `(float* point, bool absolute, float scale)`, then floor the result.
const WORLD_TO_SCREEN: usize = 0x1_401a_dfc0;
const FLOOR_POINT: usize = 0x1_402e_51c0;
/// Sprite helpers: colour, scaled draw.
const SET_SPRITE_COLOR: usize = 0x1_4017_2ae0;
const RENDER_EX: usize = 0x1_4017_2aa0;
const SPRITE_BLEND: usize = 0x168;
const SPRITE_SCALE: usize = 0x16c;
/// Battle HUD enable byte; the box pass runs on the combat screen only.
const HUD_ENABLED: usize = 0x1_43a1_1961;
/// World offset the game adds to the target's position for the box centre.
const TARGET_POS_X: usize = 0x1840;
const TARGET_POS_Y: usize = 0x1844;
const TARGET_OFFSET_X: usize = 0x240;
const TARGET_OFFSET_Y: usize = 0x244;
/// Extent multiplier for the box draw: 0.8 of the native 62 x 62 frame (about 50 x 50 px).
const BOX_SIZE_SCALE: f32 = 0.8;
/// Projection zoom handed to the world-to-screen call; the native box pass uses 1.0.
const BOX_PROJECTION_ZOOM: f32 = 1.0;

type WorldToScreenFn = unsafe extern "win64" fn(*mut f32, u8, f32);
type FloorPointFn = unsafe extern "win64" fn(*mut f32);
type SetColorFn = unsafe extern "win64" fn(usize, u32, i32);
type RenderExFn = unsafe extern "win64" fn(usize, f32, f32, f32, f32, f32);

static ALLY_COLOR: AtomicU32 = AtomicU32::new(0xff00_e4ff);

unsafe fn rd<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

unsafe fn draw_box(out: &mut [f32; 2], sprite: usize, color: u32) {
    let to_screen: WorldToScreenFn = std::mem::transmute(WORLD_TO_SCREEN);
    let floor_point: FloorPointFn = std::mem::transmute(FLOOR_POINT);
    to_screen(out.as_mut_ptr(), 0, BOX_PROJECTION_ZOOM);
    floor_point(out.as_mut_ptr());

    let set_color: SetColorFn = std::mem::transmute(SET_SPRITE_COLOR);
    set_color(sprite, color, -1);
    // native box draw keeps the sprite scale at 1.0; the size comes from the RenderEx extents
    (sprite as *mut f32).byte_add(SPRITE_SCALE).write_unaligned(1.0);
    (sprite as *mut u32).byte_add(SPRITE_BLEND).write_unaligned(0);

    let render: RenderExFn = std::mem::transmute(RENDER_EX);
    // native call: rot 0, scaleY 0 = same as scaleX
    render(sprite, out[0], out[1], 0.0, BOX_SIZE_SCALE, 0.0);
}

/// Runs after the game's own target and spawn boxes.
unsafe extern "C" fn draw_ally_targets() {
    if !crate::SPAWN_ALLY.load(Ordering::Relaxed) || rd::<u8>(HUD_ENABLED) == 0 {
        return;
    }
    let sprite = rd::<usize>(SIGN_AIM);
    if sprite == 0 {
        return;
    }
    let color = ALLY_COLOR.load(Ordering::Relaxed) | 0xff00_0000;
    let Ok(allies) = crate::ALLIES.lock() else { return };

    for &ally in allies.iter() {
        if !crate::live_ship(ally)
            || rd::<f32>(ally + crate::OFF_SIDE) <= 0.0
            || rd::<i32>(ally + crate::OFF_MODE_D70) == 1
        {
            continue;
        }
        let target = rd::<usize>(ally + crate::OFF_TARGET);
        // the cached target is a creature, not a part: it must have the creature class bits
        if !crate::plausible(target) || rd::<u8>(target + 0x28) & 0x2f != 0x2f {
            continue;
        }
        // only mark foes; side > 0 would be a friendly ship
        if rd::<f32>(target + crate::OFF_SIDE) >= 0.0 {
            continue;
        }
        let mut point = [
            rd::<f32>(target + TARGET_POS_X) + rd::<f32>(target + TARGET_OFFSET_X),
            rd::<f32>(target + TARGET_POS_Y) + rd::<f32>(target + TARGET_OFFSET_Y),
        ];
        draw_box(&mut point, sprite, color);
    }
}

pub unsafe fn install(session: &mut PatchSession, argb: u32) {
    ALLY_COLOR.store(argb & 0x00ff_ffff, Ordering::Relaxed);

    let found = std::slice::from_raw_parts(CALL_SITE as *const u8, CALL_SITE_BYTES.len());
    if found != CALL_SITE_BYTES {
        log::warn!("hf_ally: ally target boxes not installed, bytes at {CALL_SITE:#x} differ ({found:02x?})");
        return;
    }

    // call the game's target/spawn box pass first, then add one box per ally target
    let mut trampoline = Trampoline::new();
    trampoline.absolute_call(TARGET_BOX_PASS as *const ());
    trampoline.preserved_call(draw_ally_targets as *const (), &[], &[], ReturnType::None);
    trampoline.relative_jump(CALL_RESUME);
    if let Err(error) = session.detour_trampoline(CALL_SITE, CALL_SITE_BYTES.len(), trampoline) {
        log::warn!("hf_ally: ally target boxes failed: {error}");
        return;
    }
    log::info!("hf_ally: ally target boxes installed in {:#010x}", argb | 0xff00_0000);
}
