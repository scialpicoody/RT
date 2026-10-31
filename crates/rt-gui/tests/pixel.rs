//! Pixel conversion and the `&mut [u32]` DrawTarget adapter, through the
//! public seam: `rgb888_to_u32`, `BufferTarget`.

use embedded_graphics::prelude::*;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::primitives::PrimitiveStyle;
use rt_gui::BufferTarget;
use rt_gui::rgb888_to_u32;

#[test]
fn converts_channels_into_softbuffer_layout() {
    // Softbuffer's documented u32 layout: 00000000 RRRRRRRR GGGGGGGG BBBBBBBB.
    assert_eq!(rgb888_to_u32(Rgb888::BLACK), 0x0000_0000);
    assert_eq!(rgb888_to_u32(Rgb888::WHITE), 0x00FF_FFFF);
    assert_eq!(rgb888_to_u32(Rgb888::RED), 0x00FF_0000);
    assert_eq!(rgb888_to_u32(Rgb888::GREEN), 0x0000_FF00);
    assert_eq!(rgb888_to_u32(Rgb888::BLUE), 0x0000_00FF);
    assert_eq!(rgb888_to_u32(Rgb888::new(0x12, 0x34, 0x56)), 0x0012_3456);
}

#[test]
fn conversion_keeps_top_byte_zero() {
    for r in [0u8, 1, 0x7F, 0xFF] {
        for g in [0u8, 0x80, 0xFF] {
            for b in [0u8, 0xFF] {
                assert_eq!(rgb888_to_u32(Rgb888::new(r, g, b)) >> 24, 0);
            }
        }
    }
}

#[test]
fn target_reports_its_size() {
    let mut buffer = [0u32; 6];
    let target = BufferTarget::new(&mut buffer, Size::new(3, 2));
    assert_eq!(target.size(), Size::new(3, 2));
}

#[test]
fn target_writes_single_pixel_in_row_major_order() {
    let mut buffer = [0u32; 6];
    let mut target = BufferTarget::new(&mut buffer, Size::new(3, 2));
    Pixel(Point::new(1, 1), Rgb888::RED).draw(&mut target).unwrap();
    assert_eq!(buffer, [0, 0, 0, 0, 0x00FF_0000, 0]);
}

#[test]
fn target_clips_pixels_outside_its_bounds() {
    let mut buffer = [0u32; 6];
    let mut target = BufferTarget::new(&mut buffer, Size::new(3, 2));
    for p in [Point::new(-1, 0), Point::new(3, 0), Point::new(0, 2), Point::new(-5, -5)] {
        Pixel(p, Rgb888::BLUE).draw(&mut target).unwrap();
    }
    assert_eq!(buffer, [0; 6]);
}

#[test]
fn target_draws_rect_primitive_across_rows() {
    let mut buffer = [0u32; 9];
    let mut target = BufferTarget::new(&mut buffer, Size::new(3, 3));
    embedded_graphics::primitives::Rectangle::new(Point::new(1, 0), Size::new(2, 3))
        .into_styled(PrimitiveStyle::with_fill(Rgb888::GREEN))
        .draw(&mut target)
        .unwrap();
    assert_eq!(buffer, [0, 0x0000_FF00, 0x0000_FF00, 0, 0x0000_FF00, 0x0000_FF00, 0, 0x0000_FF00, 0x0000_FF00]);
}
