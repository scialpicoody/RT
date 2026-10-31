//! The widget-root orchestrator: owns the frame buffer and the widget tree
//! (one [`Button`] for now), routes [`InputEvent`]s into widgets, and paints
//! coalesced damage into the frame at one point — `paint`.
//!
//! SEAM: with rt-core objects in the loop, input dispatch becomes typed
//! signal emissions and `paint` is driven from the loop's Flush step; the
//! widget-facing contract below stays the same.

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::primitives::Rectangle;

use crate::damage::Damage;
use crate::input::InputEvent;
use crate::pixel::BufferTarget;
use crate::widget::Button;

/// One window's widget root and frame, in softbuffer pixel layout.
#[derive(Debug)]
pub struct Gui {
    size: Size,
    frame: Vec<u32>,
    button: Button,
    damage: Damage,
    clicks: u32,
}

impl Gui {
    pub fn new(width: u32, height: u32) -> Self {
        let size = Size::new(width, height);
        let button = Button::new(
            Rectangle::new(Point::new(40, 40), Size::new(160, 48)),
            "Press me",
        );
        let mut damage = Damage::new();
        damage.mark_all(size);
        Self {
            size,
            frame: vec![0; (width * height) as usize],
            button,
            damage,
            clicks: 0,
        }
    }

    /// Feed one translated input event. Returns `true` when it scheduled new
    /// damage (repaint needed); already-scheduled damage coalesces to `false`.
    pub fn handle_input(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Resized { width, height } => {
                self.size = Size::new(width, height);
                self.frame = vec![0; (width * height) as usize];
                self.damage.mark_all(self.size);
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

    /// Paint when dirty: clears to the background and renders the widgets
    /// into the frame. Returns the frame, or `None` when nothing is dirty.
    /// (Full-frame repaint; the coalesced damage rect gates it.)
    pub fn paint(&mut self) -> Option<&[u32]> {
        self.damage.take()?;
        self.frame.fill(0);
        let size = self.size;
        let mut target = BufferTarget::new(&mut self.frame, size);
        self.button.render(&mut target).expect("infallible target");
        Some(&self.frame)
    }

    /// The root button's rect (test seam).
    pub fn button_rect(&self) -> Rectangle {
        self.button.rect()
    }

    /// Completed press-release clicks on the root button.
    pub fn click_count(&self) -> u32 {
        self.clicks
    }
}
