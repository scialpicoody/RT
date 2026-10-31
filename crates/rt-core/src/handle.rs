//! Generational identity (ADR 0004): unforgeable, staleness-detecting
//! references to objects and connections.

/// An object's identity inside a [`Hub`](crate::signal::Hub); detects staleness
/// after destruction via its generation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Handle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl Handle {
    pub fn index(self) -> u32 {
        self.index
    }
}

/// A connection's identity; invalidated by disconnect or destruction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ConnectionId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}
