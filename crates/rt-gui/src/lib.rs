//! RT gui: window, painting, widgets, and the widget tree.
//!
//! winit 0.30 (window/input) + softbuffer 0.4 (presentation) + the vello
//! family (paint model: `vello_cpu` 0.3 rasterization, `parley` 0.12 labels)
//! per ADR 0007. Input translates through [`input`], damage coalesces in
//! [`damage`], and [`Gui::paint`] is the single repaint point (Flush
//! discipline). The owned [`tree`] (Widget trait, Row/Column flex
//! containers, taffy 0.15 layout, post-layout hit testing) composes the UI;
//! layout runs at most once per cycle, ordered before painting and before
//! mouse routing.

#![forbid(unsafe_code)]

pub mod damage;
pub mod gui;
#[doc(hidden)]
pub mod harness;
pub mod input;
pub mod paint;
pub mod pixel;
pub mod widget;
pub mod tree;
pub mod window;

pub use damage::Damage;
pub use gui::Gui;
pub use input::{InputEvent, InputTranslator};
pub use paint::Painter;
pub use pixel::{color_to_u32, rgb_to_u32};
pub use tree::{Column, Row, Widget, WidgetFeedback, WidgetId, WidgetTree};
pub use widget::{Button, ButtonOutcome};
pub use window::{CloseState, Window, WindowEvent, window_event_from_input};
