//! Typed signals/slots (ADR 0003/0004): explicit Hub, generational handles,
//! queued-by-default delivery at flush, direct-call opt-in, destruction and
//! disconnect semantics.

use rt_core::{Handle, Hub, Object, object_impl};

#[derive(Object)]
struct Sender;
#[object_impl]
impl Sender {
    #[signal]
    fn ping(&self, n: u32);
    #[signal]
    fn pong(&self);
}

#[derive(Object)]
struct Receiver {
    log: Vec<&'static str>,
    hits: Vec<u32>,
}
#[object_impl]
impl Receiver {
    #[slot]
    fn on_ping(&mut self, n: u32) {
        self.log.push("ping");
        self.hits.push(n);
    }
    #[slot]
    fn on_pong(&mut self) {
        self.log.push("pong");
    }
}

#[derive(Object)]
struct PairSender;
#[object_impl]
impl PairSender {
    #[signal]
    fn moved(&self, x: i32, y: i32);
}

#[derive(Object)]
struct PairReceiver {
    pos: (i32, i32),
}
#[object_impl]
impl PairReceiver {
    #[slot]
    fn on_moved(&mut self, x: i32, y: i32) {
        self.pos = (x, y);
    }
}

#[test]
fn multi_arg_signal_uses_tuple_payload_and_slot_adapter() {
    let mut hub = Hub::new();
    let s = hub.insert(PairSender);
    let r = hub.insert(PairReceiver { pos: (0, 0) });
    hub.connect(s, PairSender::MOVED, r, PairReceiver::on_moved_slot);

    assert_eq!(hub.emit(s, PairSender::MOVED, (3i32, 4i32)), 1);
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<PairReceiver>(r).unwrap().pos, (3, 4));
}

fn setup() -> (Hub, Handle, Handle, Handle) {
    let mut hub = Hub::new();
    let s = hub.insert(Sender);
    let r1 = hub.insert(Receiver { log: vec![], hits: vec![] });
    let r2 = hub.insert(Receiver { log: vec![], hits: vec![] });
    (hub, s, r1, r2)
}

#[test]
fn emit_queues_and_flush_delivers_in_fifo_order() {
    let (mut hub, s, r1, _r2) = setup();
    hub.connect(s, Sender::PING, r1, Receiver::on_ping);
    hub.connect(s, Sender::PONG, r1, Receiver::on_pong_slot);

    assert_eq!(hub.emit(s, Sender::PING, 7u32), 1);
    assert_eq!(hub.emit(s, Sender::PONG, ()), 1);
    assert_eq!(hub.pending(), 2);
    assert_eq!(hub.flush(), 2);
    assert_eq!(hub.pending(), 0);

    let recv = hub.get::<Receiver>(r1).unwrap();
    assert_eq!(recv.log, ["ping", "pong"]);
    assert_eq!(recv.hits, [7]);
}

#[test]
fn fanout_clones_args_to_each_connection() {
    let (mut hub, s, r1, r2) = setup();
    hub.connect(s, Sender::PING, r1, Receiver::on_ping);
    hub.connect(s, Sender::PING, r2, Receiver::on_ping);

    assert_eq!(hub.emit(s, Sender::PING, 5u32), 2);
    assert_eq!(hub.flush(), 2);
    assert_eq!(hub.get::<Receiver>(r1).unwrap().hits, [5]);
    assert_eq!(hub.get::<Receiver>(r2).unwrap().hits, [5]);
}

#[test]
fn direct_connection_calls_immediately_at_emit() {
    let (mut hub, s, r1, _r2) = setup();
    hub.connect_direct(s, Sender::PING, r1, Receiver::on_ping);

    assert_eq!(hub.emit(s, Sender::PING, 9u32), 1);
    assert_eq!(hub.pending(), 0);
    assert_eq!(hub.get::<Receiver>(r1).unwrap().hits, [9]);
}

#[test]
fn disconnect_stops_delivery() {
    let (mut hub, s, r1, _r2) = setup();
    let conn = hub.connect(s, Sender::PING, r1, Receiver::on_ping);
    assert!(hub.disconnect(conn));
    assert!(!hub.disconnect(conn)); // already gone

    assert_eq!(hub.emit(s, Sender::PING, 1u32), 0);
    assert_eq!(hub.flush(), 0);
    assert!(hub.get::<Receiver>(r1).unwrap().hits.is_empty());
}

#[test]
fn destroy_receiver_drops_connections_and_stale_queue_entries() {
    let (mut hub, s, r1, _r2) = setup();
    hub.connect(s, Sender::PING, r1, Receiver::on_ping);
    hub.emit(s, Sender::PING, 1u32);

    hub.destroy(r1);
    // The queued entry targeting the destroyed receiver is dropped at flush.
    assert_eq!(hub.flush(), 0);
    // The connection died with the receiver.
    assert_eq!(hub.emit(s, Sender::PING, 2u32), 0);
    assert!(hub.get::<Receiver>(r1).is_none());
}

#[test]
fn destroy_emitter_makes_emit_a_noop() {
    let (mut hub, s, r1, _r2) = setup();
    hub.connect(s, Sender::PING, r1, Receiver::on_ping);
    hub.destroy(s);
    assert_eq!(hub.emit(s, Sender::PING, 1u32), 0);
    assert_eq!(hub.flush(), 0);
}

#[test]
fn slot_index_reuse_detects_stale_handles() {
    let (mut hub, s, r1, _r2) = setup();
    hub.destroy(r1);
    // New object reuses the slot index but with a fresh generation.
    let r1b = hub.insert(Receiver { log: vec![], hits: vec![] });
    assert_ne!(Handle::index(r1), 0xFFFF_FFFF); // sanity: indexes exist
    assert!(hub.get::<Receiver>(r1).is_none());
    assert!(hub.get::<Receiver>(r1b).is_some());

    hub.connect(s, Sender::PING, r1b, Receiver::on_ping);
    hub.emit(s, Sender::PING, 3u32);
    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Receiver>(r1b).unwrap().hits, [3]);
}
