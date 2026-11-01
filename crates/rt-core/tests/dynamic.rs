//! By-name path (feature `dynamic-invoke`, ADR 0005): validation, Result
//! errors, traced lookups, coexistence with typed connections.
#![cfg(feature = "dynamic-invoke")]

use std::cell::RefCell;
use std::rc::Rc;

use rt_core::dynamic::DynamicError;
use rt_core::{Handle, Hub, Object, Property, RegistryBuilder, TraceEvent, object_impl};

#[derive(Object)]
struct Knob {
    #[property]
    level: Property<u8>,
}
#[object_impl]
impl Knob {
    #[signal]
    fn turned(&self, steps: u8);
    #[signal]
    fn bumped(&self);
    #[signal]
    fn moved(&self, x: u8, y: u8);
}

#[derive(Object)]
struct Display {
    log: Vec<String>,
}
#[object_impl]
impl Display {
    #[slot]
    fn show(&mut self, steps: u8) {
        self.log.push(format!("steps:{steps}"));
    }
    #[slot]
    fn flash(&mut self) {
        self.log.push("flash".to_string());
    }
    #[slot]
    fn pair(&mut self, a: u8, b: u8) {
        self.log.push(format!("pair:{a},{b}"));
    }
    #[slot]
    fn sum(&self, a: u8, b: u8) -> u8 {
        a.wrapping_add(b)
    }
}

fn registry() -> rt_core::Registry {
    RegistryBuilder::new().register::<Knob>().register::<Display>().build()
}

fn setup() -> (Hub, rt_core::Registry, Handle, Handle) {
    let mut hub = Hub::new();
    let knob = hub.insert(Knob {
        level: Property::new(0),
    });
    let display = hub.insert(Display { log: vec![] });
    (hub, registry(), knob, display)
}

#[test]
fn connect_and_emit_by_name_deliver() {
    let (mut hub, registry, knob, display) = setup();
    hub.dynamic(&registry)
        .connect_by_name(knob, "turned", display, "show")
        .expect("known names");

    assert_eq!(
        hub.dynamic(&registry)
            .emit_by_name(knob, "turned", Box::new(3u8)),
        Ok(1)
    );
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Display>(display).unwrap().log, ["steps:3"]);
}

#[test]
fn by_name_emit_reaches_typed_connections_too() {
    let (mut hub, registry, knob, display) = setup();
    hub.connect(knob, Knob::TURNED, display, Display::show);

    hub.dynamic(&registry)
        .emit_by_name(knob, "turned", Box::new(5u8))
        .expect("same signal");
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Display>(display).unwrap().log, ["steps:5"]);
}

#[test]
fn unit_payload_signal_and_slot() {
    let (mut hub, registry, knob, display) = setup();
    hub.dynamic(&registry)
        .connect_by_name(knob, "bumped", display, "flash")
        .expect("arity 0 both");
    hub.dynamic(&registry)
        .emit_by_name(knob, "bumped", Box::new(()))
        .expect("unit payload");
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Display>(display).unwrap().log, ["flash"]);
}

#[test]
fn unknown_names_are_errors_not_panics() {
    let (mut hub, registry, knob, display) = setup();
    let mut d = hub.dynamic(&registry);
    assert_eq!(
        d.connect_by_name(knob, "nope", display, "show"),
        Err(DynamicError::UnknownSignal { owner: "Knob", signal: "nope" })
    );
    assert_eq!(
        d.connect_by_name(knob, "turned", display, "nope"),
        Err(DynamicError::UnknownSlot { owner: "Display", slot: "nope" })
    );
    assert_eq!(
        d.connect_by_name(knob, "turned", display, "pair"),
        Err(DynamicError::ArityMismatch { owner: "Knob", signal: "turned", slot: "pair" })
    );
}

#[test]
fn payload_mismatch_is_an_error() {
    let (mut hub, registry, knob, display) = setup();
    hub.dynamic(&registry)
        .connect_by_name(knob, "turned", display, "show")
        .expect("connected");
    assert_eq!(
        hub.dynamic(&registry)
            .emit_by_name(knob, "turned", Box::new("wrong" as &str)),
        Err(DynamicError::PayloadMismatch { expected: "u8" })
    );
}

#[test]
fn stale_handles_are_errors() {
    let (mut hub, registry, knob, _display) = setup();
    hub.destroy(knob);
    assert_eq!(
        hub.dynamic(&registry)
            .emit_by_name(knob, "turned", Box::new(1u8)),
        Err(DynamicError::StaleHandle { name: "turned" })
    );
}

#[test]
fn lookups_are_traced() {
    let mut hub = Hub::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    hub.set_trace(move |e| sink.borrow_mut().push(e));
    let registry = registry();
    let knob = hub.insert(Knob {
        level: Property::new(0),
    });
    let display = hub.insert(Display { log: vec![] });

    let _ = hub.dynamic(&registry).connect_by_name(knob, "nope", display, "flash");
    assert_eq!(*events.borrow(), [TraceEvent::DynamicLookup { name: "nope", resolved: false }]);
}

