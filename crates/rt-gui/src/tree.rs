//! The owned widget tree: a small [`Widget`] trait, an arena of owned nodes
//! (ids + tree lookup, no shared ownership — the Handle discipline extended
//! to widgets), flex containers ([`Row`]/[`Column`]), and taffy-powered
//! layout with post-layout geometry exposed via [`WidgetTree::rect_of`].
//!
//! Layout determinism: a relayout runs at most once per cycle (a dirty flag
//! set by structural change or resize), ordered before painting and before
//! mouse input routing; when the resolved rects actually change, the caller
//! invalidates damage conservatively (full repaint).

use taffy::prelude::{Dimension, Display, FlexDirection, Style};
use vello_cpu::kurbo::{Point, Rect};

use crate::input::InputEvent;
use crate::paint::Painter;

/// Generalized widget feedback: the [`crate::ButtonOutcome`] pattern with the
/// concrete damage rect abstracted away (the tree knows the widget's rect).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WidgetFeedback {
    /// The widget's visual state changed (repaint needed).
    pub damage: bool,
    /// Press followed by release inside the widget.
    pub clicked: bool,
}

impl WidgetFeedback {
    /// No visual change, no click.
    pub fn clean() -> Self {
        Self::default()
    }
}

/// Copyable identity of one node in the [`WidgetTree`]. Not a Handle into a
/// registry: a tree id dies with the tree, so staleness is impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(pub usize);

/// The widget contract: input handling, painting, and the flex style the
/// layout engine positions the node by. Containers implement the same trait
/// (composite pattern) and are laid out and hit-tested like leaves.
pub trait Widget {
    /// Feed one input event; `rect` is this node's resolved post-layout rect.
    fn handle_input(&mut self, input: &InputEvent, rect: Rect) -> WidgetFeedback;

    /// Paint the node into `rect` (its resolved post-layout rect).
    fn paint(&self, painter: &mut Painter, rect: Rect);

    /// The taffy flex style describing how this node is sized and placed.
    fn style(&self) -> Style;

    /// Adopt the resolved post-layout rect. Default: nothing (containers have
    /// no own visuals); leaves with internal state sync it here.
    fn sync_rect(&mut self, _rect: Rect) {}
}

/// A vertical flex container (children stacked top to bottom).
#[derive(Debug, Default)]
pub struct Column {
    /// Space between adjacent children.
    pub gap: f32,
    /// Padding on all four edges.
    pub padding: f32,
    /// Cross-axis alignment of children.
    pub align_items: taffy::AlignItems,
    /// Main-axis placement of the child run.
    pub justify_content: taffy::JustifyContent,
}

impl Column {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A horizontal flex container (children laid left to right).
#[derive(Debug, Default)]
pub struct Row {
    pub gap: f32,
    pub padding: f32,
    pub align_items: taffy::AlignItems,
    pub justify_content: taffy::JustifyContent,
}

impl Row {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Shared container style construction: fills the available space, stacks
/// children along the container's axis with gap and padding applied.
fn container_style(direction: FlexDirection, c: &(impl ColumnProps + ?Sized)) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: direction,
        size: Size2D::percent_full(),
        gap: Size2D::uniform_gap(c.gap()),
        padding: Pad::uniform(c.padding()),
        align_items: c.align_items(),
        justify_content: c.justify_content(),
        ..Style::default()
    }
}

trait ColumnProps {
    fn gap(&self) -> f32;
    fn padding(&self) -> f32;
    fn align_items(&self) -> taffy::AlignItems;
    fn justify_content(&self) -> taffy::JustifyContent;
}

impl ColumnProps for Column {
    fn gap(&self) -> f32 { self.gap }
    fn padding(&self) -> f32 { self.padding }
    fn align_items(&self) -> taffy::AlignItems { self.align_items }
    fn justify_content(&self) -> taffy::JustifyContent { self.justify_content }
}

impl ColumnProps for Row {
    fn gap(&self) -> f32 { self.gap }
    fn padding(&self) -> f32 { self.padding }
    fn align_items(&self) -> taffy::AlignItems { self.align_items }
    fn justify_content(&self) -> taffy::JustifyContent { self.justify_content }
}

