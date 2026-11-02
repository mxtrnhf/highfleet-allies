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

use std::mem::offset_of;
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
/// The stock target panel is skipped while a message/panel is on screen (the end-game gate byte).
const TARGET_PANEL_HIDDEN: usize = 0x1_47fc_6fca;
/// Virtual frame height (1920 x 1080 target).
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
/// Player profile and its creature lookup used by mode 3.
const GET_PROFILE: usize = 0x1_4029_2a90;
const GET_PLAYER_CREATURE: usize = 0x1_4029_24b0;

/// `DrawBattleShipPanel` scans HUD marker layers 1..39 by way of the counts at `0x143a51160`
/// and the layer-1 owner field at `0x144094c88` (exactly `HUD_MARKERS + 40000 * 0x18 + 8`).
/// Ships spawned through the normal battle path have type-2 part markers there; the temporary
/// ally deployment path does not, so the panel would show only its text. The native marker
/// initialisation below is the fix; the fields after it describe the last-resort fallback that
/// builds markers for one call and removes them immediately afterward.
const HUD_MARKERS: usize = 0x1_43fa_a680;
const HUD_MARKER_COUNTS: usize = 0x1_43a5_1160;
const HUD_MARKER_LIMIT: usize = 40_000;
/// `Body::UpdateHudMarkers` `0x14002d7b0(body, initialise)`. The stock per-frame pass calls this
/// for every world body with `initialise = 0`, but it leaves bodies alone whose `+0x910` byte is
/// still clear. That byte is set only by the one-time `initialise = 1` call the normal spawn path
/// performs; the mod's temporary ally deploy skipped it, so none of the ally's parts ever entered
/// the HUD marker table. Calling it once with `initialise = 1` registers the whole child subtree
/// exactly like a stock ship.
const BODY_UPDATE_HUD_MARKERS: usize = 0x1_4002_d7b0;
type BodyHudMarkersFn = unsafe extern "C" fn(usize, u8);
const HUD_MARKER_STRIDE: usize = 0x18;
const HUD_MARKER_TYPE_CHILD: i32 = 2;
const HUD_MARKER_OWNER: usize = 8;
const HUD_MARKER_INDEX: usize = 0x10;
const HUD_MARKER_SCALE: usize = 0x14;
/// The panel applies a brightness lift of `layer / 40`; use the first scanned layer so temporary
/// markers keep the same dark palette as normal parts instead of becoming washed-out white.
const SPARE_LAYER: usize = 1;
const PANEL_LAYER_LAST: usize = 39;
/// Creature part list (`+0x1330/+0x1338`, 0x18-byte Link entries, part at +8).
const OFF_PART_LIST: (usize, usize) = (0x1330, 0x1338);
/// Part `Body` sprite vector (`CVec<*mut Sprite>`).
const OFF_PART_SPRITES: (usize, usize) = (0x8c0, 0x8c8);
/// Part `Body` owner link target: the ship a part belongs to (`Node.owner + Link.target`).
const OFF_PART_OWNER: usize = 0xb8;
/// Registration skips sprites whose `+0x128` float is zero.
const OFF_SPRITE_VISIBLE: usize = 0x128;
/// Node state; `Hud_DrawChildMarker` draws only parts with state > 1.
const OFF_NODE_STATE: usize = 0x48;
/// Part health and maximum; the panel's palette darkens towards red as `hp / hp_max` drops.
const OFF_PART_HP: usize = 0x2c8;
const OFF_PART_HP_MAX: usize = 0x2cc;
const PART_LIST_LIMIT: usize = 512;
const PART_SPRITE_LIMIT: usize = 64;
/// Main battle render body grid (`DAT_146449688`, 60 layers x 40000 entries, stride 0x18) and
/// its counts at `DAT_143a51200`. This is the same source the enemy target/module view uses, so
/// it includes hull bodies that are not in the creature's mounted-part vector.
const RENDER_BODY_GRID: usize = 0x1_4644_9688;
const RENDER_BODY_COUNTS: usize = 0x1_43a5_1200;
const RENDER_BODY_LAYERS: usize = 0x3c;
const RENDER_BODY_LIMIT: usize = 40_000;
const RENDER_BODY_STRIDE: usize = 0x18;

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
    rendered_ship: usize,
}

