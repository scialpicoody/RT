# Tiered dependency policy

Each crate earns dependencies by role:

- `rt-macros`: `syn`, `quote`, `proc-macro2` only — compile-time, never ships.
- `rt-core`: zero third-party dependencies, std only. This is the auditability invariant; CI should enforce it.
- `rt-app`: std only — `sync::mpsc`, `thread::park`/condvar, `Instant`. No async runtime (tokio et al.): its scheduler is exactly the nondeterministic SOUP the design avoids. `crossbeam-channel` only if std mpsc demonstrably fails.
- `rt-gui`: `winit 0.30` (pinned; 0.31 is beta churn), `softbuffer 0.4`, `embedded-graphics 0.8` per the GUI research (ticket #8); optional `tiny-skia` later behind the same `DrawTarget` seam.

Rejected: egui (232-crate footprint; immediate-mode model clashes with the retained metaobject model); raqote (dormant). Deferred: `log`/`tracing` (until the audit hook needs a facade), `thiserror`, `parking_lot`, `hashbrown` (only if `no_std`).
