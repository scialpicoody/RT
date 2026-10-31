//! Proc macros for RT. `#[derive(Object)]` generates a `&'static MetaObject`
//! per type — see ADR 0002 and the wayfinder map. Implementation lands with
//! the metaobject ticket.

#![forbid(unsafe_code)]
