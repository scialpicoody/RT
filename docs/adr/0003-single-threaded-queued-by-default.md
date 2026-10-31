# Single-threaded core; queued-by-default emissions; one deterministic flush

`rt-core` is single-threaded by design; other threads interact only by posting into the loop (explicit marshalling). Slot execution is queued by default: every emission enters one FIFO and is delivered at the next Flush — the same step that evaluates dirty properties — so recursive emit chains are structurally impossible. A direct, immediate call is an explicit per-connection opt-in for the rare latency-critical case.

Rejected: Qt-style direct-by-default with depth budgets (reentrancy bugs); a dual `RefCell`/`Arc<Mutex>` per-object path (two thread models living in one core).
