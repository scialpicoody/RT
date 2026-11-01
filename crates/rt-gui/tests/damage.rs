//! Damage tracking (widget invalidation), through the public seam: `Damage`.

use rt_gui::Damage;
use vello_cpu::kurbo::Rect;

fn rect(x: f64, y: f64, x1: f64, y1: f64) -> Rect {
    Rect::new(x, y, x1, y1)
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
    damage.mark(rect(2.0, 3.0, 6.0, 8.0));
    assert!(damage.is_dirty());
    assert_eq!(damage.take(), Some(rect(2.0, 3.0, 6.0, 8.0)));
    assert!(!damage.is_dirty(), "take clears the damage");
}

#[test]
fn marking_multiple_rects_coalesces_into_their_enclosing_rect() {
    let mut damage = Damage::new();
    damage.mark(rect(0.0, 0.0, 10.0, 10.0));
    damage.mark(rect(50.0, 40.0, 55.0, 45.0));
    damage.mark(rect(20.0, 60.0, 25.0, 65.0));
    // Union bounds: x 0..55, y 0..65 -> enclosing rectangle 0,0 55x65.
    assert_eq!(damage.take(), Some(rect(0.0, 0.0, 55.0, 65.0)));
}

#[test]
fn marks_after_take_start_a_new_damage_region() {
    let mut damage = Damage::new();
    damage.mark(rect(0.0, 0.0, 10.0, 10.0));
    damage.take();
    damage.mark(rect(100.0, 100.0, 105.0, 105.0));
    assert_eq!(damage.take(), Some(rect(100.0, 100.0, 105.0, 105.0)));
}

#[test]
fn contained_marks_do_not_grow_the_dirty_region() {
    let mut damage = Damage::new();
    damage.mark(rect(0.0, 0.0, 30.0, 30.0));
    damage.mark(rect(5.0, 5.0, 10.0, 10.0));
    assert_eq!(damage.take(), Some(rect(0.0, 0.0, 30.0, 30.0)));
}

#[test]
fn mark_all_explicitly_invalidates_everything() {
    let mut damage = Damage::new();
    damage.mark_all(640, 480);
    assert_eq!(damage.take(), Some(rect(0.0, 0.0, 640.0, 480.0)));
}
