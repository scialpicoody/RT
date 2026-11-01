//! Widget tree end-to-end: flex geometry via the public `rect_of` accessor,
//! post-layout hit testing under nesting, layout-at-most-once-per-cycle, and
//! relayout-driven damage — through the headless [`Harness`] seam.

use rt_gui::harness::Harness;
use rt_gui::{Button, InputEvent, color_to_u32};
use vello_cpu::kurbo::{Point, Rect};

/// A column of three buttons: the legacy first button plus two pushed ones.
fn column_of_three() -> (Harness, [rt_gui::WidgetId; 3]) {
    let mut h = Harness::new(320, 240);
    let b0 = h.button_id(0);
    let b1 = h.push_button("Two");
    let b2 = h.push_button("Three");
    h.paint();
    (h, [b0, b1, b2])
}

fn inside(rect: Rect, dx: i32, dy: i32) -> (i32, i32) {
    let c = rect.center();
    (c.x as i32 + dx, c.y as i32 + dy)
}

#[test]
fn column_lays_out_buttons_in_flex_order_with_gap() {
    let (h, ids) = column_of_three();
    // Root column: padding 40 on all edges, gap 12, buttons fixed 200x88.
    let r0 = h.rect_of(ids[0]).expect("laid out");
    let r1 = h.rect_of(ids[1]).expect("laid out");
    let r2 = h.rect_of(ids[2]).expect("laid out");

    assert_eq!(
        r0,
        Rect::new(40.0, 40.0, 240.0, 128.0),
        "first button at padding origin"
    );
    assert_eq!(
        r1,
        Rect::new(40.0, 140.0, 240.0, 228.0),
        "second button one gap below"
    );
    assert_eq!(
        r2,
        Rect::new(40.0, 240.0, 240.0, 328.0),
        "third button clipped by the frame"
    );
    assert_eq!(r1.y0 - r0.y1, 12.0, "gap honored between buttons 1 and 2");
    assert_eq!(r2.y0 - r1.y1, 12.0, "gap honored between buttons 2 and 3");
    assert_eq!(r0.width(), 200.0, "fixed flex item width");
    assert_eq!(r0.height(), 88.0, "fixed flex item height");
}

#[test]
fn row_container_lays_out_children_horizontally() {
    let mut h = Harness::new(320, 240);
    // Replace composition: push a Row into the root column and buttons into
    // the Row via the tree API.
    let root = h.root_id();
    let row = h.tree_add_child(
        root,
        rt_gui::Row {
            gap: 10.0,
            padding: 0.0,
            ..Default::default()
        },
    );
    let a = h.tree_add_button(row, "A");
    let b = h.tree_add_button(row, "B");
    h.paint();

    let (ra, rb) = (h.rect_of(a).unwrap(), h.rect_of(b).unwrap());
    assert_eq!(rb.x0 - ra.x1, 10.0, "row gap between side-by-side buttons");
    assert_eq!(ra.y0, rb.y0, "row children share the main-axis line");
}

#[test]
fn click_routes_to_exactly_the_second_button() {
    let (mut h, ids) = column_of_three();
    let r1 = h.rect_of(ids[1]).unwrap();
    let (x, y) = inside(r1, 0, 0);

    h.feed(InputEvent::MouseMoved { x, y });
    h.feed(InputEvent::MousePressed { x, y });
    h.feed(InputEvent::MouseReleased { x, y });
    assert_eq!(h.clicks(), 1, "click over button 2 hits exactly one widget");

    // And the hit widget really is the second button: its pixels change
    // (released back to hovered) while the other buttons stay normal.
    let (ux, uy) = (x as u32, y as u32);
    h.paint();
    let normal = color_to_u32(Button::new(Rect::new(0.0, 0.0, 1.0, 1.0), "").fill_color());
    assert_ne!(h.pixel(ux, uy), normal, "button 2 reacted");

    let r0 = h.rect_of(ids[0]).unwrap();
    let (c0x, c0y) = inside(r0, 0, 0);
    assert_eq!(
        h.pixel(c0x as u32, c0y as u32),
        normal,
        "button 1 untouched"
    );
    let r2 = h.rect_of(ids[2]).unwrap();
    // Button 3 is pushed fully below the frame (starts at the bottom edge):
    // no visible pixel to probe, its non-reaction is covered by the click count.
    assert!(r2.y0 >= 240.0, "button 3 is off-frame");
}

#[test]
fn pressed_pixels_differ_for_the_second_button_only() {
    let (mut h, ids) = column_of_three();
    let r1 = h.rect_of(ids[1]).unwrap();
    let (x, y) = inside(r1, 3, 3);
    h.feed(InputEvent::MouseMoved { x, y });
    h.feed(InputEvent::MousePressed { x, y });
    assert!(h.paint());

    let mut pressed_probe = Button::new(Rect::new(0.0, 0.0, 1.0, 1.0), "");
    pressed_probe.handle(&InputEvent::MouseMoved { x: 0, y: 0 });
    pressed_probe.handle(&InputEvent::MousePressed { x: 0, y: 0 });
    let pressed = color_to_u32(pressed_probe.fill_color());

    let normal = color_to_u32(Button::new(Rect::new(0.0, 0.0, 1.0, 1.0), "").fill_color());
    assert_eq!(
        h.pixel(x as u32, y as u32),
        pressed,
        "button 2 shows pressed fill"
    );
    let r0 = h.rect_of(ids[0]).unwrap();
    let (px, py) = (r0.x0 as u32 + 10, r0.y0 as u32 + 10);
    assert_eq!(h.pixel(px, py), normal, "other visible buttons stay normal");
    // Button 3 lies fully below the frame: no pixel to probe.
    assert!(h.rect_of(ids[2]).unwrap().y0 >= 240.0);
}

