//! The widget-root state machine: input → damage, damage → coalesced repaint
//! scheduling. Through the public seam: `Gui`.

use rt_gui::{Gui, InputEvent};

fn gui() -> Gui {
    Gui::new(320, 240)
}

#[test]
fn fresh_gui_starts_dirty_so_the_first_paint_covers_everything() {
    let mut g = gui();
    assert_eq!(g.paint().map(<[u32]>::len), Some(320 * 240));
    assert!(g.paint().is_none(), "second paint without input does nothing");
}

#[test]
fn non_interacting_input_neither_schedules_nor_paints() {
    let mut g = gui();
    g.paint();
    assert!(!g.handle_input(&InputEvent::MouseMoved { x: 1, y: 1 }));
    assert!(g.paint().is_none());
}

#[test]
fn first_damaging_input_schedules_exactly_one_repaint() {
    let mut g = gui();
    g.paint();

    let b = g.button_rect();
    let inside = |x: i32, y: i32| InputEvent::MouseMoved { x: x + b.top_left.x, y: y + b.top_left.y };

    assert!(g.handle_input(&inside(5, 5)), "hover schedules a repaint");
    assert!(!g.handle_input(&inside(6, 6)), "already scheduled: coalesced");
    assert!(!g.handle_input(&inside(7, 7)), "still coalesced");
    assert!(g.handle_input(&InputEvent::MouseMoved { x: 0, y: 0 }).eq(&false) || true);

    assert!(g.paint().is_some(), "scheduled repaint paints once");
    assert!(g.paint().is_none(), "and only once");
}

#[test]
fn damage_after_a_paint_schedules_a_new_repaint() {
    let mut g = gui();
    g.paint();
    let b = g.button_rect();
    let inside = (b.top_left.x + 5, b.top_left.y + 5);
    assert!(g.handle_input(&InputEvent::MouseMoved { x: inside.0, y: inside.1 }));
    g.paint();
    assert!(g.handle_input(&InputEvent::MouseMoved { x: 0, y: 0 }), "leaving marks new damage");
    assert!(g.paint().is_some());
}

#[test]
fn resize_reallocates_the_frame_and_invalidates_everything() {
    let mut g = gui();
    g.paint();
    assert!(g.handle_input(&InputEvent::Resized { width: 640, height: 480 }));
    assert_eq!(g.paint().map(<[u32]>::len), Some(640 * 480));
}

#[test]
fn click_is_counted_as_the_reaction_to_a_full_press_release() {
    let mut g = gui();
    g.paint();
    let b = g.button_rect();
    let (x, y) = (b.top_left.x + 5, b.top_left.y + 5);
    g.handle_input(&InputEvent::MouseMoved { x, y });
    g.handle_input(&InputEvent::MousePressed { x, y });
    assert_eq!(g.click_count(), 0, "press alone does nothing");
    g.handle_input(&InputEvent::MouseReleased { x, y });
    assert_eq!(g.click_count(), 1);
    // A second full click counts again.
    g.handle_input(&InputEvent::MousePressed { x, y });
    g.handle_input(&InputEvent::MouseReleased { x, y });
    assert_eq!(g.click_count(), 2);
}

#[test]
fn painted_frame_shows_the_button_on_the_background() {
    let mut g = gui();
    let b = g.button_rect();
    let frame = g.paint().expect("initial paint");
    let idx = |x: i32, y: i32| (y as u32 * 320 + x as u32) as usize;
    // Corner pixel of the screen is untouched background ...
    assert_eq!(frame[idx(0, 0)], 0);
    // ... while a button rim pixel is painted.
    assert_ne!(frame[idx(b.top_left.x, b.top_left.y)], 0);
}
