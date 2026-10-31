# Static metaobjects from proc macros; explicit registry; duplicates are hard errors

Qt's `moc` and its mutable runtime metadata are replaced by `#[derive(Object)]` generating a `&'static MetaObject` per type (read-only, link-time constant; signals/slots/properties as descriptors). The runtime Registry is filled once at startup, only through explicit `register_all` calls orchestrated from `main` — no `ctor`/`inventory`/`linkme` auto-registration (hidden control flow, extra SOUP). A duplicate registration — same `TypeId` or same name — is a hard error at startup, not last-write-wins.
