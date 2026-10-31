//! Damage tracking (widget invalidation), through the public seam: `Damage`.

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::primitives::Rectangle;
use rt_gui::Damage;

fn rect(x: i32, y: i32, w: u32, h: u32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}

#[test]
fn fresh_damage_is_clean_and_take_yields_none() {
    let mut damage = Damage::new();
    assert!(!damage.is_dirty());
    assert_eq!(damage.take(), None);
    assert!(!damage.is_dirty());
}

#[test]
fn marking_single_rect_makes_exactly_that_rect_dirty() {
    let mut damage = Damage::new();
    damage.mark(rect(2, 3, 4, 5));
    assert!(damage.is_dirty());
    assert_eq!(damage.take(), Some(rect(2, 3, 4, 5)));
    assert!(!damage.is_dirty(), "take clears the damage");
}

#[test]
fn marking_multiple_rects_coalesces_into_their_enclosing_rect() {
    let mut damage = Damage::new();
    damage.mark(rect(0, 0, 10, 10));
    damage.mark(rect(50, 40, 5, 5));
    damage.mark(rect(20, 60, 5, 5));
    // Union bounds: x 0..55, y 0..65 -> enclosing rectangle 0,0 55x65.
    assert_eq!(damage.take(), Some(rect(0, 0, 55, 65)));
}

#[test]
fn marks_after_take_start_a_new_damage_region() {
    let mut damage = Damage::new();
    damage.mark(rect(0, 0, 10, 10));
    damage.take();
    damage.mark(rect(100, 100, 5, 5));
    assert_eq!(damage.take(), Some(rect(100, 100, 5, 5)));
}

#[test]
fn contained_marks_do_not_grow_the_dirty_region() {
    let mut damage = Damage::new();
    damage.mark(rect(0, 0, 30, 30));
    damage.mark(rect(5, 5, 5, 5));
    assert_eq!(damage.take(), Some(rect(0, 0, 30, 30)));
}

#[test]
fn mark_all_explicitly_invalidates_everything() {
    let mut damage = Damage::new();
    damage.mark_all(Size::new(640, 480));
    assert_eq!(damage.take(), Some(rect(0, 0, 640, 480)));
}
