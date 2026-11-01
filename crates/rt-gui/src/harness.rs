//! Headless test-support harness: composes the widget root ([`Gui`]), feeds
//! synthetic [`InputEvent`]s, pumps [`Gui::paint`], and exposes assertions on
//! the painted `u32` frame, the damage state, and widget outcomes — no real
//! window anywhere.
//!
//! TEST SUPPORT ONLY: this module exists so tests can prove behavior parity
//! through one explicit seam. It is not part of the runtime surface and adds
//! no dependencies.

use vello_cpu::kurbo::{Point, Rect};

use crate::gui::Gui;
use crate::input::InputEvent;

/// A windowless [`Gui`] under test with a log of what happened.
///
/// The harness keeps the last painted frame so assertions can inspect pixels
/// after paint, mirroring what presentation would have flushed to a surface.
pub struct Harness {
    gui: Gui,
    width: u32,
    height: u32,
    frame: Vec<u32>,
    painted: bool,
    /// Whether each fed event scheduled a repaint, in feed order.
    schedule_log: Vec<bool>,
}

impl Harness {
    /// Build a fresh window of `width` x `height` physical pixels.
    /// The initial state is fully dirty (first paint covers everything).
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            gui: Gui::new(width, height),
            width,
            height,
            frame: vec![0; (width * height) as usize],
            painted: false,
            schedule_log: Vec::new(),
        }
    }

    /// Feed one input event and record whether it scheduled a repaint.
    pub fn feed(&mut self, event: InputEvent) -> bool {
        if let InputEvent::Resized { width, height } = event {
            self.width = width;
            self.height = height;
        }
        let scheduled = self.gui.handle_input(&event);
        self.schedule_log.push(scheduled);
        scheduled
    }

    /// Feed a sequence of events, returning the schedule flags in order.
    pub fn feed_all(&mut self, events: impl IntoIterator<Item = InputEvent>) -> Vec<bool> {
        events.into_iter().map(|event| self.feed(event)).collect()
    }

    /// Paint when dirty, storing the frame for pixel assertions.
    /// Returns `true` when a repaint happened, `false` when nothing was dirty.
    pub fn paint(&mut self) -> bool {
        match self.gui.paint() {
            Some(frame) => {
                if frame.len() != self.frame.len() {
                    self.frame.resize(frame.len(), 0);
                }
                self.frame.copy_from_slice(frame);
                self.painted = true;
                true
            }
            None => false,
        }
    }

    /// The last painted frame's pixel at (`x`, `y`), row-major.
    ///
    /// Panics when out of bounds or before the first paint.
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        assert!(self.painted, "pixel before the first paint");
        self.frame[(y * self.width + x) as usize]
    }

    /// The last painted frame, row-major in softbuffer pixel layout.
    pub fn frame(&self) -> &[u32] {
        &self.frame
    }

    /// Window width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Window height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether any damage is pending (marked and not yet taken).
    pub fn is_dirty(&self) -> bool {
        self.gui.is_dirty()
    }

    /// Whether no damage is pending (empty after a paint).
    pub fn is_clean(&self) -> bool {
        !self.gui.is_dirty()
    }

    /// Completed press-release clicks on the root button.
    pub fn clicks(&self) -> u32 {
        self.gui.click_count()
    }

    /// The root button's rect, for locating synthetic input.
    pub fn button_rect(&self) -> Rect {
        self.gui.button_rect()
    }

    /// The button's center point, for synthetic inside-the-button input.
    pub fn point_in_button(&self) -> Point {
        self.gui.button_rect().center()
    }

    /// Whether the last fed event scheduled a repaint.
    pub fn last_scheduled(&self) -> Option<bool> {
        self.schedule_log.last().copied()
    }

    /// All schedule flags, in feed order.
    pub fn schedule_log(&self) -> &[bool] {
        &self.schedule_log
    }
}
