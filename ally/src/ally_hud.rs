//! Damage popups of allied ships in the ally colour (`ally_damage_color`, HEX in the config).
//!
//! The yellow-green popup that appears beside a ship when it is hit ("296 ХП", the module lines under it) is drawn every
//! frame by `displayAmountOfHPDMG?` `0x1402a19c0` (notes `findings/battle-hud-callouts.md` section 6). R13 holds the popup
//! entry for the HP line; in the module-line loop R13 is the loop index and R12 = entry + 0x20. `entry+8` is the ship
//! (Creature) it belongs to. Two colour sites are replaced:
//! * `0x1402a2454` `mov edx,0xffa8fc00` (the HP line), and
//! * `0x1402a2578` `mov edx,ecx` + `mov rcx,r15` (each module line; ECX = the line's own colour with the pulsing alpha).
//! A ship on the player's side (side float `+0xad4` > 0) that the player does not fly (`+0xd70 != 1`) gets the ally
//! colour, the module lines keep their alpha.

use std::sync::atomic::{AtomicU32, Ordering};

use patchy::{PatchSession, ReturnType, Trampoline};

const HP_SITE: usize = 0x1_402a_2454;
const HP_BYTES: [u8; 5] = [0xba, 0x00, 0xfc, 0xa8, 0xff];
const LINE_SITE: usize = 0x1_402a_2578;
const LINE_BYTES: [u8; 5] = [0x8b, 0xd1, 0x49, 0x8b, 0xcf];
const LIME: u32 = 0xffa8_fc00;

static BLUE: AtomicU32 = AtomicU32::new(0xff00_e4ff);

unsafe fn rd<T: Copy>(a: usize) -> T {
    (a as *const T).read_unaligned()
}

/// True when `ship` is a creature (ship) on the player's side that the player is not flying.
unsafe fn friendly_ship(ship: usize) -> bool {
    if !crate::plausible(ship) || rd::<u8>(ship + 0x28) & 0x2f != 0x2f {
        return false; // shells, missiles and drones keep their colour: +0xad4 is not a side there
    }
    rd::<f32>(ship + 0xad4) > 0.0 && rd::<i32>(ship + 0xd70) != 1
}

/// True when the popup `entry` belongs to a friendly ship the player is not flying.
unsafe fn friendly(entry: usize) -> bool {
    crate::plausible(entry) && friendly_ship(rd(entry + 8))
}

unsafe extern "C" fn hp_color(entry: usize) -> u32 {
    if friendly(entry) { BLUE.load(Ordering::Relaxed) } else { LIME }
}

unsafe extern "C" fn line_color(entry: usize, argb: u32) -> u32 {
    if friendly(entry) { (argb & 0xff00_0000) | (BLUE.load(Ordering::Relaxed) & 0x00ff_ffff) } else { argb }
}

/// The popup leader line: sprite `DAT_14800eb70`, one global sprite whose colour persists, so it is set every time it is
/// drawn (`TadsDamage_DrawPopups`, the call at `0x1402a1cb9`, R13 = the entry).
const LEADER_SPRITE: usize = 0x1_4800_eb70;
const LEADER_CALL: usize = 0x1_402a_1cb9;
const LEADER_CALL_BYTES: [u8; 5] = [0xe8, 0xe2, 0x0d, 0xed, 0xff];
const LEADER_DRAW: usize = 0x1_4017_2aa0;
const SET_SPRITE_COLOR: usize = 0x1_4017_2ae0;

unsafe extern "C" fn leader_color(entry: usize) {
    let sprite: usize = rd(LEADER_SPRITE);
    if !crate::plausible(sprite) {
        return;
    }
    let argb = if friendly(entry) { BLUE.load(Ordering::Relaxed) } else { LIME };
    let set: unsafe extern "win64" fn(usize, u32, i32) = std::mem::transmute(SET_SPRITE_COLOR);
    set(sprite, argb, -1);
}

/// The star (`sign_friend`, a white texture, drawn beside every ship with side > 0 by the combat HUD pass, RSI = the
/// ship): `0x14029e008` sets the lime tint right before the draw (notes `battle-hud-callouts.md` section 8).
const STAR_SITE: usize = 0x1_4029_e008;
const STAR_BYTES: [u8; 11] = [0xba, 0x00, 0xfc, 0xa8, 0xff, 0x41, 0xb8, 0xff, 0xff, 0xff, 0xff];
const STAR_SPRITE: usize = 0x1_47fc_6a18;
const STAR_RESUME: usize = 0x1_4029_e013;