// Small constructors keeping the style literal above readable.
struct Size2D;
impl Size2D {
    fn percent_full() -> taffy::Size<Dimension> {
        taffy::Size {
            width: Dimension::percent(1.0),
            height: Dimension::percent(1.0),
        }
    }
    fn uniform_gap(value: f32) -> taffy::Size<taffy::LengthPercentage> {
        let gap: taffy::LengthPercentage = length(value);
        taffy::Size { width: gap, height: gap }
    }
}
struct Pad;
impl Pad {
    fn uniform(value: f32) -> taffy::Rect<taffy::LengthPercentage> {
        let pad: taffy::LengthPercentage = length(value);
        taffy::Rect { left: pad, right: pad, top: pad, bottom: pad }
    }
}

fn length(value: f32) -> taffy::LengthPercentage {
    taffy::style_helpers::length(value)
}

impl Widget for Column {
    fn handle_input(&mut self, _input: &InputEvent, _rect: Rect) -> WidgetFeedback {
        WidgetFeedback::clean()
    }
    fn paint(&self, _painter: &mut Painter, _rect: Rect) {}
    fn style(&self) -> Style {
        container_style(FlexDirection::Column, self)
    }
}

impl Widget for Row {
    fn handle_input(&mut self, _input: &InputEvent, _rect: Rect) -> WidgetFeedback {
        WidgetFeedback::clean()
    }
    fn paint(&self, _painter: &mut Painter, _rect: Rect) {}
    fn style(&self) -> Style {
        container_style(FlexDirection::Row, self)
    }
}

struct WidgetNode {
    widget: Box<dyn Widget>,
    children: Vec<WidgetId>,
    parent: Option<WidgetId>,
}

/// Arena-backed owned widget tree. Children keep insertion order; painting
/// runs front to back, hit testing back to front (last child wins — the same
/// z-order the painter produces).
#[derive(Default)]
pub struct WidgetTree {
    nodes: Vec<WidgetNode>,
    taffy: Option<taffy::TaffyTree>,
    taffy_nodes: Vec<taffy::NodeId>,
    rects: Vec<Option<Rect>>,
    layout_clean: bool,
}

