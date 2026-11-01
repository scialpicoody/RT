//! The widget-root orchestrator: owns the frame buffer, the vello paint
//! engine ([`Painter`]), and the widget tree (one [`Button`] for now); routes
//! [`InputEvent`]s into widgets, and paints coalesced damage into the frame
//! at one point — `paint`.
//!
//! SEAM: with rt-core objects in the loop, input dispatch becomes typed
//! signal emissions and `paint` is driven from the loop's Flush step; the
//! widget-facing contract below stays the same.

use vello_cpu::kurbo::{Point, Rect};

use crate::damage::Damage;
use crate::input::InputEvent;
use crate::paint::{LABEL_COLOR, Painter};
use crate::widget::Button;

/// One window's widget root and frame, in softbuffer pixel layout.
pub struct Gui {
    width: u32,
    height: u32,
    frame: Vec<u32>,
    painter: Painter,
    button: Button,
    label: String,
    damage: Damage,
    clicks: u32,
}

impl Gui {
    pub fn new(width: u32, height: u32) -> Self {
        let painter = Painter::new(width, height);
        let button = Button::new(
            Rect::new(40.0, 40.0, 200.0, 88.0),
            "Press me",
        );
        let mut damage = Damage::new();
        damage.mark_all(width, height);
        Self {
            width,
            height,
            frame: vec![0; (width * height) as usize],
            painter,
            button,
            label: String::new(),
            damage,
            clicks: 0,
        }
    }

    /// Status line rendered under the button; setting it invalidates
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
                self.damage.mark_all(width, height);
                true
            }
            InputEvent::CloseRequested => false,
            _ => {
                let outcome = self.button.handle(event);
                if outcome.clicked {
                    self.clicks += 1;
                }
                match outcome.damage {
                    Some(rect) => {
                        self.damage.mark(rect);
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Paint when dirty: the widgets emit into the vello scene and the CPU
    /// rasterizer turns it into the frame. Returns the frame, or `None` when
    /// nothing is dirty. (Full-frame repaint; the coalesced damage rect
    /// gates it.)
    pub fn paint(&mut self) -> Option<&[u32]> {
        self.damage.take()?;
        self.painter.begin_frame();
        self.button.render(&mut self.painter);
        if !self.label.is_empty() {
            let b = self.button.rect();
            let origin = Point::new(b.x0, b.y1 + 4.0);
            self.painter.draw_label(&self.label, origin, LABEL_COLOR);
        }
        self.painter.render_into(&mut self.frame);
        Some(&self.frame)
    }

    /// Whether any region is dirty (test seam: inspect the damage state).
    pub fn is_dirty(&self) -> bool {
        self.damage.is_dirty()
    }

    /// The root button's rect (test seam).
    pub fn button_rect(&self) -> Rect {
        self.button.rect()
    }

    /// Completed press-release clicks on the root button.
    pub fn click_count(&self) -> u32 {
        self.clicks
    }
}