unsafe extern "C" fn star_color(ship: usize) -> u32 {
    if friendly_ship(ship) { BLUE.load(Ordering::Relaxed) } else { LIME }
}

pub unsafe fn install(session: &mut PatchSession, argb: u32) {
    BLUE.store(argb | 0xff00_0000, Ordering::Relaxed);
    let check = |addr: usize, want: &[u8]| std::slice::from_raw_parts(addr as *const u8, want.len()) == want;
    if !check(HP_SITE, &HP_BYTES) || !check(LINE_SITE, &LINE_BYTES) {
        log::warn!("hf_ally: ally damage colour not applied, bytes at {HP_SITE:#x} / {LINE_SITE:#x} differ");
        return;
    }
    // HP line: edx = colour for the entry in R13 (RSP is 0 mod 16 here, right before the call that follows)
    let mut t = Trampoline::new();
    t.preserved_call(hp_color as *const (), &[0x4c, 0x89, 0xe9], &[], ReturnType::Rax); // mov rcx,r13
    t.bytes(&[0x89, 0xc2]); // mov edx,eax
    t.relative_jump(HP_SITE + HP_BYTES.len());
    if let Err(e) = session.detour_trampoline(HP_SITE, HP_BYTES.len(), t) {
        log::warn!("hf_ally: ally damage colour (HP line) failed: {e}");
        return;
    }
    // module lines: ecx = the line's colour; edx = recoloured, then the original `mov rcx,r15`
    let mut t = Trampoline::new();
    t.preserved_call(line_color as *const (), &[0x89, 0xca, 0x49, 0x8d, 0x4c, 0x24, 0xe0], &[], ReturnType::Rax); // mov edx,ecx; lea rcx,[r12-0x20]
    t.bytes(&[0x89, 0xc2]); // mov edx,eax
    t.bytes(&[0x49, 0x8b, 0xcf]); // mov rcx,r15
    t.relative_jump(LINE_SITE + LINE_BYTES.len());
    if let Err(e) = session.detour_trampoline(LINE_SITE, LINE_BYTES.len(), t) {
        log::warn!("hf_ally: ally damage colour (module lines) failed: {e}");
        return;
    }
    // leader line: set the shared sprite's colour for this entry, then the original draw call
    if check(LEADER_CALL, &LEADER_CALL_BYTES) {
        let mut t = Trampoline::new();
        t.preserved_call(leader_color as *const (), &[0x4c, 0x89, 0xe9], &[], ReturnType::None); // mov rcx,r13
        t.absolute_call(LEADER_DRAW as *const ());
        t.relative_jump(LEADER_CALL + LEADER_CALL_BYTES.len());
        if let Err(e) = session.detour_trampoline(LEADER_CALL, LEADER_CALL_BYTES.len(), t) {
            log::warn!("hf_ally: ally leader line colour failed: {e}");
        }
    } else {
        log::warn!("hf_ally: leader line call at {LEADER_CALL:#x} differs, not recoloured");
    }
    // the star: edx = the colour for the ship in RSI, r8d = -1, rax = the sprite again (the call clobbers it)
    if check(STAR_SITE, &STAR_BYTES) {
        let mut t = Trampoline::new();
        t.preserved_call(star_color as *const (), &[0x48, 0x89, 0xf1], &[], ReturnType::Rax); // mov rcx,rsi
        t.bytes(&[0x89, 0xc2]); // mov edx,eax
        t.bytes(&[0x41, 0xb8, 0xff, 0xff, 0xff, 0xff]); // mov r8d,-1
        t.bytes(&[0x48, 0xb8]); // movabs rax, STAR_SPRITE
        t.bytes(&(STAR_SPRITE as u64).to_le_bytes());
        t.bytes(&[0x48, 0x8b, 0x00]); // mov rax,[rax]
        t.relative_jump(STAR_RESUME);
        if let Err(e) = session.detour_trampoline(STAR_SITE, STAR_BYTES.len(), t) {
            log::warn!("hf_ally: ally star colour failed: {e}");
        }
    } else {
        log::warn!("hf_ally: star colour site {STAR_SITE:#x} differs, not recoloured");
    }
    log::info!("hf_ally: damage popups of allied ships drawn in {:#010x}", argb | 0xff00_0000);
}