#[test]
fn hit_test_prefers_the_last_child_back_to_front() {
    // Two overlapping buttons: the later child wins at the overlap.
    let mut h = Harness::new(320, 240);
    let root = h.root_id();
    let first = h.button_id(0);
    let second = h.tree_add_child(
        root,
        OverlappingButton::pinned(Rect::new(60.0, 40.0, 260.0, 200.0)),
    );
    h.paint();

    // Point inside both rects: the second (later) button is on top.
    assert_eq!(h.tree().hit_test(Point::new(130.0, 100.0)), Some(second));
    // Point only inside the first: the first is still reachable.
    assert_eq!(h.tree().hit_test(Point::new(50.0, 50.0)), Some(first));
    // Point over no child: the root column container matches (containers
    // receive but ignore input), so the event reaches nobody in effect.
    assert_eq!(
        h.tree().hit_test(Point::new(300.0, 10.0)),
        Some(h.root_id())
    );
    h.feed(InputEvent::MousePressed { x: 300, y: 10 });
    h.feed(InputEvent::MouseReleased { x: 300, y: 10 });
    assert_eq!(h.clicks(), 0, "container hits are inert");
    assert_eq!(
        h.tree().hit_test(Point::new(320.5, 10.0)),
        None,
        "outside the frame"
    );
}

/// Minimal test widget pinned to a fixed rect, for z-order assertions.
struct OverlappingButton(Button);

impl OverlappingButton {
    fn pinned(rect: Rect) -> Self {
        Self(Button::new(rect, "top"))
    }
}

impl rt_gui::Widget for OverlappingButton {
    fn handle_input(&mut self, input: &rt_gui::InputEvent, _rect: Rect) -> rt_gui::WidgetFeedback {
        self.0.handle(input).into_feedback()
    }
    fn paint(&self, painter: &mut rt_gui::Painter, rect: Rect) {
        self.0.render_at(painter, rect);
    }
    fn style(&self) -> taffy::Style {
        taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::style_helpers::length(self.0.rect().width() as f32),
                height: taffy::style_helpers::length(self.0.rect().height() as f32),
            },
            inset: taffy::Rect {
                left: taffy::style_helpers::length(self.0.rect().x0 as f32),
                top: taffy::style_helpers::length(self.0.rect().y0 as f32),
                right: taffy::style_helpers::auto(),
                bottom: taffy::style_helpers::auto(),
            },
            ..taffy::Style::default()
        }
    }
}

trait IntoFeedback {
    fn into_feedback(self) -> rt_gui::WidgetFeedback;
}

impl IntoFeedback for rt_gui::ButtonOutcome {
    fn into_feedback(self) -> rt_gui::WidgetFeedback {
        rt_gui::WidgetFeedback {
            damage: self.damage.is_some(),
            clicked: self.clicked,
        }
    }
}

#[test]
fn layout_runs_at_most_once_per_cycle_and_before_paint() {
    let (mut h, ids) = column_of_three();
    let r0_before = h.rect_of(ids[0]).unwrap();

    // A clean tree relayouts for free: repeated paints add no damage.
    assert!(h.is_clean());
    assert!(!h.paint(), "no repaint without input or structural change");
    assert_eq!(
        h.rect_of(ids[0]),
        Some(r0_before),
        "geometry stable across cycles"
    );

    // A structural change dirties layout; the NEXT paint picks it up and the
    // changed rects invalidate damage.
    let b4 = h.push_button("Four");
    assert!(h.paint(), "relayout changed rects -> repaint");
    let r0_after = h.rect_of(ids[0]).unwrap();
    assert_eq!(r0_after, r0_before, "existing buttons keep their rects");
    assert!(h.rect_of(b4).is_some(), "new button has resolved geometry");
    assert!(!h.paint(), "layout clean again: at most once per cycle");
}

#[test]
fn resize_forces_relayout_and_marks_damage() {
    let (mut h, ids) = column_of_three();
    let r0 = h.rect_of(ids[0]).unwrap();
    h.feed(InputEvent::Resized {
        width: 500,
        height: 300,
    });
    assert!(h.is_dirty(), "resize invalidates everything");
    assert!(h.paint());
    assert_eq!(
        h.rect_of(ids[0]),
        Some(r0),
        "padding-anchored first button unchanged"
    );
    assert_eq!(h.frame().len(), 500 * 300, "frame reallocated");
    assert!(h.is_clean());

    // A second resize that leaves geometry unchanged still repaints once
    // (conservative invalidation), then settles.
    h.feed(InputEvent::Resized {
        width: 500,
        height: 300,
    });
    assert!(h.paint());
    assert!(!h.paint());
}

#[test]
fn nested_container_routes_clicks_to_the_leaf() {
    let mut h = Harness::new(320, 240);
    let root = h.root_id();
    let row = h.tree_add_child(root, rt_gui::Row::default());
    let leaf = h.tree_add_button(row, "leaf");
    h.paint();

    let r = h.rect_of(leaf).unwrap();
    let (x, y) = inside(r, 0, 0);
    h.feed(InputEvent::MouseMoved { x, y });
    h.feed(InputEvent::MousePressed { x, y });
    h.feed(InputEvent::MouseReleased { x, y });
    assert_eq!(h.clicks(), 1, "click reaches the leaf through the nesting");
    assert!(h.tree().hit_test(Point::new(x as f64, y as f64)) == Some(leaf));
}
