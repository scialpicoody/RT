//! Harness-native end-to-end tests: build widget state -> synthetic input ->
//! paint -> assert frame pixels + damage, plus observable widget outcomes.

use rt_gui::harness::Harness;
use rt_gui::{Button, InputEvent};
use vello_cpu::kurbo::Rect;

/// Expected button body color for a given visual state, via the widget's own
/// color contract (drive a detached button into the state).
fn fill_in_state(events: &[InputEvent]) -> u32 {
    let mut b = Button::new(Rect::new(0.0, 0.0, 1.0, 1.0), "");
    for e in events {
        b.handle(e);
    }
    rt_gui::color_to_u32(b.fill_color())
}

fn hovered_fill() -> u32 {
    fill_in_state(&[InputEvent::MouseMoved { x: 0, y: 0 }])
}

fn pressed_fill() -> u32 {
    fill_in_state(&[
        InputEvent::MouseMoved { x: 0, y: 0 },
        InputEvent::MousePressed { x: 0, y: 0 },
    ])
}

#[test]
fn input_paints_pixels_end_to_end() {
    let mut h = Harness::new(320, 240);
    let b = h.button_rect();
    let (ix, iy) = ((b.x0 as i32 + 10) as u32, (b.y0 as i32 + 10) as u32);

    assert!(h.paint(), "fresh state is dirty and paints");
    // Fresh paint: normal button fill inside, background outside.
    let normal = fill_in_state(&[]);
    assert_eq!(h.pixel(ix, iy), normal);
    assert_eq!(h.pixel(0, 0), 0);

    // Hover marks damage; nothing repaints until paint is pumped.
    assert!(h.is_clean(), "previous paint took the damage");
    assert!(h.feed(InputEvent::MouseMoved { x: b.x0 as i32 + 10, y: b.y0 as i32 + 10 }));
    assert!(h.is_dirty(), "hover marked damage");
    assert!(h.paint(), "damage drives a repaint");
    assert!(h.is_clean(), "paint consumed the damage");
    assert_eq!(h.pixel(ix, iy), hovered_fill(), "center pixel shows hovered fill");
    assert!(!h.paint(), "clean state does not repaint");
}

#[test]
fn pressed_pixel_and_click_outcome_are_observable() {
    let mut h = Harness::new(320, 240);
    let p = h.point_in_button();
    let (px, py) = (p.x as u32, p.y as u32);
    h.paint();

    h.feed(InputEvent::MouseMoved { x: p.x as i32, y: p.y as i32 });
    h.feed(InputEvent::MousePressed { x: p.x as i32, y: p.y as i32 });
    assert!(h.is_dirty());
    assert!(h.paint());
    assert_eq!(h.pixel(px, py), pressed_fill(), "center pixel shows pressed fill");
    assert_eq!(h.clicks(), 0, "press alone is not a click");

    h.feed(InputEvent::MouseReleased { x: p.x as i32, y: p.y as i32 });
    assert!(h.is_dirty());
    assert!(h.paint());
    assert_eq!(h.clicks(), 1, "press+release inside is one click");
    assert_eq!(h.pixel(px, py), hovered_fill(), "released back to hovered fill");
}

#[test]
fn outside_press_neither_marks_damage_nor_clicks() {
    let mut h = Harness::new(320, 240);
    h.paint();
    assert!(!h.feed(InputEvent::MousePressed { x: 2, y: 2 }));
    assert!(!h.feed(InputEvent::MouseReleased { x: 2, y: 2 }));
    assert!(h.is_clean());
    assert!(!h.paint());
    assert_eq!(h.clicks(), 0);
    assert_eq!(h.schedule_log(), [false, false]);
}

#[test]
fn harness_sees_damage_mark_take_cycle_through_coalescing() {
    let mut h = Harness::new(160, 120);
    assert!(h.is_dirty(), "construction marks all");
    h.paint();
    assert!(h.is_clean());

    // Two hovers inside the button mark twice but coalesce: one repaint.
    let p = h.point_in_button();
    h.feed(InputEvent::MouseMoved { x: p.x as i32 - 3, y: p.y as i32 });
    assert!(h.is_dirty());
    h.feed(InputEvent::MouseMoved { x: p.x as i32 + 3, y: p.y as i32 });
    assert!(h.is_dirty(), "second mark still dirty until taken");
    h.paint();
    assert!(h.is_clean());
    assert_eq!(h.schedule_log(), [true, false]);
    assert_eq!(h.last_scheduled(), Some(false));
}

#[test]
fn label_change_invalidates_and_repaints_through_the_harness() {
    let mut h = Harness::new(320, 240);
    let b = h.button_rect();
    h.paint();
    let glyph_y = (b.y1 as i32 + 4) as u32;
    assert_eq!(h.pixel(50, glyph_y), 0, "no label painted yet");
    assert!(!h.feed(InputEvent::MouseMoved { x: 0, y: 0 }), "no-op hover outside");
    h.paint();
    assert!(h.is_clean());
    assert!(!h.paint(), "harness stays clean without new damage");
}
