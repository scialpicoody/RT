//! Pure pixel conversion between [`Rgb888`] and softbuffer's `u32` layout,
//! plus [`BufferTarget`], the [`DrawTarget`] adapter over `&mut [u32]`.
//!
//! These are the only pixel-format-sensitive items in the crate: everything
//! else draws through `embedded_graphics` in `Rgb888`.

use embedded_graphics::geometry::{OriginDimensions, Point, Size};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::{DrawTarget, RgbColor};
use embedded_graphics::Pixel;

/// Convert `color` to softbuffer's documented pixel layout:
/// `00000000 RRRRRRRR GGGGGGGG BBBBBBBB` (top byte zero).
///
/// Platform byte-order quirks are handled inside softbuffer itself, so this
/// mapping is platform-independent.
pub fn rgb888_to_u32(color: Rgb888) -> u32 {
    (u32::from(color.r()) << 16) | (u32::from(color.g()) << 8) | u32::from(color.b())
}

/// A [`DrawTarget`] over a `u32` pixel buffer in softbuffer layout.
///
/// Rows are laid out consecutively (row-major); out-of-bounds pixels are
/// clipped.
pub struct BufferTarget<'a> {
    buffer: &'a mut [u32],
    size: Size,
}

impl<'a> BufferTarget<'a> {
    /// Wrap `buffer` (row-major, `size` pixels wide) as a draw target.
    pub fn new(buffer: &'a mut [u32], size: Size) -> Self {
        Self { buffer, size }
    }

    /// The raw pixels, for tests and diagnostics.
    pub fn as_slice(&self) -> &[u32] {
        self.buffer
    }
}

impl DrawTarget for BufferTarget<'_> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb888>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(pos, color) in pixels {
            if let Some(index) = index_of(pos, self.size) {
                self.buffer[index] = rgb888_to_u32(color);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for BufferTarget<'_> {
    fn size(&self) -> Size {
        self.size
    }
}

/// Row-major index of `pos` inside a `size` target, or `None` if outside.
fn index_of(pos: Point, size: Size) -> Option<usize> {
    if pos.x < 0 || pos.y < 0 {
        return None;
    }
    let (x, y) = (pos.x as u32, pos.y as u32);
    if x >= size.width || y >= size.height {
        return None;
    }
    Some((y * size.width + x) as usize)
}
