//! Pixel conversion between the engine's colors and softbuffer's `u32`
//! layout, plus the row-major RGBA8 → frame conversion used after
//! rasterization.
//!
//! These are the only pixel-format-sensitive items in the crate: everything
//! else paints through the vello family (`Painter` → `vello_cpu`), and the
//! rasterized premultiplied RGBA8 output lands here on its way into the
//! softbuffer-presented frame.

use vello_cpu::peniko::Color;

/// Convert RGB channels to softbuffer's documented pixel layout:
/// `00000000 RRRRRRRR GGGGGGGG BBBBBBBB` (top byte zero).
///
/// Platform byte-order quirks are handled inside softbuffer itself, so this
/// mapping is platform-independent.
pub fn rgb_to_u32(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

/// Convert a [`Color`] to the `u32` frame layout.
///
/// The frame is opaque (alpha is dropped); premultiplied channels over the
/// black background mean the stored RGB is the on-screen color, so partial
/// coverage (antialiased edges) darkens toward black exactly like the
/// zeroed frame behind it.
pub fn color_to_u32(color: Color) -> u32 {
    // Premultiplied components; the frame is opaque, so partial coverage
    // darkens toward black exactly like the zeroed frame behind it.
    let quantize = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let [r, g, b, _] = color.components;
    rgb_to_u32(quantize(r), quantize(g), quantize(b))
}

/// Convert a rasterized premultiplied RGBA8 buffer (row-major, 4 bytes per
/// pixel) into the `u32` frame layout. Partial-alpha pixels composite onto
/// black (the cleared frame color), matching the softbuffer contract that
/// the top byte stays zero.
pub(crate) fn premul_rgba8_to_frame(rgba: &[u8], frame: &mut [u32]) {
    assert_eq!(
        rgba.len(),
        frame.len() * 4,
        "RGBA scratch buffer must match the frame"
    );
    for (i, pixel) in frame.iter_mut().enumerate() {
        let j = i * 4;
        *pixel = rgb_to_u32(rgba[j], rgba[j + 1], rgba[j + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_u32_matches_the_documented_layout() {
        assert_eq!(rgb_to_u32(0x00, 0x00, 0x00), 0x0000_0000);
        assert_eq!(rgb_to_u32(0xFF, 0xFF, 0xFF), 0x00FF_FFFF);
        assert_eq!(rgb_to_u32(0xFF, 0x00, 0x00), 0x00FF_0000);
        assert_eq!(rgb_to_u32(0x00, 0xFF, 0x00), 0x0000_FF00);
        assert_eq!(rgb_to_u32(0x00, 0x00, 0xFF), 0x0000_00FF);
        assert_eq!(rgb_to_u32(0x12, 0x34, 0x56), 0x0012_3456);
    }

    #[test]
    fn color_conversion_is_lossless_for_opaque_colors() {
        assert_eq!(color_to_u32(Color::from_rgb8(0x12, 0x34, 0x56)), 0x0012_3456);
    }

    #[test]
    fn premul_rgba_conversion_drops_alpha_and_keeps_channels() {
        let mut frame = [0u32; 2];
        premul_rgba8_to_frame(
            &[0x12, 0x34, 0x56, 0xFF, 0xAB, 0xCD, 0xEF, 0x80],
            &mut frame,
        );
        assert_eq!(frame, [0x0012_3456, 0x00AB_CDEF]);
    }
}
