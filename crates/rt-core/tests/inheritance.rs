//! Parent member merging (ticket #32): a derived object's MetaObject exposes
//! the parent's members as its own — merged by-name lookups, composed slot
//! invocation through the parent value field, and inherited property access
//! with the dirty changes remapped into the child's merged index space.
#![cfg(feature = "dynamic-invoke")]

use rt_core::dynamic::DynamicError;
use rt_core::property::PropertyInputs;
use rt_core::{Handle, Hub, Object, Property, RegistryBuilder, object_impl};

/// Observer receiving deliveries and recording what arrived.
#[derive(Object)]
struct Observer {
    log: Vec<String>,
}
#[object_impl]
impl Observer {
    #[slot]
    fn note_i32(&mut self, value: i32) {
        self.log.push(format!("i32:{value}"));
    }
    #[slot]
    fn note_reset(&mut self) {
        self.log.push("reset".to_string());
    }
    #[slot]
    fn note_u8(&mut self, value: u8) {
        self.log.push(format!("u8:{value}"));
    }
}

// ---------------------------------------------------------------------------
// The demoable hierarchy: Base carries a signal, a slot, and a property;
// Child derives from it, holds the parent value field, and adds its own
// signal and property.
// ---------------------------------------------------------------------------

#[derive(Object)]
struct Base {
    #[property]
    value: Property<i32>,
    log: Vec<String>,
}
#[object_impl]
impl Base {
    #[signal]
    fn changed(&self, v: i32);

    #[slot]
    fn reset(&mut self) {
        self.log.push("reset".to_string());
    }

    #[slot]
    fn set_to(&mut self, v: i32) {
        self.value.set(v);
    }

    #[slot]
    fn double(&self, v: i32) -> i32 {
        v * 2
    }
}

#[derive(Object)]
#[object(parent = Base)]
struct Child {
    parent: Base,
    #[property]
    extra: Property<u8>,
}
#[object_impl]
impl Child {
    #[signal]
    fn ticked(&self);
}

#[derive(Object)]
#[object(parent = Child)]
struct Grand {
    parent: Child,
}
#[object_impl]
impl Grand {}

fn registry() -> rt_core::Registry {
    RegistryBuilder::new()
        .register::<Observer>()
        .register::<Base>()
        .register::<Child>()
        .register::<Grand>()
        .build()
}

fn setup() -> (Hub, rt_core::Registry, Handle, Handle, Handle) {
    let mut hub = Hub::new();
    let observer = hub.insert(Observer { log: vec![] });
    let base = hub.insert(Base {
        value: Property::new(5),
        log: vec![],
    });
    let child = hub.insert(Child {
        parent: Base {
            value: Property::new(5),
            log: vec![],
        },
        extra: Property::new(0),
    });
    (hub, registry(), observer, base, child)
}

#[test]
fn merged_meta_lookups_cover_the_chain() {
    let meta = Child::meta();
    // Own members keep their own-space indices.
    assert_eq!(meta.find_signal_merged("ticked").map(|(i, _)| i), Some(0));
    assert_eq!(
        meta.find_property_merged("extra").map(|(i, _)| i),
        Some((meta.signals.len()) as u32)
    );
    // Inherited signal: child's own signal space comes first.
    let (signal_index, signal) = meta
        .find_signal_merged("changed")
        .expect("inherited signal");
    assert_eq!(signal_index, 1);
    assert_eq!(signal.name, "changed");
    // Inherited property: appends after the child's whole signal+property
    // space, positioned after Base's signals (Base's own property indices
    // already sit after its signals).
    let (property_index, property) = meta
        .find_property_merged("value")
        .expect("inherited property");
    assert_eq!(
        property_index,
        (Child::meta().signals.len() + Child::meta().properties.len() + 1) as u32
    );
    assert_eq!(property.name, "value");
    // Inherited slots: descriptors and invokers resolve through the chain,
    // with the owner level reported for receiver routing.
    let (slot, up) = meta.find_slot_merged("reset").expect("inherited slot");
    assert_eq!(slot.name, "reset");
    assert_eq!(up, 1);
    assert!(meta.slot_invoker_merged("reset").is_some());
    assert_eq!(
        meta.find_slot_merged("double").map(|(_, up)| up),
        Some(1)
    );
    assert!(meta.find_signal_merged("absent").is_none());
    assert!(meta.find_property_merged("absent").is_none());
    assert!(meta.find_slot_merged("absent").is_none());
}