impl WidgetTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `widget` as the root. A tree has exactly one root.
    pub fn set_root(&mut self, widget: impl Widget + 'static) -> WidgetId {
        debug_assert!(self.nodes.is_empty(), "root already set");
        self.push(widget, None)
    }

    pub fn root(&self) -> Option<WidgetId> {
        self.nodes.first().map(|_| WidgetId(0))
    }

    /// Append `widget` as the last child of `parent`.
    pub fn add_child(&mut self, parent: WidgetId, widget: impl Widget + 'static) -> WidgetId {
        let id = self.push(widget, Some(parent));
        self.nodes[parent.0].children.push(id);
        id
    }

    fn push(&mut self, widget: impl Widget + 'static, parent: Option<WidgetId>) -> WidgetId {
        let id = WidgetId(self.nodes.len());
        self.nodes.push(WidgetNode {
            widget: Box::new(widget),
            children: Vec::new(),
            parent,
        });
        self.invalidate_layout();
        id
    }

    pub fn children(&self, id: WidgetId) -> &[WidgetId] {
        &self.nodes[id.0].children
    }

    pub fn parent(&self, id: WidgetId) -> Option<WidgetId> {
        self.nodes[id.0].parent
    }

    /// Read-only widget lookup by id.
    pub fn widget(&self, id: WidgetId) -> &dyn Widget {
        &*self.nodes[id.0].widget
    }

    /// Mutable widget lookup by id.
    pub fn widget_mut(&mut self, id: WidgetId) -> &mut dyn Widget {
        &mut *self.nodes[id.0].widget
    }

    /// The resolved post-layout rect of `id`, or `None` before the first
    /// layout. This is the public geometry accessor.
    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.rects.get(id.0).copied().flatten()
    }

    /// Drop resolved geometry and the taffy mirror; the next [`Self::layout`]
    /// rebuilds everything (structural change or resize).
    pub fn invalidate_layout(&mut self) {
        self.layout_clean = false;
    }

    /// Sync the tree into taffy (rebuild; kept rare — structural changes
    /// only) and compute one layout pass in `width x height`.
    ///
    /// Returns `true` when any resolved rect actually changed (callers
    /// invalidate damage). Sync happens at most once per cycle: a clean
    /// layout is skipped entirely.
    pub fn layout(&mut self, width: u32, height: u32) -> bool {
        if self.layout_clean {
            return false;
        }
        self.sync_taffy();
        if self.rects.len() != self.nodes.len() {
            self.rects = vec![None; self.nodes.len()];
        }
        let taffy_tree = self.taffy.as_mut().expect("sync_taffy built the mirror");
        let root = self.taffy_nodes[0];
        taffy_tree
            .compute_layout(
                root,
                taffy::Size {
                    width: taffy::AvailableSpace::Definite(width as f32),
                    height: taffy::AvailableSpace::Definite(height as f32),
                },
            )
            .expect("taffy layout on a well-formed tree");

        let mut changed = self.rects.iter().any(Option::is_none);
        let root_id = self.root().expect("tree has a root");
        self.resolve_rects(root_id, Point::ZERO, &mut changed);
        self.layout_clean = true;
        changed
    }

    /// Walk the tree top-down accumulating parent origins: taffy reports each
    /// node's location relative to its parent; the public geometry is
    /// absolute (frame coordinates).
    fn resolve_rects(&mut self, id: WidgetId, origin: Point, changed: &mut bool) {
        let taffy_tree = self.taffy.as_ref().expect("taffy mirror");
        let laid = taffy_tree
            .layout(self.taffy_nodes[id.0])
            .expect("node exists");
        let rect = Rect::new(
            origin.x + f64::from(laid.location.x),
            origin.y + f64::from(laid.location.y),
            origin.x + f64::from(laid.location.x + laid.size.width),
            origin.y + f64::from(laid.location.y + laid.size.height),
        );
        let previous = self.rects[id.0].replace(rect);
        *changed |= previous != Some(rect);
        if previous != Some(rect) {
            self.nodes[id.0].widget.sync_rect(rect);
        }
        let child_origin = Point::new(rect.x0, rect.y0);
        let children = self.nodes[id.0].children.clone();
        for child in children {
            self.resolve_rects(child, child_origin, changed);
        }
    }

    fn sync_taffy(&mut self) {
        let mut taffy_tree = taffy::TaffyTree::new();
        self.taffy_nodes.clear();
        // Two passes: nodes first (children reference parents by id), then
        // the child links in insertion order.
        for node in &self.nodes {
            let style = node.widget.style();
            let taffy_node = taffy_tree
                .new_leaf(style)
                .expect("fresh leaf");
            self.taffy_nodes.push(taffy_node);
        }
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.children.is_empty() {
                let children: Vec<taffy::NodeId> = node
                    .children
                    .iter()
                    .map(|child| self.taffy_nodes[child.0])
                    .collect();
                taffy_tree
                    .set_children(self.taffy_nodes[index], &children)
                    .expect("nodes exist");
            }
        }
        self.taffy = Some(taffy_tree);
    }

    /// Paint the whole tree front to back (parents before children).
    pub fn paint(&mut self, painter: &mut Painter) {
        self.paint_subtree(self.root().expect("tree has a root"), painter);
    }

    fn paint_subtree(&self, id: WidgetId, painter: &mut Painter) {
        let rect = self.rect_of(id).expect("painting after layout");
        self.nodes[id.0].widget.paint(painter, rect);
        for &child in &self.nodes[id.0].children {
            self.paint_subtree(child, painter);
        }
    }

    /// Deepest node whose resolved rect contains `point`, scanning children
    /// back to front (last child wins). Containers match too when no child
    /// covers the point.
    pub fn hit_test(&self, point: Point) -> Option<WidgetId> {
        let root = self.root()?;
        self.hit_in_subtree(root, point)
    }

    fn hit_in_subtree(&self, id: WidgetId, point: Point) -> Option<WidgetId> {
        let rect = self.rect_of(id)?;
        if !rect.contains(point) {
            return None;
        }
        for &child in self.nodes[id.0].children.iter().rev() {
            if let Some(hit) = self.hit_in_subtree(child, point) {
                return Some(hit);
            }
        }
        Some(id)
    }
}
