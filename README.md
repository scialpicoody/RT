# RT

A Qt-style object model runtime in Rust: statically generated metaobjects, typed
signals and slots, stateful properties, and a deterministic event loop. See
`GLOSSARY.md` and `docs/adr/`.

| Crate | Role | Qt analog |
| :--- | :--- | :--- |
| `rt-macros` | Proc macros: `#[derive(Object)]`, `#[signal]`/`#[slot]`/`#[property]` | `moc` |
| `rt-core` | Object model: MetaObjects, Registry, signals/slots, properties | QtCore's object layer |
| `rt-app` | Event loop and application shell | QCoreApplication |
| `rt-gui` | Window, painting, widgets | QtGui/QtWidgets (basic only) |

Status: being charted and built ticket by ticket — see the
[wayfinder map](https://github.com/scialpicoody/RT/issues/1).