static PANELS: Mutex<[Option<NativePanel>; MAX_PANELS]> = Mutex::new([None, None]);
static DISPLAYED_SHIPS: Mutex<[usize; MAX_PANELS]> = Mutex::new([0; MAX_PANELS]);
static PANEL_CREATE_FAILED: AtomicBool = AtomicBool::new(false);
static FIRST_RENDER: AtomicBool = AtomicBool::new(true);
static PANEL_MARKERS_LOGGED: AtomicBool = AtomicBool::new(false);

unsafe fn rd<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

/// Up to [`MAX_PANELS`] live allies the mod deployed, in deployment order.
unsafe fn living_allies() -> [usize; MAX_PANELS] {
    let mut slots = [0usize; MAX_PANELS];
    let roster = crate::roster();
    let Ok(allies) = crate::ALLIES.lock() else { return slots };
    let mut used = 0;
    for &ally in allies.iter() {
        if used == MAX_PANELS {
            break;
        }
        // side > 0 keeps this to the player's own ships; mode 1 is the ship the player flies.
        // State 2 is the AI strike/retreat state; its ship is leaving and must not keep a panel.
        if !roster.contains(&ally)
            || !crate::live_ship(ally)
            || rd::<f32>(ally + crate::OFF_SIDE) <= 0.0
            || rd::<i32>(ally + crate::OFF_MODE_D70) == 1
            || !matches!(rd::<i32>(ally + crate::OFF_STATE), 0 | 1)
        {
            continue;
        }
        slots[used] = ally;
        used += 1;
    }
    slots
}

/// Identity of the ship actually composited into this slot, not the next frame's roster selection.
pub fn displayed_ship(slot: usize) -> Option<usize> {
    let displayed = DISPLAYED_SHIPS.lock().ok()?;
    displayed.get(slot).copied().filter(|ship| *ship != 0)
}

/// Clear only the panel content. Keep `DISPLAYED_SHIPS` (the last composited frame) for key selection.
fn clear_rendered() {
    if let Ok(mut panels) = PANELS.lock() {
        for panel in panels.iter_mut().flatten() {
            panel.rendered_ship = 0;
        }
    }
}

/// Do not associate an old target texture with a new ship after a handoff or battle teardown.
pub fn invalidate() {
    if let Ok(mut displayed) = DISPLAYED_SHIPS.lock() {
        *displayed = [0; MAX_PANELS];
    }
    clear_rendered();
    PANEL_MARKERS_LOGGED.store(false, Ordering::Relaxed);
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
        panels[slot] = Some(NativePanel { surface_a, surface_b, sprite: object, rendered_ship: 0 });
    }
    log::info!("hf_ally: ally ship panel targets created");
    true
}

/// Temporary part markers appended to the spare layer for one panel call.
struct TemporaryMarkers {
    original_count: usize,
    added: usize,
}

/// Type-2 markers the native panel can already see for this ally. Do not add duplicates for these:
/// the panel draws with blend mode 6, so drawing the same sprite twice lightens it ("milky" panels).
unsafe fn existing_panel_markers(ally: usize) -> Vec<(usize, usize)> {
    let mut existing = Vec::new();
    for layer in 1..=PANEL_LAYER_LAST {
        let count = rd::<i32>(HUD_MARKER_COUNTS + layer * 4);
        if count < 0 || count as usize > HUD_MARKER_LIMIT {
            continue;
        }
        let base = HUD_MARKERS + layer * HUD_MARKER_LIMIT * HUD_MARKER_STRIDE;
        for index in 0..count as usize {
            let marker = base + index * HUD_MARKER_STRIDE;
            if rd::<i32>(marker) != HUD_MARKER_TYPE_CHILD {
                continue;
            }
            let part = rd::<usize>(marker + HUD_MARKER_OWNER);
            if !crate::plausible(part) {
                continue;
            }
            let Some(part_owner) = crate::diag::rd::<usize>(part + OFF_PART_OWNER) else {
                continue;
            };
            if part_owner != ally {
                continue;
            }
            let part_index = rd::<i32>(marker + HUD_MARKER_INDEX);
            if part_index >= 0 {
                existing.push((part, part_index as usize));
            }
        }
    }
    existing
}