#[test]
fn two_level_chains_compose_routes_and_offsets() {
    let mut hub = Hub::new();
    let observer = hub.insert(Observer { log: vec![] });
    let grand = hub.insert(Grand {
        parent: Child {
            parent: Base {
                value: Property::new(1),
                log: vec![],
            },
            extra: Property::new(0),
        },
    });
    let registry = registry();
    // Two levels up: the route climbs child then base.
    hub.dynamic(&registry)
        .invoke_by_name(grand, "reset", Box::new(()))
        .expect("grandparent slot invokes");
    assert_eq!(
        hub.get::<Grand>(grand).unwrap().parent.parent.log,
        ["reset"]
    );
    // Merged offsets accumulate per level: grand(0) + child's signals.
    let (signal_index, _) = Grand::meta()
        .find_signal_merged("changed")
        .expect("inherited through two levels");
    assert_eq!(signal_index, 1);
    let (_, up) = Grand::meta().find_slot_merged("reset").expect("slot");
    assert_eq!(up, 2);
    let _ = observer;
}

#[test]
fn by_name_emit_of_inherited_signal_reaches_slots() {
    let (mut hub, registry, observer, _, child) = setup();
    hub.dynamic(&registry)
        .connect_by_name(child, "changed", observer, "note_i32")
        .expect("inherited signal connects");
    hub.dynamic(&registry)
        .emit_by_name(child, "changed", Box::new(7i32))
        .expect("inherited signal emits");
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Observer>(observer).unwrap().log, ["i32:7"]);
}

#[test]
fn by_name_invoke_of_inherited_slots_operates_on_the_parent_value_field() {
    let (mut hub, registry, _, _, child) = setup();
    // Unit-returning inherited slot: runs on the child's parent value.
    hub.dynamic(&registry)
        .invoke_by_name(child, "reset", Box::new(()))
        .expect("inherited slot invokes");
    assert_eq!(
        hub.get::<Child>(child).unwrap().parent.log,
        ["reset"]
    );
    // Value-returning inherited slot: the erased result comes back.
    let result = hub
        .dynamic(&registry)
        .invoke_by_name(child, "double", Box::new(21i32))
        .expect("inherited slot invokes");
    assert_eq!(*result.unwrap().downcast::<i32>().unwrap(), 42);
    // A mutation slot writes through to the inherited property.
    hub.dynamic(&registry)
        .invoke_by_name(child, "set_to", Box::new(9i32))
        .expect("inherited slot invokes");
    assert_eq!(hub.get::<Child>(child).unwrap().parent.value.get(), 9);
}

#[test]
fn by_name_property_access_serves_inherited_properties() {
    let (mut hub, registry, _, _, child) = setup();
    let value = hub
        .dynamic(&registry)
        .get_property_by_name(child, "value")
        .expect("inherited property reads")
        .expect("present");
    assert_eq!(*value.downcast::<i32>().unwrap(), 5);

    hub.dynamic(&registry)
        .set_property_by_name(child, "value", Box::new(11i32))
        .expect("inherited property writes");
    assert_eq!(hub.get::<Child>(child).unwrap().parent.value.get(), 11);
}

