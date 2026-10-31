//! Audit trace (#12): every dispatch lifecycle event is observable through a
//! pluggable callback.

use std::cell::RefCell;
use std::rc::Rc;

use rt_core::{Hub, Object, TraceEvent, object_impl};

#[derive(Object)]
struct Emitter;
#[object_impl]
impl Emitter {
    #[signal]
    fn ping(&self, n: u8);
}

#[derive(Object)]
struct Receiver {
    hits: u8,
}
#[object_impl]
impl Receiver {
    #[slot]
    fn on_ping(&mut self, n: u8) {
        self.hits += n;
    }
}

fn traced() -> (Hub, Rc<RefCell<Vec<TraceEvent>>>) {
    let mut hub = Hub::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    hub.set_trace(move |event| sink.borrow_mut().push(event));
    (hub, events)
}

#[test]
fn full_dispatch_cycle_is_observable() {
    let (mut hub, events) = traced();
    let e = hub.insert(Emitter);
    let r = hub.insert(Receiver { hits: 0 });
    hub.connect(e, Emitter::PING, r, Receiver::on_ping);

    assert_eq!(*events.borrow(), [TraceEvent::Connected { emitter: e, receiver: r, signal_index: 0 }]);

    hub.emit(e, Emitter::PING, 3u8);
    assert_eq!(*events.borrow(), [
        TraceEvent::Connected { emitter: e, receiver: r, signal_index: 0 },
        TraceEvent::Emitted { emitter: e, signal_index: 0, reached: 1 },
    ]);

    hub.flush();
    assert_eq!(*events.borrow(), [
        TraceEvent::Connected { emitter: e, receiver: r, signal_index: 0 },
        TraceEvent::Emitted { emitter: e, signal_index: 0, reached: 1 },
        TraceEvent::Delivered { receiver: r, signal_index: 0 },
    ]);
    assert_eq!(hub.get::<Receiver>(r).unwrap().hits, 3);
}

#[test]
fn disconnect_and_destroy_are_observable() {
    let (mut hub, events) = traced();
    let e = hub.insert(Emitter);
    let r = hub.insert(Receiver { hits: 0 });
    let conn = hub.connect(e, Emitter::PING, r, Receiver::on_ping);
    events.borrow_mut().clear();

    assert!(hub.disconnect(conn));
    assert_eq!(*events.borrow(), [TraceEvent::Disconnected { id: conn }]);

    hub.connect(e, Emitter::PING, r, Receiver::on_ping);
    events.borrow_mut().clear();
    hub.destroy(r);
    assert_eq!(*events.borrow(), [TraceEvent::Destroyed { object: r, connections_removed: 1 }]);
}

#[test]
fn stale_delivery_drop_is_observable() {
    let (mut hub, events) = traced();
    let e = hub.insert(Emitter);
    let r = hub.insert(Receiver { hits: 0 });
    hub.connect(e, Emitter::PING, r, Receiver::on_ping);
    hub.emit(e, Emitter::PING, 1u8);
    events.borrow_mut().clear();

    hub.destroy(r);
    events.borrow_mut().clear();
    assert_eq!(hub.flush(), 0);
    assert_eq!(*events.borrow(), [TraceEvent::DroppedStale { receiver: r, signal_index: 0 }]);
}

#[test]
fn property_change_delivery_is_observable() {
    use rt_core::Property;

    #[derive(Object)]
    struct Lamp {
        #[property]
        brightness: Property<u8>,
    }
    #[object_impl]
    impl Lamp {
        #[signal]
        fn flickered(&self);
    }

    let (mut hub, events) = traced();
    let lamp = hub.insert(Lamp {
        brightness: Property::new(0),
    });
    let r = hub.insert(Receiver { hits: 0 });
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, r, Receiver::on_ping);
    hub.get_mut::<Lamp>(lamp).unwrap().set_brightness(4);
    events.borrow_mut().clear();

    assert_eq!(hub.flush(), 1);
    assert_eq!(*events.borrow(), [TraceEvent::Delivered { receiver: r, signal_index: 1 }]);
}

#[test]
fn clear_trace_stops_events() {
    let (mut hub, events) = traced();
    let e = hub.insert(Emitter);
    hub.clear_trace();
    let r = hub.insert(Receiver { hits: 0 });
    hub.connect(e, Emitter::PING, r, Receiver::on_ping);
    assert!(events.borrow().is_empty());
}