/// All part bodies of the ally. Prefer the main battle render grid (the source the enemy target
/// view uses); fall back to the creature's mounted-part vector if the grid has nothing.
unsafe fn part_bodies(ally: usize) -> Vec<usize> {
    let mut bodies = Vec::new();
    for layer in 0..RENDER_BODY_LAYERS {
        let count = rd::<i32>(RENDER_BODY_COUNTS + layer * 4);
        if count < 0 || count as usize > RENDER_BODY_LIMIT {
            continue;
        }
        let base = RENDER_BODY_GRID + layer * RENDER_BODY_LIMIT * RENDER_BODY_STRIDE;
        for index in 0..count as usize {
            let part = rd::<usize>(base + index * RENDER_BODY_STRIDE);
            if !crate::plausible(part) || bodies.contains(&part) {
                continue;
            }
            let Some(owner) = crate::diag::rd::<usize>(part + OFF_PART_OWNER) else {
                continue;
            };
            if owner == ally {
                bodies.push(part);
            }
        }
    }
    let (begin, end): (usize, usize) = (rd(ally + OFF_PART_LIST.0), rd(ally + OFF_PART_LIST.1));
    if begin < 0x10000 || end < begin {
        return bodies;
    }
    let bytes = end - begin;
    if !bytes.is_multiple_of(crate::LINK_STRIDE) || bytes / crate::LINK_STRIDE > PART_LIST_LIMIT {
        return bodies;
    }
    for entry in (begin..end).step_by(crate::LINK_STRIDE) {
        let part = rd::<usize>(entry + offset_of!(crate::native_layout::Link, target));
        if !crate::plausible(part) || bodies.contains(&part) {
            continue;
        }
        let Some(owner) = crate::diag::rd::<usize>(part + OFF_PART_OWNER) else {
            continue;
        };
        if owner == ally {
            bodies.push(part);
        }
    }
    bodies
}

/// Give the ally the same HUD marker initialisation the stock spawn path performs, then return the
/// type-2 markers the panel can now see. Called before every panel render because the marker table
/// is rebuilt each frame; it only does work while the ally still has no markers.
unsafe fn ensure_native_markers(ally: usize) -> Vec<(usize, usize)> {
    let mut existing = existing_panel_markers(ally);
    if existing.is_empty() {
        let update: BodyHudMarkersFn = std::mem::transmute(BODY_UPDATE_HUD_MARKERS);
        update(ally, 1);
        existing = existing_panel_markers(ally);
        if !PANEL_MARKERS_LOGGED.swap(true, Ordering::Relaxed) {
            log::info!(
                "ALLY panel c={ally:#x}: native marker initialisation produced {} part markers",
                existing.len()
            );
        }
    }
    existing
}