#[test]
fn inherited_dirty_changes_remap_into_the_merged_property_space() {
    let (mut hub, registry, _, _, child) = setup();
    hub.dynamic(&registry)
        .set_property_by_name(child, "value", Box::new(3i32))
        .expect("inherited property writes");
    hub.dynamic(&registry)
        .set_property_by_name(child, "extra", Box::new(2u8))
        .expect("own property writes");
    let merged_property_base =
        (Child::meta().signals.len() + Child::meta().properties.len() + 1) as u32;
    let own_extra_index = Child::meta().signals.len() as u32;
    let child_value = hub.get_mut::<Child>(child).unwrap();
    let mut changes = PropertyInputs::take_dirty_changes(&mut *child_value);
    changes.sort_by_key(|(index, _)| *index);
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].0, own_extra_index);
    assert_eq!(*changes[0].1.clone_any().downcast::<u8>().unwrap(), 2);
    assert_eq!(changes[1].0, merged_property_base);
    assert_eq!(*changes[1].1.clone_any().downcast::<i32>().unwrap(), 3);
}

#[test]
fn unknown_inherited_names_are_errors_not_panics() {
    let (mut hub, registry, _, _, child) = setup();
    let mut d = hub.dynamic(&registry);
    assert_eq!(
        d.emit_by_name(child, "nope", Box::new(())),
        Err(DynamicError::UnknownSignal {
            owner: "Child",
            signal: "nope"
        })
    );
    assert!(matches!(
        d.invoke_by_name(child, "nope", Box::new(())),
        Err(DynamicError::UnknownSlot {
            owner: "Child",
            slot: "nope"
        })
    ));
    assert!(matches!(
        d.get_property_by_name(child, "nope"),
        Err(DynamicError::UnknownProperty {
            owner: "Child",
            property: "nope"
        })
    ));
}

// ---------------------------------------------------------------------------
// Shadowing: a member name redeclared down the chain is a registration
// error, per kind.
// ---------------------------------------------------------------------------

mod shadowing {
    use super::*;

    #[derive(Object)]
    struct ShadowBase {
        #[property]
        value: Property<i32>,
    }
    #[object_impl]
    impl ShadowBase {
        #[signal]
        fn changed(&self, v: i32);
        #[slot]
        fn reset(&mut self) {}
    }

    #[derive(Object)]
    #[object(parent = ShadowBase)]
    struct ShadowSignal {
        parent: ShadowBase,
    }
    #[object_impl]
    impl ShadowSignal {
        #[signal]
        fn changed(&self, v: i32);
    }

    #[derive(Object)]
    #[object(parent = ShadowBase)]
    struct ShadowSlot {
        parent: ShadowBase,
    }
    #[object_impl]
    impl ShadowSlot {
        #[slot]
        fn reset(&mut self) {}
    }

    #[derive(Object)]
    #[object(parent = ShadowBase)]
    struct ShadowProperty {
        parent: ShadowBase,
        #[property]
        value: Property<i32>,
    }
    #[object_impl]
    impl ShadowProperty {}

    #[test]
    #[should_panic(expected = "duplicate signal 'changed'")]
    fn redeclared_signal_panics_at_registration() {
        RegistryBuilder::new().register::<ShadowSignal>();
    }

    #[test]
    #[should_panic(expected = "duplicate slot 'reset'")]
    fn redeclared_slot_panics_at_registration() {
        RegistryBuilder::new().register::<ShadowSlot>();
    }

    #[test]
    #[should_panic(expected = "duplicate property 'value'")]
    fn redeclared_property_panics_at_registration() {
        RegistryBuilder::new().register::<ShadowProperty>();
    }
}

// The typed path stays own-members by design: SignalId consts and the
// `NAME_CHANGED` change tokens index the owner type's own signal space;
// inherited members are reachable by name (this file) and by the typed API
// on the owner type itself.
#[test]
fn typed_emit_still_uses_own_signal_indices() {
    let (mut hub, _registry, observer, base, child) = setup();
    hub.connect(base, Base::VALUE_CHANGED, observer, Observer::note_i32);
    hub.connect(child, Child::EXTRA_CHANGED, observer, Observer::note_u8);
    hub.emit(base, Base::VALUE_CHANGED, 4i32);
    hub.emit(child, Child::EXTRA_CHANGED, 1u8);
    assert_eq!(hub.flush(), 2);
    assert_eq!(hub.get::<Observer>(observer).unwrap().log.len(), 2);
}
