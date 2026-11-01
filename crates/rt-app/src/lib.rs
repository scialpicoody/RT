//! RT app: the application shell and its single event loop.
//!
//! One loop model: an [`Application`] driven through the pump — posted
//! messages and timers delivered each cycle, then one Flush step via
//! [`Application::on_cycle`]. Other threads participate only by sending
//! `Send` messages through an [`AppHandle`] (explicit marshalling, ADR 0003).

#![forbid(unsafe_code)]

pub mod app;

pub use app::{App, AppHandle, Application, Ctx, TimerId};
