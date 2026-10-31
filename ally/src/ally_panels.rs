//! Small ship panels for the allies, next to the player's own panel.
//!
//! The battle HUD draws one 350 x 300 panel per call of `DrawBattleShipPanel` `0x1402fde80` (notes
//! `rendering/battle-ship-panels.md`): mode 2 is the player's ship, mode 3 is `player+0xb00` (the target link).
//! Two hooks run the feature:
//! * the own-panel call `0x140309ab4` renders each live ally into two private 350 x 300 render targets
//!   (target link pointed at the ally, mode 3, restored immediately), and
//! * the panel-composite flush `0x14030915f` draws those targets at half width and half height (a quarter of
//!   the area) beside the player's panel, where the renderer is already set up for the stock panel sprites.
//!
//! Rendering one frame before compositing matches the stock panels, which also show the previous frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use patchy::{PatchSession, ReturnType, Trampoline};

/// `DrawBattleShipPanel(surface_a, surface_b, mode)`.
const PANEL: usize = 0x1_402f_de80;
const PANEL_MODE_TARGET: u32 = 3;
/// Own-panel call in the battle renderer: `E8 C7 43 FF FF`.
const RENDER_SITE: usize = 0x1_4030_9ab4;
const RENDER_SITE_BYTES: [u8; 5] = [0xe8, 0xc7, 0x43, 0xff, 0xff];
const RENDER_RESUME: usize = 0x1_4030_9ab9;
/// Panel-composite flush: the stock left/right panel sprites have just been drawn.
const COMPOSITE_SITE: usize = 0x1_4030_915f;
const COMPOSITE_SITE_BYTES: [u8; 5] = [0xe8, 0xec, 0xa9, 0xd7, 0xff];
const COMPOSITE_RESUME: usize = 0x1_4030_9164;
/// HGE helpers: draw a sprite scaled, set its colour and its z.
const RENDER_EX: usize = 0x1_4017_2aa0;
const SET_SPRITE_COLOR: usize = 0x1_4017_2ae0;
const SET_SPRITE_Z: usize = 0x1_4017_2ee0;
const LAYER_Z: usize = 0x1_4021_6480;
/// HGE system object: slot `+0x300` creates the colour target, `+0x308` its depth/stencil surface.
const ENGINE: usize = 0x1_43a1_19e0;
const CREATE_TEXTURE_SLOT: usize = 0x300;
const CREATE_DEPTH_SLOT: usize = 0x308;
const TEXTURE_FORMAT: u32 = 0x1c;
const RENDER_TARGET: u32 = 1;
const OPERATOR_NEW: usize = 0x1_4033_4094;
const SPRITE_CTOR: usize = 0x1_4017_1d50;
const SPRITE_SIZE: u32 = 0x1a8;
/// `hgeSprite` blend mode (`+0x168`); the game uses `0x10` for the panels.
const SPRITE_BLEND: usize = 0x168;
const BLEND_PANEL: u32 = 0x10;
/// Battle HUD enable byte and the panel pass' own suppression flag.
const HUD_ENABLED: usize = 0x1_43a1_1961;
const PANEL_SUPPRESSED: usize = 0x1_43a1_5908;
/// Virtual frame height (1920 x 1080 target).
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
/// Player profile and its creature lookup used by mode 3.
const GET_PROFILE: usize = 0x1_4029_2a90;
const GET_PLAYER_CREATURE: usize = 0x1_4029_24b0;

const PANEL_WIDTH: f32 = 350.0;
const PANEL_HEIGHT: f32 = 300.0;
/// Half width and half height = a quarter of the panel area.
const ALLY_SCALE: f32 = 0.5;
/// Panels sit immediately right of the player's own panel, in a column from `H - 300`.
const ALLY_X: f32 = PANEL_WIDTH;
const ALLY_SLOT_STEP: f32 = PANEL_HEIGHT * ALLY_SCALE; // 150
/// The player's own panel is 350 x 300, so two half-size panels fill the same height.
const MAX_PANELS: usize = 2;

type PanelFn = unsafe extern "win64" fn(usize, usize, u32);
type RenderExFn = unsafe extern "win64" fn(usize, f32, f32, f32, f32, f32);
type SetColorFn = unsafe extern "win64" fn(usize, u32, i32);
type SetZFn = unsafe extern "win64" fn(usize, f32, i32);
type LayerZFn = unsafe extern "win64" fn(u32) -> f32;
type CreateTextureFn = unsafe extern "win64" fn(usize, u32, u32, u32, u32) -> usize;
type CreateDepthFn = unsafe extern "win64" fn(usize, u32, u32) -> usize;
type AllocFn = unsafe extern "win64" fn(u32) -> usize;
type SpriteCtorFn =
    unsafe extern "win64" fn(usize, usize, f32, f32, f32, f32, f32, f32, f32, f32);

