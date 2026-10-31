# Workspace of three suites mirroring QtCore/QtApp/QtGui

RT is a Cargo workspace: `rt-macros` (proc macros), `rt-core` (object model), `rt-app` (event loop / application shell), `rt-gui` (windowing, painting, widgets). The near-term goal is basic functionality in each, macro-first: wherever metadata or boilerplate can be produced at compile time by `#[derive]`/attribute macros, it is. Rendering, windowing and platform abstraction stay out of `rt-core`.
