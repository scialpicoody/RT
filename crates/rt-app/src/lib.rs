//! RT app: the event loop and application shell.
//!
//! Runs the single-threaded core loop: dispatch queued emissions, evaluate
//! dirty properties (the Flush step), deliver posted events and timers.
//! See ADR 0003 and the wayfinder map.

#![forbid(unsafe_code)]
