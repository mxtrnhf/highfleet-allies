//! Native layout evidence: ../../docs/vanilla-performance.md.
//! No native objects are copied, owned, retained, or freed by this module.

pub const GLYPH_COUNT: usize = 256;
/// Offset of the native font's 256-entry glyph pointer array.
pub const GLYPH_POINTERS: usize = 8;
/// Offset of the native font's cached text color (`dwCol`).
pub const FONT_COLOR_OFFSET: usize = 0x1020;
/// `GfxQuad` corner color offsets, in native `setSpriteFields(quad, color, -1)` order.
pub const CORNER_COLOR_OFFSETS: [usize; 4] = [0x18, 0x70, 0xc8, 0x120];

/// Assign the native font color without 256 out-of-line sprite setter calls.
///
/// # Safety
/// `font` must be a live, aligned Steam 1.163 hgeFont, exclusively accessed on
/// the rendering thread for this call. Each non-null glyph must be a live,
/// aligned writable GfxQuad. These are the original setter's preconditions;
/// readable memory or a matching version alone does not establish them.
/// Glyph allocations must not overlap font storage, as with native hgeFont.
pub unsafe extern "win64" fn set_color(font: *mut u8, color: u32) {
    // SAFETY: The entry detour inherits the native caller's object lifetime and
    // thread confinement; all accesses below are bounded scalar/pointer fields.
    // We create no Rust references to native objects and retain nothing on return.
    unsafe {
        font.add(FONT_COLOR_OFFSET).cast::<u32>().write(color);
        for index in 0..GLYPH_COUNT {
            let glyph = font
                .add(GLYPH_POINTERS + index * size_of::<*mut u8>())
                .cast::<*mut u8>()
                .read();
            if glyph.is_null() {
                continue;
            }
            // Check every corner, not font.dwCol or a representative glyph.
            // This repairs independent per-corner writes on the next assignment.
            if CORNER_COLOR_OFFSETS
                .iter()
                .all(|&offset| glyph.add(offset).cast::<u32>().read() == color)
            {
                continue;
            }
            // Same order as setSpriteFields(quad, color, -1).
            for offset in CORNER_COLOR_OFFSETS.into_iter().rev() {
                glyph.add(offset).cast::<u32>().write(color);
            }
        }
    }
}
