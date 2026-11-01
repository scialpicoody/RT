//! Button widget: a pressable labeled rect painted into the vello CPU scene
//! (fill, 1px border, centered label). State changes mark damage; a click is
//! reported as an outcome (the seam where a core Signal emission plugs in
//! once ticket #5 lands).

use vello_cpu::kurbo::{Point, Rect};
use vello_cpu::peniko::Color;

use taffy::Style;
use taffy::style_helpers::length;

use crate::input::InputEvent;
use crate::paint::{LABEL_COLOR, LABEL_FONT_SIZE, Painter, measure_label};
use crate::tree::{Widget, WidgetFeedback};

/// Result of feeding one [`InputEvent`] to a widget.
#[derive(Debug, Default, PartialEq)]
pub struct ButtonOutcome {
    /// Region needing repaint, if the visual state changed.
    pub damage: Option<Rect>,
    /// Press followed by release inside the button.
    pub clicked: bool,
}

impl ButtonOutcome {
    fn clean() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visual {
    Normal,
    Hovered,
    Pressed,
}

/// A labeled push button occupying a fixed rectangle (physical pixels).
#[derive(Debug)]
pub struct Button {
    rect: Rect,
    label: String,
    visual: Visual,
}

impl Button {
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            visual: Visual::Normal,
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Adopt the layout-resolved rect (the tree calls this post-layout).
    fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    pub fn contains(&self, point: Point) -> bool {
        self.rect.contains(point)
    }

    /// Current body color (part of the rendering contract).
    pub fn fill_color(&self) -> Color {
        match self.visual {
            Visual::Normal => Color::from_rgba8(0xE0, 0xE0, 0xE0, 0xFF),
            Visual::Hovered => Color::from_rgba8(0xF5, 0xF5, 0xF5, 0xFF),
            Visual::Pressed => Color::from_rgba8(0xB8, 0xB8, 0xB8, 0xFF),
        }
    }

    /// Border color (part of the rendering contract).
    pub fn border_color(&self) -> Color {
        match self.visual {
            Visual::Pressed => Color::from_rgba8(0x30, 0x30, 0x30, 0xFF),
            _ => Color::from_rgba8(0x60, 0x60, 0x60, 0xFF),
        }
    }

    /// Feed one input event; returns damage and whether it counts as a click.
    pub fn handle(&mut self, event: &InputEvent) -> ButtonOutcome {
        match *event {
            InputEvent::MouseMoved { x, y } => {
                let next = if self.contains(Point::new(f64::from(x), f64::from(y))) {
                    Visual::Hovered
                } else {
                    Visual::Normal
                };
                self.set_visual(next)
            }
            InputEvent::MousePressed { x, y } => {
                if self.contains(Point::new(f64::from(x), f64::from(y))) {
                    self.set_visual(Visual::Pressed)
                } else {
                    ButtonOutcome::clean()
                }
            }
            InputEvent::MouseReleased { x, y } => {
                if self.visual != Visual::Pressed {
                    return ButtonOutcome::clean();
                }
                let inside = self.contains(Point::new(f64::from(x), f64::from(y)));
                let outcome = self.set_visual(if inside {
                    Visual::Hovered
                } else {
                    Visual::Normal
                });
                ButtonOutcome {
                    clicked: inside,
                    ..outcome
                }
            }
            InputEvent::Resized { .. } | InputEvent::CloseRequested => ButtonOutcome::clean(),
        }
    }

    fn set_visual(&mut self, next: Visual) -> ButtonOutcome {
        if self.visual == next {
            return ButtonOutcome::clean();
        }
        self.visual = next;
        ButtonOutcome {
            damage: Some(self.rect),
            clicked: false,
        }
    }

    /// Paint the button: fill, 1px border, centered label.
    pub fn render(&self, painter: &mut Painter) {
        self.render_at(painter, self.rect);
    }

    /// Paint into an explicit rect (the tree supplies the layout rect).
    pub fn render_at(&self, painter: &mut Painter, rect: Rect) {
        painter.fill_rect(rect, self.fill_color());
        // 1px border as the four rim pixel rows/columns (pixel-aligned, so
        // rim pixels get the exact border color).
        let (x0, y0, x1, y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
        let border = self.border_color();
        painter.fill_rect(Rect::new(x0, y0, x1, y0 + 1.0), border);
        painter.fill_rect(Rect::new(x0, y1 - 1.0, x1, y1), border);
        painter.fill_rect(Rect::new(x0, y0 + 1.0, x0 + 1.0, y1 - 1.0), border);
        painter.fill_rect(Rect::new(x1 - 1.0, y0 + 1.0, x1, y1 - 1.0), border);

        if !self.label.is_empty() {
            let text_w = measure_label(painter, &self.label);
            let center = rect.center();
            let top_left = Point::new(
                center.x - text_w / 2.0,
                center.y - f64::from(LABEL_FONT_SIZE) / 2.0,
            );
            painter.draw_label(&self.label, top_left, LABEL_COLOR);
        }
    }
}

impl Widget for Button {
    fn handle_input(&mut self, input: &InputEvent, _rect: Rect) -> WidgetFeedback {
        let outcome = self.handle(input);
        WidgetFeedback {
            damage: outcome.damage.is_some(),
            clicked: outcome.clicked,
        }
    }

    fn paint(&self, painter: &mut Painter, rect: Rect) {
        self.render_at(painter, rect);
    }

    fn style(&self) -> taffy::Style {
        taffy::Style {
            size: taffy::Size {
                width: length(200.0),
                height: length(88.0),
            },
            flex_shrink: 0.0,
            ..Style::default()
        }
    }

    fn sync_rect(&mut self, rect: Rect) {
        self.set_rect(rect);
    }
}
