//! Winit `WindowEvent` → [`InputEvent`] translation.
//!
//! SEAM: this module is the only place that knows winit's input taxonomy.
//! When rt-core typed signals land (ticket #5), widget-facing delivery below
//! becomes typed signal emissions on rt-core objects; nothing outside this
//! module should need to change.

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};

/// Widget-facing input event, in physical pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    MouseMoved { x: i32, y: i32 },
    MousePressed { x: i32, y: i32 },
    MouseReleased { x: i32, y: i32 },
    Resized { width: u32, height: u32 },
    CloseRequested,
}

/// Stateful translator from winit's event stream to [`InputEvent`]s.
///
/// winit reports button presses separately from cursor position, so the
/// translator remembers the last cursor position; a press or release with no
/// known position is dropped.
#[derive(Default)]
pub struct InputTranslator {
    cursor: Option<(i32, i32)>,
}

impl InputTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate one window event, or `None` when it carries no input we act on.
    pub fn translate(&mut self, event: &WindowEvent) -> Option<InputEvent> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                Some(self.cursor_moved(*position))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_input(*state, *button)
            }
            WindowEvent::CloseRequested => Some(InputEvent::CloseRequested),
            WindowEvent::Resized(size) => Some(InputEvent::Resized {
                width: size.width,
                height: size.height,
            }),
            _ => None,
        }
    }

    /// Track cursor movement (physical pixels, truncated toward zero).
    pub fn cursor_moved(&mut self, position: PhysicalPosition<f64>) -> InputEvent {
        let pos = (position.x as i32, position.y as i32);
        self.cursor = Some(pos);
        InputEvent::MouseMoved { x: pos.0, y: pos.1 }
    }

    /// Press/release of a button at the last known cursor position.
    /// Only the left button translates; others are ignored for now.
    pub fn mouse_input(&mut self, state: ElementState, button: MouseButton) -> Option<InputEvent> {
        if button != MouseButton::Left {
            return None;
        }
        self.cursor.map(|(x, y)| match state {
            ElementState::Pressed => InputEvent::MousePressed { x, y },
            ElementState::Released => InputEvent::MouseReleased { x, y },
        })
    }
}
