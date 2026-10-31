//! Read-only Registry (ADR 0002): filled explicitly at startup, duplicates
//! are hard errors, lookups only afterwards.

use core::any::TypeId;
use std::collections::HashMap;

use crate::meta::MetaObject;
use crate::Object;

/// Mutable fill phase, consumed by [`RegistryBuilder::build`].
///
/// Crates expose a hand-written `fn register_all(builder: &mut RegistryBuilder)`
/// listing their object kinds; `main` orchestrates the calls (a generated form
/// lands with `rt-macros` later). No auto-registration.
#[derive(Default)]
pub struct RegistryBuilder {
    by_type_id: HashMap<TypeId, &'static MetaObject>,
    by_name: HashMap<&'static str, &'static MetaObject>,
}

impl RegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one kind of object; chainable.
    ///
    /// Panics when the same type is registered twice or two distinct types
    /// share a name — hard startup errors by design (ADR 0002), never
    /// last-write-wins.
    pub fn register<T: Object + 'static>(&mut self) -> &mut Self {
        let meta = T::meta();
        if self.by_type_id.contains_key(&TypeId::of::<T>()) {
            panic!("rt-core registry: type '{}' registered twice", meta.type_name);
        }
        if self.by_name.contains_key(meta.type_name) {
            panic!(
                "rt-core registry: duplicate name '{}' from a distinct type",
                meta.type_name
            );
        }
        self.by_type_id.insert(TypeId::of::<T>(), meta);
        self.by_name.insert(meta.type_name, meta);
        self
    }

    /// Seal into a read-only [`Registry`]; the builder is left drained.
    pub fn build(&mut self) -> Registry {
        Registry {
            by_type_id: std::mem::take(&mut self.by_type_id),
            by_name: std::mem::take(&mut self.by_name),
        }
    }
}

/// The runtime's read-only index from a kind of object to its MetaObject.
pub struct Registry {
    by_type_id: HashMap<TypeId, &'static MetaObject>,
    by_name: HashMap<&'static str, &'static MetaObject>,
}

impl Registry {
    pub fn by_type_id(&self, id: TypeId) -> Option<&'static MetaObject> {
        self.by_type_id.get(&id).copied()
    }

    pub fn by_name(&self, name: &str) -> Option<&'static MetaObject> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}