#[derive(Clone, Copy)]
struct NativePanel {
    surface_a: usize,
    surface_b: usize,
    sprite: usize,
}

static PANELS: Mutex<[Option<NativePanel>; MAX_PANELS]> = Mutex::new([None, None]);
static PANEL_CREATE_FAILED: AtomicBool = AtomicBool::new(false);
static FIRST_RENDER: AtomicBool = AtomicBool::new(true);

unsafe fn rd<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

unsafe fn write(address: usize, value: usize) {
    (address as *mut usize).write_unaligned(value);
}

/// Up to [`MAX_PANELS`] living allies the mod deployed, in deployment order.
unsafe fn living_allies() -> [usize; MAX_PANELS] {
    let mut slots = [0usize; MAX_PANELS];
    let Ok(allies) = crate::ALLIES.lock() else { return slots };
    let mut used = 0;
    for &ally in allies.iter() {
        if used == MAX_PANELS {
            break;
        }
        // side > 0 keeps this to the player's own ships; mode 1 is the ship the player flies
        if !crate::live_ship(ally)
            || rd::<f32>(ally + crate::OFF_SIDE) <= 0.0
            || rd::<i32>(ally + crate::OFF_MODE_D70) == 1
        {
            continue;
        }
        slots[used] = ally;
        used += 1;
    }
    slots
}

unsafe fn player_creature() -> Option<usize> {
    let get_profile: GetProfile = std::mem::transmute(GET_PROFILE);
    let get_player: GetPlayer = std::mem::transmute(GET_PLAYER_CREATURE);
    let profile = get_profile();
    if !crate::plausible(profile) {
        return None;
    }
    let player = get_player(profile, 0);
    crate::live_ship(player).then_some(player)
}

type GetProfile = unsafe extern "win64" fn() -> usize;
type GetPlayer = unsafe extern "win64" fn(usize, u32) -> usize;

/// Create the two private 350 x 300 targets and their sprites once, while an ally is first drawn.
unsafe fn ensure_panels() -> bool {
    if PANEL_CREATE_FAILED.load(Ordering::Relaxed) {
        return false;
    }
    let Ok(mut panels) = PANELS.lock() else { return false };
    if panels[0].is_some() {
        return true;
    }

    let engine = rd::<usize>(ENGINE);
    if engine == 0 {
        return false;
    }
    let vtable = rd::<usize>(engine);
    let create: CreateTextureFn = std::mem::transmute(rd::<usize>(vtable + CREATE_TEXTURE_SLOT));
    let create_depth: CreateDepthFn = std::mem::transmute(rd::<usize>(vtable + CREATE_DEPTH_SLOT));
    let alloc: AllocFn = std::mem::transmute(OPERATOR_NEW);
    let ctor: SpriteCtorFn = std::mem::transmute(SPRITE_CTOR);

    for slot in 0..MAX_PANELS {
        let surface_a = create(engine, 350, 300, TEXTURE_FORMAT, RENDER_TARGET);
        let surface_b = create_depth(engine, 350, 300);
        let object = alloc(SPRITE_SIZE);
        if surface_a == 0 || surface_b == 0 || object == 0 {
            PANEL_CREATE_FAILED.store(true, Ordering::Relaxed);
            log::warn!(
                "hf_ally: ally ship panels not created: texture {surface_a:#x}/{surface_b:#x}, sprite {object:#x}"
            );
            return false;
        }
        ctor(object, surface_a, 0.0, 0.0, 350.0, 300.0, 0.0, 0.0, 0.0, 0.0);
        panels[slot] = Some(NativePanel { surface_a, surface_b, sprite: object });
    }
    log::info!("hf_ally: ally ship panel targets created");
    true
}