/// Build type-2 child markers from the ally's actual part/sprite lists. The normal ally deploy
/// path does not reliably leave these in the HUD marker table; without them the panel draws its
/// text but no ship schematic. Markers are appended to [`SPARE_LAYER`] and removed by count restore.
unsafe fn insert_part_markers(ally: usize) -> Option<TemporaryMarkers> {
    // The native registration draws the ship exactly like an enemy panel, so only fall back when
    // it produced nothing at all.
    let existing = ensure_native_markers(ally);
    if !existing.is_empty() {
        return None;
    }

    let spare_count = rd::<i32>(HUD_MARKER_COUNTS + SPARE_LAYER * 4);
    if spare_count < 0 || spare_count as usize > HUD_MARKER_LIMIT {
        return None;
    }
    let spare_count = spare_count as usize;
    let parts_list = part_bodies(ally);
    if parts_list.is_empty() {
        return None;
    }

    let mut markers = Vec::new();
    let parts = parts_list.len();
    let mut inactive = 0usize;
    let mut hp_low = f32::INFINITY;
    let mut hp_high = 0.0f32;
    for part in parts_list {
        if rd::<i32>(part + OFF_NODE_STATE) <= 1 {
            inactive += 1;
        }
        let hp = rd::<f32>(part + OFF_PART_HP);
        let hp_max = rd::<f32>(part + OFF_PART_HP_MAX);
        if hp_max > 0.0 {
            let ratio = (hp / hp_max).clamp(0.0, 1.0);
            hp_low = hp_low.min(ratio);
            hp_high = hp_high.max(ratio);
        }
        let (sprites_begin, sprites_end): (usize, usize) =
            (rd(part + OFF_PART_SPRITES.0), rd(part + OFF_PART_SPRITES.1));
        if sprites_begin < 0x10000 || sprites_end < sprites_begin {
            continue;
        }
        let sprite_bytes = sprites_end - sprites_begin;
        if !sprite_bytes.is_multiple_of(8) || sprite_bytes / 8 > PART_SPRITE_LIMIT {
            continue;
        }
        for index in 0..sprite_bytes / 8 {
            let sprite = rd::<usize>(sprites_begin + index * 8);
            if !crate::plausible(sprite) {
                continue;
            }
            let Some(visible) = crate::diag::rd::<f32>(sprite + OFF_SPRITE_VISIBLE) else {
                continue;
            };
            if visible == 0.0 {
                continue;
            }
            let mut marker = [0u8; HUD_MARKER_STRIDE];
            std::ptr::write_unaligned(marker.as_mut_ptr() as *mut i32, HUD_MARKER_TYPE_CHILD);
            std::ptr::write_unaligned(
                marker.as_mut_ptr().byte_add(HUD_MARKER_OWNER) as *mut usize,
                part,
            );
            std::ptr::write_unaligned(
                marker.as_mut_ptr().byte_add(HUD_MARKER_INDEX) as *mut i32,
                index as i32,
            );
            std::ptr::write_unaligned(
                marker.as_mut_ptr().byte_add(HUD_MARKER_SCALE) as *mut f32,
                1.0,
            );
            markers.push(marker);
        }
    }

    if markers.is_empty() {
        return None;
    }
    if spare_count + markers.len() > HUD_MARKER_LIMIT {
        if !PANEL_MARKERS_LOGGED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "ALLY panel c={ally:#x}: spare marker layer full ({spare_count} + {})",
                markers.len()
            );
        }
        return None;
    }
    let spare_base = HUD_MARKERS + SPARE_LAYER * HUD_MARKER_LIMIT * HUD_MARKER_STRIDE;
    for (offset, marker) in markers.iter().enumerate() {
        let destination = spare_base + (spare_count + offset) * HUD_MARKER_STRIDE;
        std::ptr::copy_nonoverlapping(marker.as_ptr(), destination as *mut u8, HUD_MARKER_STRIDE);
    }
    (HUD_MARKER_COUNTS as *mut i32)
        .byte_add(SPARE_LAYER * 4)
        .write_unaligned((spare_count + markers.len()) as i32);
    if !PANEL_MARKERS_LOGGED.swap(true, Ordering::Relaxed) {
        let hp_low = if hp_low.is_finite() { hp_low } else { 1.0 };
        log::info!(
            "ALLY panel c={ally:#x}: built {} fallback part markers from {parts} parts ({inactive} inactive, hp {hp_low:.2}..{hp_high:.2})",
            markers.len()
        );
    }
    Some(TemporaryMarkers {
        original_count: spare_count,
        added: markers.len(),
    })
}

