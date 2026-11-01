//! The widget-root orchestrator: owns the frame buffer, the vello paint
//! engine ([`Painter`]), and the owned [`WidgetTree`] rooted in a flex
//! [`Column`]. Layout runs at most once per cycle (before painting and
//! before mouse routing); mouse events are hit-tested against post-layout
//! geometry, back to front; painting coalesces damage at one point —
//! `paint` (Flush discipline).
//!
//! SEAM: with rt-core objects in the loop, input dispatch becomes typed
//! signal emissions and `paint` is driven from the loop's Flush step; the
//! widget-facing contract below stays the same.

use vello_cpu::kurbo::{Point, Rect};

use crate::damage::Damage;
use crate::input::InputEvent;
use crate::paint::{LABEL_COLOR, Painter};
use crate::tree::{Column, Widget, WidgetId, WidgetTree};
use crate::widget::Button;

/// One window's widget root and frame, in softbuffer pixel layout.
pub struct Gui {
    width: u32,
    height: u32,
    frame: Vec<u32>,
    painter: Painter,
    tree: WidgetTree,
    root: WidgetId,
    buttons: Vec<WidgetId>,
    damage: Damage,
    clicks: u32,
    /// The widget currently holding a press, so a release that leaves every
    /// rect still reaches it (press inside, release outside: visual reset,
    /// no click — the pre-tree Button semantics).
    pressed: Option<WidgetId>,
    /// The widget the cursor is currently over, so a move that leaves every
    /// rect still reaches the previously hovered widget (it must un-hover).
    hovered: Option<WidgetId>,
    label: String,
}

impl Gui {
    pub fn new(width: u32, height: u32) -> Self {
        let mut tree = WidgetTree::new();
        let root = tree.set_root(Column {
            gap: 12.0,
            padding: 40.0,
            ..Column::new()
        });
        let mut damage = Damage::new();
        damage.mark_all(width, height);
        let mut gui = Self {
            width,
            height,
            frame: vec![0; (width * height) as usize],
            painter: Painter::new(width, height),
            tree,
            root,
            buttons: Vec::new(),
            damage,
            clicks: 0,
            pressed: None,
            hovered: None,
            label: String::new(),
        };
        gui.push_button("Press me");
        gui.ensure_layout();
        gui
    }

    /// Append a button as the root column's last child; returns its tree id.
    pub fn push_button(&mut self, label: impl Into<String>) -> WidgetId {
        let id = self.tree.add_child(self.root, Button::new(Rect::ZERO, label));
        self.buttons.push(id);
        self.tree.invalidate_layout();
        id
    }

    /// Append an arbitrary widget under `parent` (composition seam).
    pub fn add_child(
        &mut self,
        parent: WidgetId,
        widget: impl Widget + 'static,
    ) -> WidgetId {
        let id = self.tree.add_child(parent, widget);
        self.tree.invalidate_layout();
        id
    }

    /// The tree id of the `index`-th pushed button.
    pub fn button_id(&self, index: usize) -> WidgetId {
        self.buttons[index]
    }

    /// The resolved post-layout rect of `id` (public geometry accessor).
    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.tree.rect_of(id)
    }

    /// The root column's id, for composing further children.
    pub fn root_id(&self) -> WidgetId {
        self.root
    }

    /// The owned widget tree (read-only access for tests and composition).
    pub fn tree(&self) -> &WidgetTree {
        &self.tree
    }

    /// The first pushed button's rect (legacy test seam; `None` before the
    /// first layout or with no buttons).
    pub fn button_rect(&self) -> Rect {
        self.rect_of(self.buttons[0]).expect("gui has a laid-out button")
    }

    /// Status line rendered at the bottom edge; setting it invalidates
    /// everything (basic full repaint).
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
        self.damage.mark_all(self.width, self.height);
    }

    /// Feed one translated input event. Returns `true` when it scheduled new
    /// damage (repaint needed); already-scheduled damage coalesces to `false`.
    pub fn handle_input(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Resized { width, height } => {
                self.width = width;
                self.height = height;
                self.frame = vec![0; (width * height) as usize];
                self.painter.resize(width, height);
                self.tree.invalidate_layout();
                self.ensure_layout();
                self.damage.mark_all(width, height);
                true
            }
            InputEvent::CloseRequested => false,
            InputEvent::MousePressed { x, y } => {
                self.ensure_layout();
                let Some(target) = self.tree.hit_test(point(x, y)) else {
                    return false;
                };
                self.pressed = Some(target);
                self.deliver(target, event)
            }
            InputEvent::MouseReleased { x, y } => {
                self.ensure_layout();
                // A release routes to the pressed widget even when the cursor
                // left every rect (stale release reaches nobody otherwise).
                let Some(target) = self.pressed.take().or_else(|| self.tree.hit_test(point(x, y))) else {
                    return false;
                };
                self.deliver(target, event)
            }
            InputEvent::MouseMoved { x, y } => {
                self.ensure_layout();
                let target = self.tree.hit_test(point(x, y));
                let mut scheduled = match target {
                    Some(target) => self.deliver(target, event),
                    None => false,
                };
                if self.hovered != target {
                    // The previously hovered widget must learn the cursor
                    // left (it resets its visual and marks damage).
                    if let Some(previous) = self.hovered {
                        scheduled |= self.deliver(previous, event);
                    }
                    self.hovered = target;
                }
                scheduled
            }
        }
    }

    /// Deliver `event` to `target` with its post-layout rect; mark damage on
    /// visual change, count clicks.
    fn deliver(&mut self, target: WidgetId, event: &InputEvent) -> bool {
        let rect = self.tree.rect_of(target).expect("layout ran before routing");
        let feedback = self.tree.widget_mut(target).handle_input(event, rect);
        if feedback.clicked {
            self.clicks += 1;
        }
        match feedback.damage {
            true => {
                self.damage.mark(rect);
                true
            }
            false => false,
        }
    }

    /// Layout at most once per cycle: skipped when clean; a relayout whose
    /// rects actually changed invalidates everything.
    fn ensure_layout(&mut self) {
        if self.tree.layout(self.width, self.height) {
            self.damage.mark_all(self.width, self.height);
        }
    }

    /// Paint when dirty: layout first (at most once per cycle), then the
    /// widgets emit front to back into the vello scene and the CPU
    /// rasterizer turns it into the frame. Returns the frame, or `None` when
    /// nothing is dirty. (Full-frame repaint; the coalesced damage rect
    /// gates it.)
    pub fn paint(&mut self) -> Option<&[u32]> {
        self.ensure_layout();
        self.damage.take()?;
        self.painter.begin_frame();
        self.tree.paint(&mut self.painter);
        if !self.label.is_empty() {
            let origin = Point::new(8.0, f64::from(self.height) - 24.0);
            self.painter.draw_label(&self.label, origin, LABEL_COLOR);
        }
        self.painter.render_into(&mut self.frame);
        Some(&self.frame)
    }

    /// The last painted frame in softbuffer pixel layout (the presenter ships
    /// this across the thread boundary); zeros until the first paint.
    pub fn frame(&self) -> &[u32] {
        &self.frame
    }

    /// Whether any region is dirty (test seam: inspect the damage state).
    pub fn is_dirty(&self) -> bool {
        self.damage.is_dirty()
    }

    /// Completed press-release clicks across all widgets.
    pub fn click_count(&self) -> u32 {
        self.clicks
    }
}

fn point(x: i32, y: i32) -> Point {
    Point::new(f64::from(x), f64::from(y))
}
