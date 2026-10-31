//! RT gui: basic window, painting, and widgets.
//!
//! winit 0.30 (window/input) + softbuffer 0.4 (presentation) +
//! embedded-graphics 0.8 (paint model) per the GUI research ticket (#8).
//! Input translates through [`input`], damage coalesces in [`damage`],
//! and [`Gui::paint`] is the single repaint point (Flush discipline).

#![forbid(unsafe_code)]

pub mod damage;
pub mod gui;
pub mod input;
pub mod pixel;
pub mod widget;

pub use damage::Damage;
pub use gui::Gui;
pub use input::{InputEvent, InputTranslator};
pub use pixel::{BufferTarget, rgb888_to_u32};
pub use widget::{Button, ButtonOutcome};
