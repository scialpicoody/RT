//! Audit trace (map ticket #12): observable dispatch lifecycle events via a
//! pluggable callback — no log-crate dependency, zero third-party.

use crate::handle::{ConnectionId, Handle};

/// One observable dispatch lifecycle event. Indexes refer to a signal's
/// position in its MetaObject (declared signals first, then properties).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceEvent {
    Connected {
        emitter: Handle,
        receiver: Handle,
        signal_index: u32,
    },
    Disconnected {
        id: ConnectionId,
    },
    Destroyed {
        object: Handle,
        connections_removed: usize,
    },
    Emitted {
        emitter: Handle,
        signal_index: u32,
        reached: usize,
    },
    Delivered {
        receiver: Handle,
        signal_index: u32,
    },
    DroppedStale {
        receiver: Handle,
        signal_index: u32,
    },
    /// By-name path (dynamic-invoke feature): a lookup and whether it resolved.
    DynamicLookup {
        name: &'static str,
        resolved: bool,
    },
}
