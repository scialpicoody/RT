//! The widget-root state machine: input → damage, damage → coalesced repaint
//! scheduling — driven end to end through the headless [`Harness`] seam.

use rt_gui::harness::Harness;
use rt_gui::InputEvent;

fn harness() -> Harness {
    Harness::new(320, 240)
}

fn inside_move(h: &Harness, dx: i32, dy: i32) -> InputEvent {
    let b = h.button_rect();
    InputEvent::MouseMoved { x: b.top_left.x + dx, y: b.top_left.y + dy }
}

#[test]
fn fresh_gui_starts_dirty_so_the_first_paint_covers_everything() {
    let mut h = harness();
    assert!(h.is_dirty(), "fresh state is fully dirty");
    assert!(h.paint());
    assert_eq!(h.frame().len(), 320 * 240);
    assert!(h.is_clean(), "paint took the damage");
    assert!(!h.paint(), "second paint without input does nothing");
}

#[test]
fn non_interacting_input_neither_schedules_nor_paints() {
    let mut h = harness();
    h.paint();
    assert!(!h.feed(InputEvent::MouseMoved { x: 1, y: 1 }));
    assert!(h.is_clean());
    assert!(!h.paint());
}

#[test]
fn first_damaging_input_schedules_exactly_one_repaint() {
    let mut h = harness();
    h.paint();

    assert!(h.feed(inside_move(&h, 5, 5)), "hover schedules a repaint");
    assert!(!h.feed(inside_move(&h, 6, 6)), "already scheduled: coalesced");
    assert!(!h.feed(inside_move(&h, 7, 7)), "still coalesced");
    assert_eq!(h.schedule_log(), [true, false, false]);

    assert!(h.paint(), "scheduled repaint paints once");
    assert!(h.is_clean());
    assert!(!h.paint(), "and only once");
}

#[test]
fn damage_after_a_paint_schedules_a_new_repaint() {
    let mut h = harness();
    h.paint();
    assert!(h.feed(inside_move(&h, 5, 5)));
    h.paint();
    assert!(h.feed(InputEvent::MouseMoved { x: 0, y: 0 }), "leaving marks new damage");
    assert!(h.paint());
}

#[test]
fn resize_reallocates_the_frame_and_invalidates_everything() {
    let mut h = harness();
    h.paint();
    assert!(h.feed(InputEvent::Resized { width: 640, height: 480 }));
    assert!(h.paint());
    assert_eq!(h.frame().len(), 640 * 480);
    assert_eq!((h.width(), h.height()), (640, 480));
    assert!(h.is_clean());
}

#[test]
fn click_is_counted_as_the_reaction_to_a_full_press_release() {
    let mut h = harness();
    h.paint();
    let p = h.point_in_button();
    h.feed(InputEvent::MouseMoved { x: p.x, y: p.y });
    h.feed(InputEvent::MousePressed { x: p.x, y: p.y });
    assert_eq!(h.clicks(), 0, "press alone does nothing");
    h.feed(InputEvent::MouseReleased { x: p.x, y: p.y });
    assert_eq!(h.clicks(), 1);
    // A second full click counts again.
    h.feed(InputEvent::MousePressed { x: p.x, y: p.y });
    h.feed(InputEvent::MouseReleased { x: p.x, y: p.y });
    assert_eq!(h.clicks(), 2);
}

#[test]
fn painted_frame_shows_the_button_on_the_background() {
    let mut h = harness();
    let b = h.button_rect();
    assert!(h.paint());
    // Corner pixel of the screen is untouched background ...
    assert_eq!(h.pixel(0, 0), 0);
    // ... while a button rim pixel is painted.
    assert_ne!(h.pixel(b.top_left.x as u32, b.top_left.y as u32), 0);
}
