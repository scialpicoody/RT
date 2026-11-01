//! Bindings (ticket #30): reactive properties evaluated in the Flush step —
//! seed evaluation at `bind_property`, one re-evaluation per flush when a
//! dependency changed, insertion-order determinism, latest-value coalescing,
//! cycle rejection, audit traces, destroy cleanup.

use rt_core::{Handle, Hub, Object, Property, TraceEvent, object_impl};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Object)]
struct Cell {
    #[property]
    value: Property<i32>,
}
#[object_impl]
impl Cell {}

#[derive(Object)]
struct Total {
    #[property]
    total: Property<i32>,
}
#[object_impl]
impl Total {}

/// Observer with a `#[slot]` that records delivered change values.
#[derive(Object)]
struct ChangeLog {
    log: Vec<i32>,
}
#[object_impl]
impl ChangeLog {
    #[slot]
    fn note(&mut self, v: i32) {
        self.log.push(v);
    }
}

fn traced() -> (Hub, Rc<RefCell<Vec<TraceEvent>>>) {
    let mut hub = Hub::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    hub.set_trace(move |event| sink.borrow_mut().push(event));
    (hub, events)
}

/// `total = a + b`, returning the (total, a, b) handles.
fn bind_sum(hub: &mut Hub) -> (Handle, Handle, Handle) {
    let a = hub.insert(Cell {
        value: Property::new(0),
    });
    let b = hub.insert(Cell {
        value: Property::new(0),
    });
    let total = hub.insert(Total {
        total: Property::new(0),
    });
    hub.bind_property( total, "total", move |read| {
        let a: i32 = read.property::<i32>(a, "value");
        let b: i32 = read.property::<i32>(b, "value");
        Box::new(a + b)
    });
    (total, a, b)
}

#[test]
fn seed_evaluation_is_immediate_without_flush() {
    let mut hub = Hub::new();
    let (total, a, b) = {
        let a = hub.insert(Cell {
            value: Property::new(2),
        });
        let b = hub.insert(Cell {
            value: Property::new(40),
        });
        let total = hub.insert(Total {
            total: Property::new(0),
        });
        hub.bind_property( total, "total", move |read| {
            Box::new(read.property::<i32>(a, "value") + read.property::<i32>(b, "value"))
        });
        (total, a, b)
    };
    // The seed evaluation already produced the value — no flush needed for it.
    assert_eq!(hub.get::<Total>(total).unwrap().total(), 42);
    assert_eq!(hub.get::<Cell>(a).unwrap().value(), 2);
    assert_eq!(hub.get::<Cell>(b).unwrap().value(), 40);
}

#[test]
fn reevaluation_delivers_exactly_one_change_per_flush() {
    let (mut hub, _events) = traced();
    let (total, a, b) = bind_sum(&mut hub);

    let observer = hub.insert(ChangeLog { log: Vec::new() });
    hub.connect(total, Total::TOTAL_CHANGED, observer, ChangeLog::note);

    // One dependency changed in this cycle: exactly one delivery, new sum.
    hub.get_mut::<Cell>(a).unwrap().value.set(10);
    hub.flush();
    assert_eq!(hub.get::<ChangeLog>(observer).unwrap().log, [10]);

    // Both dependencies changed in the same cycle: latest-value coalescing
    // keeps it at exactly one delivery carrying the final sum.
    hub.get_mut::<Cell>(a).unwrap().value.set(100);
    hub.get_mut::<Cell>(b).unwrap().value.set(11);
    hub.flush();
    assert_eq!(hub.get::<ChangeLog>(observer).unwrap().log, [10, 111]);

    // No dependency changed: no re-evaluation, no delivery.
    hub.flush();
    assert_eq!(hub.get::<ChangeLog>(observer).unwrap().log, [10, 111]);
}

