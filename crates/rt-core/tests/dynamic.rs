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
