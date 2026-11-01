//! Lifecycle cascade (ticket #31): `#[object(children)]` ownership, depth-first
//! destroy with per-object trace, stale-connection behavior across the cascade,
//! and no-direct-connection emission hygiene.

use std::cell::RefCell;
use std::rc::Rc;

use rt_core::{Handle, Hub, Object, TraceEvent, object_impl};

#[derive(Object)]
struct Node {
    #[object(children)]
    kids: Vec<Handle>,
}
#[object_impl]
impl Node {
    #[signal]
    fn ping(&self);
}

#[derive(Object)]
struct Leaf {
    log: Vec<&'static str>,
}
#[object_impl]
impl Leaf {
    #[slot]
    fn on_ping(&mut self) {
        self.log.push("ping");
    }
}

fn traced() -> (Hub, Rc<RefCell<Vec<TraceEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&events);
    let mut hub = Hub::new();
    hub.set_trace(move |event| sink.borrow_mut().push(event));
    (hub, events)
}

#[test]
fn destroy_cascades_depth_first_with_per_object_trace() {
    let (mut hub, events) = traced();
    let parent = hub.insert(Node { kids: Vec::new() });
    let child_a = hub.insert(Node { kids: Vec::new() });
    let child_b = hub.insert(Node { kids: Vec::new() });
    let grandchild = hub.insert(Node { kids: Vec::new() });
    hub.get_mut::<Node>(child_a).unwrap().kids = vec![grandchild];
    hub.get_mut::<Node>(parent).unwrap().kids = vec![child_a, child_b];

    // One receiver per node so each destroyed node has an in-edge.
    let leaf = |hub: &mut Hub| hub.insert(Leaf { log: Vec::new() });
    let (r_a, r_b, r_gc) = (leaf(&mut hub), leaf(&mut hub), leaf(&mut hub));
    hub.connect(parent, Node::PING, r_a, Leaf::on_ping_slot);
    hub.connect(parent, Node::PING, r_b, Leaf::on_ping_slot);
    hub.connect(child_a, Node::PING, r_gc, Leaf::on_ping_slot);

    assert!(hub.destroy(parent));
    let destroyed: Vec<Handle> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Destroyed { object, .. } => Some(*object),
            _ => None,
        })
        .collect();

    // Depth-first, preorder: the parent goes first, then child_a's whole
    // subtree (grandchild included) before the sibling child_b.
    assert_eq!(destroyed, [parent, child_a, grandchild, child_b]);

    // Each object is individually stale, including the deep grandchild.
    for handle in [parent, child_a, child_b, grandchild] {
        assert!(hub.get::<Node>(handle).is_none(), "stale {handle:?}");
    }
    // Three removed connections in total, counted on whichever endpoint
    // destroyed first — one `Destroyed` trace per object.
    let removed: Vec<usize> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Destroyed {
                connections_removed, ..
            } => Some(*connections_removed),
            _ => None,
        })
        .collect();
    assert_eq!(removed.iter().sum::<usize>(), 3);
}

#[test]
fn connection_into_grandchild_drops_stale_after_cascade() {
    let (mut hub, events) = traced();
    let parent = hub.insert(Node { kids: Vec::new() });
    let child = hub.insert(Node { kids: Vec::new() });
    let grandchild = hub.insert(Node { kids: Vec::new() });
    let receiver = hub.insert(Leaf { log: Vec::new() });
    hub.get_mut::<Node>(child).unwrap().kids = vec![grandchild];
    hub.get_mut::<Node>(parent).unwrap().kids = vec![child];

    // Delivery queued BEFORE the destroy, then flushed after: stale drop.
    hub.connect(grandchild, Node::PING, receiver, Leaf::on_ping_slot);
    hub.emit(grandchild, Node::PING, ());
    // Destroy the grandchild's parent — the cascade reaches it.
    assert!(hub.destroy(child));
    assert_eq!(hub.emit(grandchild, Node::PING, ()), 0, "stale emitter");
    hub.flush();
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        TraceEvent::Destroyed { object, .. } if *object == grandchild
    )));

    // A queued delivery to a receiver destroyed by the cascade drops stale.
    let (mut hub2, events2) = traced();
    let sender = hub2.insert(Node { kids: Vec::new() });
    let target = hub2.insert(Leaf { log: Vec::new() });
    hub2.connect(sender, Node::PING, target, Leaf::on_ping_slot);
    hub2.emit(sender, Node::PING, ()); // queued before destroy
    assert!(hub2.destroy(target));
    hub2.flush();
    assert!(
        events2
            .borrow()
            .iter()
            .any(|e| matches!(e, TraceEvent::DroppedStale { .. })),
        "queued delivery to a destroyed receiver drops stale"
    );
}

#[test]
fn survivors_still_deliver_after_a_cascade() {
    let mut hub = Hub::new();
    let parent = hub.insert(Node { kids: Vec::new() });
    let child = hub.insert(Node { kids: Vec::new() });
    let doomed = hub.insert(Leaf { log: Vec::new() });
    let survivor = hub.insert(Leaf { log: Vec::new() });
    hub.get_mut::<Node>(parent).unwrap().kids = vec![child];

    // child → doomed dies with the cascade; parent's connections to both
    // leaves are untouched by it. Direct and queued mixes on both paths.
    hub.connect(child, Node::PING, doomed, Leaf::on_ping_slot);
    hub.connect(parent, Node::PING, doomed, Leaf::on_ping_slot);
    hub.connect(parent, Node::PING, survivor, Leaf::on_ping_slot);
    hub.connect_direct(parent, Node::PING, survivor, Leaf::on_ping_slot);

    // Queued from the doomed subtree while it is still alive.
    hub.emit(child, Node::PING, ());
    assert!(hub.destroy(child));
    // Emitting from the destroyed child reaches nothing.
    assert_eq!(hub.emit(child, Node::PING, ()), 0, "destroyed emitter");
    // Three parent edges remain: doomed queued, survivor queued + direct.
    // The cascade removed only the child → doomed edge.
    assert_eq!(hub.emit(parent, Node::PING, ()), 3);
    assert_eq!(hub.get::<Leaf>(survivor).unwrap().log, ["ping"]);
    hub.flush();
    // doomed: one queued delivery from before the destroy, one from the
    // surviving parent edge — nothing from the dead child afterwards.
    assert_eq!(hub.get::<Leaf>(doomed).unwrap().log, ["ping", "ping"]);
    assert_eq!(hub.get::<Leaf>(survivor).unwrap().log, ["ping", "ping"]);
}

#[test]
fn destroying_twice_and_stale_handles_report_false() {
    let mut hub = Hub::new();
    let node = hub.insert(Node { kids: Vec::new() });
    assert!(hub.destroy(node));
    assert!(!hub.destroy(node), "already destroyed");
    let ghost = hub.insert(Node { kids: Vec::new() });
    assert!(hub.destroy(ghost));
    assert!(!hub.destroy(ghost), "stale generation reports false");
}