#[test]
fn multi_arg_payload_is_validated_by_name() {
    let (mut hub, registry, knob, display) = setup();
    hub.dynamic(&registry)
        .connect_by_name(knob, "moved", display, "pair")
        .expect("matching arities");

    assert_eq!(
        hub.dynamic(&registry)
            .emit_by_name(knob, "moved", Box::new((1u8, 2u8))),
        Ok(1)
    );
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Display>(display).unwrap().log, ["pair:1,2"]);

    // A wrong payload is rejected ahead of delivery at any arity.
    assert_eq!(
        hub.dynamic(&registry)
            .emit_by_name(knob, "moved", Box::new(7u8)),
        Err(DynamicError::PayloadMismatch {
            expected: "(u8,u8)"
        })
    );
}

#[test]
fn invoke_by_name_returns_the_slot_value() {
    let (mut hub, registry, _knob, display) = setup();

    match hub
        .dynamic(&registry)
        .invoke_by_name(display, "sum", Box::new((2u8, 3u8)))
    {
        Ok(Some(value)) => assert_eq!(*value.downcast::<u8>().unwrap(), 5),
        _ => panic!("expected the erased slot value"),
    }

    // Unit-returning slots hand back None.
    match hub
        .dynamic(&registry)
        .invoke_by_name(display, "flash", Box::new(()))
    {
        Ok(None) => {}
        _ => panic!("expected None for a unit-returning slot"),
    }
}

#[test]
fn invoke_by_name_rejects_bad_targets_and_payloads() {
    let (mut hub, registry, knob, _display) = setup();
    match hub
        .dynamic(&registry)
        .invoke_by_name(knob, "nope", Box::new(()))
    {
        Err(DynamicError::UnknownSlot { owner, slot }) => {
            assert_eq!(owner, "Knob");
            assert_eq!(slot, "nope");
        }
        _ => panic!("expected UnknownSlot"),
    }
    // A slot that exists on another kind is unknown for this owner.
    match hub
        .dynamic(&registry)
        .invoke_by_name(knob, "sum", Box::new(()))
    {
        Err(DynamicError::UnknownSlot { owner, slot }) => {
            assert_eq!(owner, "Knob");
            assert_eq!(slot, "sum");
        }
        _ => panic!("expected UnknownSlot"),
    }

    let (mut hub, registry, _knob, display) = setup();
    match hub
        .dynamic(&registry)
        .invoke_by_name(display, "sum", Box::new(7u8))
    {
        Err(DynamicError::PayloadMismatch { expected }) => {
            assert_eq!(expected, "(u8,u8)");
        }
        _ => panic!("expected PayloadMismatch"),
    }

    // Stale handle: the object was destroyed in this hub.
    let (mut hub, registry, _knob, display) = setup();
    hub.destroy(display);
    match hub
        .dynamic(&registry)
        .invoke_by_name(display, "sum", Box::new((1u8, 1u8)))
    {
        Err(DynamicError::StaleHandle { name }) => assert_eq!(name, "sum"),
        _ => panic!("expected StaleHandle"),
    }
}

#[test]
fn properties_are_readable_and_writable_by_name() {
    let (mut hub, registry, knob, display) = setup();
    // Typed observer for the change token: delivered by flush after the
    // by-name write, exactly like a typed set.
    hub.connect(knob, Knob::LEVEL_CHANGED, display, Display::show);

    match hub.dynamic(&registry).get_property_by_name(knob, "level") {
        Ok(Some(value)) => assert_eq!(*value.downcast::<u8>().unwrap(), 0),
        _ => panic!("expected the erased level"),
    }

    hub.dynamic(&registry)
        .set_property_by_name(knob, "level", Box::new(5u8))
        .expect("known property");
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Knob>(knob).unwrap().level.get(), 5);
    assert_eq!(hub.get::<Display>(display).unwrap().log, ["steps:5"]);
}

#[test]
fn property_by_name_rejects_unknown_names_and_payloads() {
    let (mut hub, registry, knob, display) = setup();

    match hub.dynamic(&registry).get_property_by_name(knob, "nope") {
        Err(DynamicError::UnknownProperty { owner, property }) => {
            assert_eq!(owner, "Knob");
            assert_eq!(property, "nope");
        }
        _ => panic!("expected UnknownProperty"),
    }
    // A property that exists on another kind is unknown for this owner.
    match hub
        .dynamic(&registry)
        .set_property_by_name(display, "level", Box::new(1u8))
    {
        Err(DynamicError::UnknownProperty { owner, property }) => {
            assert_eq!(owner, "Display");
            assert_eq!(property, "level");
        }
        _ => panic!("expected UnknownProperty"),
    }
    // Wrong payload type is rejected ahead of the write.
    match hub
        .dynamic(&registry)
        .set_property_by_name(knob, "level", Box::new(1i32))
    {
        Err(DynamicError::PayloadMismatch { expected }) => assert_eq!(expected, "u8"),
        _ => panic!("expected PayloadMismatch"),
    }

    // Stale handle.
    let (mut hub, registry, _knob, display) = setup();
    hub.destroy(display);
    match hub.dynamic(&registry).get_property_by_name(display, "level") {
        Err(DynamicError::StaleHandle { name }) => assert_eq!(name, "level"),
        _ => panic!("expected StaleHandle"),
    }
}