/// Render each live ally into its private target for the next frame's composite.
unsafe extern "C" fn render_allies() {
    if !crate::SPAWN_ALLY.load(Ordering::Relaxed) || rd::<usize>(PANEL_SUPPRESSED) != 0 {
        return;
    }
    let allies = living_allies();
    if allies[0] == 0 {
        return;
    }
    let Some(player) = player_creature() else { return };
    if !ensure_panels() {
        return;
    }
    let Ok(panels) = PANELS.lock() else { return };

    let panel: PanelFn = std::mem::transmute(PANEL);
    for (slot, &ally) in allies.iter().take(MAX_PANELS).enumerate() {
        if ally == 0 {
            break;
        }
        let Some(native) = panels[slot] else { continue };
        let saved_target = rd::<usize>(player + crate::OFF_TARGET);
        write(player + crate::OFF_TARGET, ally);
        panel(native.surface_a, native.surface_b, PANEL_MODE_TARGET);
        // the AI reads the target link during the frame: put it back before anything else runs
        write(player + crate::OFF_TARGET, saved_target);
    }

    if FIRST_RENDER.swap(false, Ordering::Relaxed) {
        log::info!(
            "hf_ally: ally ship panels drawing: player c={player:#x}, allies c={:#x}/{:#x}",
            allies[0],
            allies[1]
        );
    }
}

/// Composite the targets prepared last frame, where the stock panel sprites are drawn.
unsafe extern "C" fn composite_allies() {
    if !crate::SPAWN_ALLY.load(Ordering::Relaxed)
        || rd::<u8>(HUD_ENABLED) == 0
        || rd::<usize>(PANEL_SUPPRESSED) != 0
    {
        return;
    }
    let allies = living_allies();
    if allies[0] == 0 {
        return;
    }
    let Ok(panels) = PANELS.lock() else { return };

    let frame_height = rd::<i32>(FRAME_HEIGHT);
    let frame_height = if frame_height > 0 { frame_height as f32 } else { 1080.0 };
    let layer_z: LayerZFn = std::mem::transmute(LAYER_Z);
    let set_z: SetZFn = std::mem::transmute(SET_SPRITE_Z);
    let set_color: SetColorFn = std::mem::transmute(SET_SPRITE_COLOR);
    let render: RenderExFn = std::mem::transmute(RENDER_EX);

    for (slot, &ally) in allies.iter().take(MAX_PANELS).enumerate() {
        if ally == 0 {
            break;
        }
        let Some(native) = panels[slot] else { continue };
        let sprite = native.sprite;
        let y = frame_height - PANEL_HEIGHT + slot as f32 * ALLY_SLOT_STEP;
        set_color(sprite, 0xffff_ffff, -1);
        set_z(sprite, layer_z(3), -1);
        (sprite as *mut u32).byte_add(SPRITE_BLEND).write_unaligned(BLEND_PANEL);
        render(sprite, ALLY_X, y, 0.0, ALLY_SCALE, ALLY_SCALE);
    }
}

pub unsafe fn install(session: &mut PatchSession) {
    let check = |address: usize, bytes: &[u8]| {
        std::slice::from_raw_parts(address as *const u8, bytes.len()) == bytes
    };
    if !check(RENDER_SITE, &RENDER_SITE_BYTES) || !check(COMPOSITE_SITE, &COMPOSITE_SITE_BYTES) {
        log::warn!(
            "hf_ally: ally ship panels not installed, bytes at {RENDER_SITE:#x}/{COMPOSITE_SITE:#x} differ"
        );
        return;
    }

    // own panel call: render our targets first, then the game's own panel
    let mut render_trampoline = Trampoline::new();
    render_trampoline.preserved_call(render_allies as *const (), &[], &[], ReturnType::None);
    render_trampoline.absolute_call(PANEL as *const ());
    render_trampoline.relative_jump(RENDER_RESUME);
    if let Err(error) = session.detour_trampoline(RENDER_SITE, RENDER_SITE_BYTES.len(), render_trampoline) {
        log::warn!("hf_ally: ally ship panel render hook failed: {error}");
        return;
    }

    // panel-composite flush: draw our scaled sprites into the already prepared HUD frame
    let mut composite_trampoline = Trampoline::new();
    composite_trampoline.preserved_call(composite_allies as *const (), &[], &[], ReturnType::None);
    composite_trampoline.absolute_call(0x1_4008_3b50 as *const ()); // the original flush
    composite_trampoline.relative_jump(COMPOSITE_RESUME);
    if let Err(error) = session.detour_trampoline(
        COMPOSITE_SITE,
        COMPOSITE_SITE_BYTES.len(),
        composite_trampoline,
    ) {
        log::warn!("hf_ally: ally ship panel composite hook failed: {error}");
        return;
    }

    log::info!("hf_ally: ally ship panels installed (half-size, right of the own panel)");
}
