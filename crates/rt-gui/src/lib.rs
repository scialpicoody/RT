//! RT gui: basic window, painting, and widgets.
//!
//! Strategy TBD by the GUI research ticket; input arrives as core Signals,
//! painting rides the event loop's Flush step. See the wayfinder map.

#![forbid(unsafe_code)]
