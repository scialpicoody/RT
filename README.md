# RT

A Qt-style object model runtime in Rust: statically generated metaobjects, typed
signals and slots, stateful properties, and a deterministic event loop. See
`GLOSSARY.md` and `docs/adr/`.

| Crate | Role | Qt analog |
| :--- | :--- | :--- |
| `rt-macros` | Proc macros: `#[derive(Object)]`, `#[signal]`/`#[slot]`/`#[property]`; `object_impl` generates static MetaObjects plus emit helpers | `moc` |
| `rt-core` | Hub object model: signals/slots queued by default, Property + bindings, inheritance merging, Registry, by-name dynamic path behind a feature, audit trace | QtCore's object layer |
| `rt-app` | Single Application loop: typed messages, timers, self-posts, shutdown, one Flush hook per cycle | QCoreApplication |
| `rt-gui` | vello-family painting, taffy flex widget tree, Window object + Flush seam, `run_window` presenter, optional `gpu` presentation feature | QtGui/QtWidgets (basic only) |

Status: still being built ticket by ticket — a working windowed demo now runs
via `run_window`; see the
[wayfinder map](https://github.com/scialpicoody/RT/issues/1).