#[test]
fn insertion_order_chains_converge_in_one_flush() {
    let mut hub = Hub::new();
    let src = hub.insert(Cell {
        value: Property::new(1),
    });
    let mid = hub.insert(Total {
        total: Property::new(0),
    });
    let out = hub.insert(Total {
        total: Property::new(0),
    });

    // Declared in order: `mid` first, then `out` reading `mid`. Insertion
    // order guarantees `out` sees `mid`'s NEW value in the same flush.
    // (Reverse declaration order would converge one flush later — the second
    // binding would consume `mid`'s change on the next flush. Deterministic
    // either way, but insertion order gives same-flush convergence.)
    hub.bind_property( mid, "total", move |read| {
        Box::new(read.property::<i32>(src, "value") * 2)
    });
    hub.bind_property( out, "total", move |read| {
        Box::new(read.property::<i32>(mid, "total") + 1)
    });

    assert_eq!(hub.get::<Total>(mid).unwrap().total(), 2);
    assert_eq!(hub.get::<Total>(out).unwrap().total(), 3);

    hub.get_mut::<Cell>(src).unwrap().value.set(5);
    hub.flush();
    assert_eq!(hub.get::<Total>(mid).unwrap().total(), 10);
    assert_eq!(hub.get::<Total>(out).unwrap().total(), 11);
}

#[test]
#[should_panic(expected = "rt: binding cycle through total")]
fn self_reading_binding_is_rejected() {
    let mut hub = Hub::new();
    let t = hub.insert(Total {
        total: Property::new(0),
    });
    hub.bind_property( t, "total", move |read| {
        Box::new(read.property::<i32>(t, "total") + 1)
    });
}

#[test]
fn audit_traces_binding_events_before_change_delivery() {
    let (mut hub, events) = traced();
    let (total, a, _b) = bind_sum(&mut hub);
    events.borrow_mut().clear();

    hub.get_mut::<Cell>(a).unwrap().value.set(7);
    hub.flush();

    // The BindingEvaluated event precedes the change Delivered event.
    // The property-change path traces Delivered per connection only; with no
    // connections the flush yields exactly the binding event.
    assert_eq!(
        *events.borrow(),
        [TraceEvent::BindingEvaluated {
            target: total,
            property: "total",
        }]
    );
    // No connections, so no Delivered events — connect one and check order.
    events.borrow_mut().clear();
    let observer = hub.insert(Total {
        total: Property::new(0),
    });
    hub.connect(total, Total::TOTAL_CHANGED, observer, |recv: &mut Total, v: i32| {
        recv.total.set(v);
    });
    hub.get_mut::<Cell>(a).unwrap().value.set(8);
    hub.flush();
    let events = events.borrow();
    let binding_pos = events
        .iter()
        .position(|e| matches!(e, TraceEvent::BindingEvaluated { .. }))
        .expect("binding evaluated event");
    let delivered_pos = events
        .iter()
        .position(|e| matches!(e, TraceEvent::Delivered { .. }))
        .expect("delivered event");
    assert_eq!(
        events.iter().filter(|e| matches!(e, TraceEvent::BindingEvaluated { .. })).count(),
        1
    );
    assert!(binding_pos < delivered_pos);
}

#[test]
fn destroyed_target_and_stale_deps_stop_evaluation() {
    let (mut hub, events) = traced();
    let (total, a, b) = bind_sum(&mut hub);
    assert_eq!(hub.get::<Total>(total).unwrap().total(), 0);
    events.borrow_mut().clear(); // drop the seed-evaluation event

    // Destroying a dependency: stale dep reports `is_dirty -> None`, so the
    // binding stops re-evaluating instead of panicking.
    hub.destroy(b);
    hub.get_mut::<Cell>(a).unwrap().value.set(3);
    hub.flush();
    assert_eq!(hub.get::<Total>(total).unwrap().total(), 0);
    assert!(events
        .borrow()
        .iter()
        .all(|e| !matches!(e, TraceEvent::BindingEvaluated { .. })));

    // Destroying the binding target itself: the binding is dropped, so a
    // later flush neither panics nor evaluates anything.
    hub.destroy(total);
    hub.get_mut::<Cell>(a).unwrap().value.set(9);
    hub.flush();
    assert!(events
        .borrow()
        .iter()
        .all(|e| !matches!(e, TraceEvent::BindingEvaluated { .. })));
}
