//! RT gui: basic window, painting, and widgets.
//!
//! winit 0.30 (window/input) + softbuffer 0.4 (presentation) + the vello
//! family (paint model: `vello_cpu` 0.3 rasterization, `parley` 0.12 labels)
//! per ADR 0007. Input translates through [`input`], damage coalesces in
//! [`damage`], and [`Gui::paint`] is the single repaint point (Flush
//! discipline).

#![forbid(unsafe_code)]

pub mod damage;
pub mod gui;
#[doc(hidden)]
pub mod harness;
pub mod input;
pub mod paint;
pub mod pixel;
pub mod widget;
pub mod window;

pub use damage::Damage;
pub use gui::Gui;
pub use input::{InputEvent, InputTranslator};
pub use paint::Painter;
pub use pixel::{color_to_u32, rgb_to_u32};
pub use widget::{Button, ButtonOutcome};
pub use window::{CloseState, Window, WindowEvent, window_event_from_input};
