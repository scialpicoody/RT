//! Button widget: interaction state machine (damage marking, click) and
//! geometry-to-primitives rendering, through the public seam: `Button`.

use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use rt_gui::{BufferTarget, Button, InputEvent, rgb888_to_u32};

fn button() -> Button {
    Button::new(Rectangle::new(Point::new(2, 3), Size::new(30, 20)), "Hi")
}

fn render(button: &Button) -> (Vec<u32>, Size) {
    let size = Size::new(40, 30);
    let mut buffer = vec![0u32; (size.width * size.height) as usize];
    let mut target = BufferTarget::new(&mut buffer, size);
    button.render(&mut target).unwrap();
    (buffer, size)
}

fn px(buffer: &[u32], size: Size, x: i32, y: i32) -> u32 {
    buffer[(y as u32 * size.width + x as u32) as usize]
}

#[test]
fn reports_its_geometry_and_hit_tests() {
    let b = button();
    assert_eq!(b.rect(), Rectangle::new(Point::new(2, 3), Size::new(30, 20)));
    assert!(b.contains(Point::new(2, 3)), "top-left corner is inside");
    assert!(b.contains(Point::new(31, 22)), "bottom-right corner is inside");
    assert!(!b.contains(Point::new(1, 3)));
    assert!(!b.contains(Point::new(32, 22)));
}

#[test]
fn render_fills_exactly_the_button_area() {
    let (buffer, size) = render(&button());
    // Background outside the button is untouched (zero) ...
    assert_eq!(px(&buffer, size, 0, 0), 0);
    assert_eq!(px(&buffer, size, 33, 23), 0);
    // ... every pixel inside the rect is painted: border on the rim,
    // fill just inside the rim.
    let fill = rgb888_to_u32(button().fill_color());
    let border = rgb888_to_u32(button().border_color());
    assert_eq!(px(&buffer, size, 2, 3), border, "top-left rim");
    assert_eq!(px(&buffer, size, 31, 22), border, "bottom-right rim");
    assert_eq!(px(&buffer, size, 2, 22), border, "bottom-left rim");
    assert_eq!(px(&buffer, size, 31, 3), border, "top-right rim");
    assert_eq!(px(&buffer, size, 3, 4), fill, "interior next to rim");
    assert_eq!(px(&buffer, size, 30, 21), fill, "interior far corner");
}

#[test]
fn render_draws_the_label_within_the_button() {
    let (buffer, size) = render(&button());
    let fill = rgb888_to_u32(button().fill_color());
    // The interior contains pixels darker than the fill: the label glyphs.
    let interior_x = 3..=30;
    let interior_y = 4..=21;
    let has_text = interior_x
        .clone()
        .flat_map(|x| interior_y.clone().map(move |y| (x, y)))
        .any(|(x, y)| px(&buffer, size, x, y) != fill);
    assert!(has_text, "no label pixels found inside the button");
}

#[test]
fn moving_over_the_button_marks_whole_button_damage_and_hover() {
    let mut b = button();
    let out = b.handle(&InputEvent::MouseMoved { x: 10, y: 10 });
    assert_eq!(out.damage, Some(b.rect()));
    let (buffer, size) = render(&b);
    let fill = rgb888_to_u32(b.fill_color());
    assert_eq!(px(&buffer, size, 3, 4), fill, "hover state has its own fill");

    // Moving again inside produces no new damage.
    let out = b.handle(&InputEvent::MouseMoved { x: 11, y: 11 });
    assert_eq!(out.damage, None);

    // Leaving the button marks damage again.
    let out = b.handle(&InputEvent::MouseMoved { x: 100, y: 100 });
    assert_eq!(out.damage, Some(b.rect()));
}

#[test]
fn press_and_release_inside_clicks_and_marks_damage_each_step() {
    let mut b = button();
    let press = b.handle(&InputEvent::MousePressed { x: 10, y: 10 });
    assert_eq!(press.damage, Some(b.rect()));
    assert!(!press.clicked);
    let release = b.handle(&InputEvent::MouseReleased { x: 10, y: 10 });
    assert_eq!(release.damage, Some(b.rect()));
    assert!(release.clicked, "release after press inside clicks");
}

#[test]
fn press_inside_release_outside_does_not_click() {
    let mut b = button();
    b.handle(&InputEvent::MousePressed { x: 10, y: 10 });
    let release = b.handle(&InputEvent::MouseReleased { x: 200, y: 200 });
    assert!(release.damage.is_some(), "visual state resets");
    assert!(!release.clicked);
    // A stale later release far away must not click either.
    let stray = b.handle(&InputEvent::MouseReleased { x: 0, y: 0 });
    assert!(!stray.clicked);
}

#[test]
fn press_outside_does_not_press_the_button() {
    let mut b = button();
    let out = b.handle(&InputEvent::MousePressed { x: 100, y: 100 });
    assert_eq!(out.damage, None);
    let release = b.handle(&InputEvent::MouseReleased { x: 10, y: 10 });
    assert!(!release.clicked, "release alone never clicks");
}

#[test]
fn pressed_state_renders_differently_than_normal() {
    let normal = render(&button()).0;
    let mut b = button();
    b.handle(&InputEvent::MousePressed { x: 10, y: 10 });
    let pressed = render(&b).0;
    assert_ne!(normal, pressed);
}

#[test]
fn resize_and_close_are_ignored() {
    let mut b = button();
    assert_eq!(b.handle(&InputEvent::Resized { width: 800, height: 600 }).damage, None);
    assert_eq!(b.handle(&InputEvent::CloseRequested).damage, None);
}
