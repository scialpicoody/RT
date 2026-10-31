//! Properties (ADR 0003): dirty marking, batched evaluation at flush,
//! deterministic order, change notification via the signal system.

use rt_core::{Handle, Hub, Object, Property, object_impl};

#[derive(Object)]
struct Lamp {
    #[property]
    brightness: Property<u8>,
    #[property]
    label: Property<String>,
}
#[object_impl]
impl Lamp {
    #[signal]
    fn flickered(&self);
}

#[derive(Object)]
struct Panel {
    log: Vec<String>,
}
#[object_impl]
impl Panel {
    #[slot]
    fn note_u8(&mut self, v: u8) {
        self.log.push(format!("u8:{v}"));
    }
    #[slot]
    fn note_string(&mut self, v: String) {
        self.log.push(format!("s:{v}"));
    }
    #[slot]
    fn note_flicker(&mut self) {
        self.log.push("flicker".to_string());
    }
}

fn setup() -> (Hub, Handle, Handle) {
    let mut hub = Hub::new();
    let lamp = hub.insert(Lamp {
        brightness: Property::new(0),
        label: Property::new("desk".to_string()),
    });
    let panel = hub.insert(Panel { log: vec![] });
    (hub, lamp, panel)
}

#[test]
fn set_marks_dirty_and_flush_notifies_observers() {
    let (mut hub, lamp, panel) = setup();
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, panel, Panel::note_u8);

    hub.get_mut::<Lamp>(lamp).unwrap().set_brightness(200);
    assert_eq!(hub.pending(), 0); // nothing synchronous

    assert_eq!(hub.flush(), 1);
    assert_eq!(hub.get::<Panel>(panel).unwrap().log, ["u8:200"]);
}

#[test]
fn batched_sets_coalesce_to_latest_value() {
    let (mut hub, lamp, panel) = setup();
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, panel, Panel::note_u8);

    let lamp_obj = hub.get_mut::<Lamp>(lamp).unwrap();
    lamp_obj.set_brightness(10);
    lamp_obj.set_brightness(20);
    lamp_obj.set_brightness(30);

    assert_eq!(hub.flush(), 1); // one delivery, not three
    assert_eq!(hub.get::<Panel>(panel).unwrap().log, ["u8:30"]);
}

#[test]
fn evaluation_order_is_deterministic() {
    let (mut hub, lamp, panel) = setup();
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, panel, Panel::note_u8);
    hub.connect(lamp, Lamp::LABEL_CHANGED, panel, Panel::note_string);

    // Set in reverse declaration order; delivery follows declaration order.
    let lamp_obj = hub.get_mut::<Lamp>(lamp).unwrap();
    lamp_obj.set_label("bed".to_string());
    lamp_obj.set_brightness(5);

    hub.flush();
    assert_eq!(hub.get::<Panel>(panel).unwrap().log, ["u8:5", "s:bed"]);
}

#[test]
fn observer_detaches_when_destroyed() {
    let (mut hub, lamp, panel) = setup();
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, panel, Panel::note_u8);
    hub.get_mut::<Lamp>(lamp).unwrap().set_brightness(9);

    hub.destroy(panel);
    assert_eq!(hub.flush(), 0);
}

#[test]
fn property_changes_deliver_before_queued_emissions() {
    let (mut hub, lamp, panel) = setup();
    hub.connect(lamp, Lamp::BRIGHTNESS_CHANGED, panel, Panel::note_u8);
    hub.connect(lamp, Lamp::FLICKERED, panel, Panel::note_flicker_slot);

    hub.emit(lamp, Lamp::FLICKERED, ()); // queued first…
    hub.get_mut::<Lamp>(lamp).unwrap().set_brightness(1); // …but properties evaluate first

    hub.flush();
    assert_eq!(hub.get::<Panel>(panel).unwrap().log, ["u8:1", "flicker"]);
}

#[test]
fn setters_without_observers_are_inert() {
    let (mut hub, lamp, _panel) = setup();
    hub.get_mut::<Lamp>(lamp).unwrap().set_brightness(77);
    assert_eq!(hub.flush(), 0);
    assert_eq!(hub.get::<Lamp>(lamp).unwrap().brightness(), 77);
}
