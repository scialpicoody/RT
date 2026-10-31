# Handle-based object identity; no Arc object graphs

Objects are identified by Handles: an id plus a generation counter (ECS-style). Connections and parents reference handles, not pointers or smart pointers; a stale handle is detected, never dangling. Parent–child is non-owning layout metadata, not a memory-lifecycle device (unlike `QObject`'s tree).

Rejected: `Arc`/`Weak` object graphs — without a cycle collector they leak, and they make lifetime reasoning implicit.