/// Hide the temporary part markers again; the native panel has already consumed them.
unsafe fn restore_temporary_markers(temporary: TemporaryMarkers) {
    if temporary.added == 0 {
        return;
    }
    (HUD_MARKER_COUNTS as *mut i32)
        .byte_add(SPARE_LAYER * 4)
        .write_unaligned(temporary.original_count as i32);
}

/// Render each live ally into its private target for the next frame's composite.
unsafe extern "C" fn render_allies() {
    // A frame with no fresh panel content must not composite last frame's texture.
    clear_rendered();
    if !crate::SPAWN_ALLY.load(Ordering::Relaxed)
        || rd::<usize>(PANEL_SUPPRESSED) != 0
        || rd::<u8>(TARGET_PANEL_HIDDEN) != 0
    {
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
    let Ok(mut panels) = PANELS.lock() else { return };

    let panel: PanelFn = std::mem::transmute(PANEL);
    let set_link: crate::SetLinkFn = std::mem::transmute(crate::SET_LINK);
    let target_link = player + crate::OFF_TARGET_LINK;
    let saved_target = rd::<usize>(player + crate::OFF_TARGET);
    for native in panels.iter_mut().flatten() {
        native.rendered_ship = 0;
    }
    for (slot, &ally) in allies.iter().take(MAX_PANELS).enumerate() {
        if ally == 0 {
            break;
        }
        let Some(native) = panels[slot].as_mut() else { continue };
        // Use the native link operation: a raw +0xb00 write leaves the link registered on the old
        // target's link list and its saved id stale.
        let temporary = insert_part_markers(ally);
        set_link(target_link, ally);
        panel(native.surface_a, native.surface_b, PANEL_MODE_TARGET);
        set_link(target_link, saved_target);
        if let Some(temporary) = temporary {
            restore_temporary_markers(temporary);
        }
        native.rendered_ship = ally;
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
    // A hidden HUD must not leave keyboard-selectable panels from a previous frame.
    let Ok(mut displayed) = DISPLAYED_SHIPS.lock() else { return };
    *displayed = [0; MAX_PANELS];
    if !crate::SPAWN_ALLY.load(Ordering::Relaxed)
        || rd::<u8>(HUD_ENABLED) == 0
        || rd::<usize>(PANEL_SUPPRESSED) != 0
        || rd::<u8>(TARGET_PANEL_HIDDEN) != 0
    {
        return;
    }
    let allies = living_allies();
    let Ok(panels) = PANELS.lock() else { return };

    let frame_height = rd::<i32>(FRAME_HEIGHT);
    let frame_height = if frame_height > 0 { frame_height as f32 } else { 1080.0 };
    let layer_z: LayerZFn = std::mem::transmute(LAYER_Z);
    let set_z: SetZFn = std::mem::transmute(SET_SPRITE_Z);
    let set_color: SetColorFn = std::mem::transmute(SET_SPRITE_COLOR);
    let render: RenderExFn = std::mem::transmute(RENDER_EX);

    for slot in 0..MAX_PANELS {
        let Some(native) = panels[slot] else { continue };
        let ally = native.rendered_ship;
        // Only a ship still in a live ally slot may be dereferenced and composited.
        if ally == 0 || !allies.contains(&ally) {
            continue;
        }
        let sprite = native.sprite;
        let y = frame_height - PANEL_HEIGHT + slot as f32 * ALLY_SLOT_STEP;
        set_color(sprite, 0xffff_ffff, -1);
        set_z(sprite, layer_z(3), -1);
        (sprite as *mut u32).byte_add(SPRITE_BLEND).write_unaligned(BLEND_PANEL);
        render(sprite, ALLY_X, y, 0.0, ALLY_SCALE, ALLY_SCALE);
        displayed[slot] = ally;
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
