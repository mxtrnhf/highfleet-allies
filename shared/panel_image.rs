//! Native ship panel render targets used by the battle HUD and Shipworks damage viewer.

use crate::native::read as rd;

/// `DrawBattleShipPanel(surface_a, surface_b, mode)`.
pub const PANEL: usize = 0x1_402f_de80;

/// HGE helpers: draw a sprite scaled, set its colour and its z.
const RENDER_EX: usize = 0x1_4017_2aa0;
const SET_SPRITE_COLOR: usize = 0x1_4017_2ae0;
const SET_SPRITE_Z: usize = 0x1_4017_2ee0;

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

pub const PANEL_SUPPRESSED: usize = 0x1_43a1_5908;

/// Player profile creature lookup used by mode 3.
pub const GET_PLAYER_CREATURE: usize = 0x1_4029_24b0;

// Texture allocation takes whole pixels; the sprite constructor takes the same size as floats.
const PANEL_PIXEL_WIDTH: u32 = 350;
const PANEL_PIXEL_HEIGHT: u32 = 300;
pub const PANEL_WIDTH: f32 = PANEL_PIXEL_WIDTH as f32;
pub const PANEL_HEIGHT: f32 = PANEL_PIXEL_HEIGHT as f32;

type PanelFn = unsafe extern "win64" fn(usize, usize, u32);
type RenderExFn = unsafe extern "win64" fn(usize, f32, f32, f32, f32, f32);
type SetColorFn = unsafe extern "win64" fn(usize, u32, i32);
type SetZFn = unsafe extern "win64" fn(usize, f32, i32);

type CreateTextureFn = unsafe extern "win64" fn(usize, u32, u32, u32, u32) -> usize;
type CreateDepthFn = unsafe extern "win64" fn(usize, u32, u32) -> usize;
type AllocFn = unsafe extern "win64" fn(u32) -> usize;
type SpriteCtorFn = unsafe extern "win64" fn(usize, usize, f32, f32, f32, f32, f32, f32, f32, f32);

#[derive(Clone, Copy)]
pub struct PanelImage {
    surface_a: usize,
    sprite: usize,
}

impl PanelImage {
    /// Allocate a private colour target and its composite sprite on the render thread.
    /// The caller keeps the handles in a bounded, process-lifetime target pool.
    pub unsafe fn create() -> Option<Self> {
        let engine = rd::<usize>(ENGINE);
        if engine == 0 {
            return None;
        }
        let vtable = rd::<usize>(engine);
        let create: CreateTextureFn =
            std::mem::transmute(rd::<usize>(vtable + CREATE_TEXTURE_SLOT));
        let surface_a = create(
            engine,
            PANEL_PIXEL_WIDTH,
            PANEL_PIXEL_HEIGHT,
            TEXTURE_FORMAT,
            RENDER_TARGET,
        );
        let alloc: AllocFn = std::mem::transmute(OPERATOR_NEW);
        let sprite = alloc(SPRITE_SIZE);
        if surface_a == 0 || sprite == 0 {
            return None;
        }
        let ctor: SpriteCtorFn = std::mem::transmute(SPRITE_CTOR);
        // SAFETY: the sprite is fresh storage from the native allocator. It borrows the
        // matching native texture, both of which are kept by the target pool.
        ctor(
            sprite,
            surface_a,
            0.0,
            0.0,
            PANEL_WIDTH,
            PANEL_HEIGHT,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        Some(Self { surface_a, sprite })
    }

    /// Draw the chosen native panel into this image. A matching depth surface must be
    /// available and the caller must be in the native battle panel render pass.
    pub unsafe fn capture(self, depth: usize, mode: u32) {
        let panel: PanelFn = std::mem::transmute(PANEL);
        panel(self.surface_a, depth, mode);
    }

    /// Composite this image in the current native sprite pass at top-left `(x, y)`.
    pub unsafe fn draw(self, x: f32, y: f32, z: f32, scale: f32) {
        let set_z: SetZFn = std::mem::transmute(SET_SPRITE_Z);
        let set_color: SetColorFn = std::mem::transmute(SET_SPRITE_COLOR);
        let render: RenderExFn = std::mem::transmute(RENDER_EX);
        set_color(self.sprite, 0xffff_ffff, -1);
        set_z(self.sprite, z, -1);
        (self.sprite as *mut u32)
            .byte_add(SPRITE_BLEND)
            .write_unaligned(BLEND_PANEL);
        render(self.sprite, x, y, 0.0, scale, scale);
    }
}

/// Create a native depth target matching `PanelImage`. Only the game render thread
/// calls the HGE factory; the caller holds the returned handle for the process.
pub unsafe fn create_panel_depth() -> Option<usize> {
    let engine = rd::<usize>(ENGINE);
    if engine == 0 {
        return None;
    }
    let vtable = rd::<usize>(engine);
    let create: CreateDepthFn = std::mem::transmute(rd::<usize>(vtable + CREATE_DEPTH_SLOT));
    let depth = create(engine, PANEL_PIXEL_WIDTH, PANEL_PIXEL_HEIGHT);
    (depth != 0).then_some(depth)
}
