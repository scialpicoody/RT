//! Acceptance demo (#10), headless leg: the full stack chain —
//! registry → typed signal → slot → Property → change token → repaint —
//! asserted end to end, with the audit trace as transcript. The by-name
//! shadow connection runs when `dynamic-invoke` is enabled. The window leg
//! (real rt-app loop + winit bridge) lives in `examples/acceptance.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rt_core::{
    Hub, Object, Property, Registry, RegistryBuilder, TraceEvent, object_impl,
};
use rt_gui::{Gui, InputEvent};

#[derive(Object)]
struct Panel;
#[object_impl]
impl Panel {
    #[signal]
    fn button_clicked(&self);
}

#[derive(Object)]
struct Counter {
    #[property]
    count: Property<u32>,
}
#[object_impl]
impl Counter {
    #[slot]
    fn bump(&mut self) {
        self.count.set(self.count() + 1);
    }
}

#[derive(Object)]
struct Bridge {
    label: String,
}
#[object_impl]
impl Bridge {
    #[slot]
    fn refresh(&mut self, new_count: u32) {
        self.label = format!("clicks: {new_count}");
    }
}

#[cfg(feature = "dynamic-invoke")]
#[derive(Object)]
struct Recorder {
    ticks: u32,
}
#[cfg(feature = "dynamic-invoke")]
#[object_impl]
impl Recorder {
    #[slot]
    fn ticked(&mut self) {
        self.ticks += 1;
    }
}

fn click_inside(gui: &Gui) -> Vec<InputEvent> {
    let b = gui.button_rect();
    let (x, y) = (b.top_left.x + 5, b.top_left.y + 5);
    vec![
        InputEvent::MouseMoved { x, y },
        InputEvent::MousePressed { x, y },
        InputEvent::MouseReleased { x, y },
    ]
}

#[test]
fn full_stack_chain_registry_to_repaint() {
    let sink = Rc::new(RefCell::new(Vec::<TraceEvent>::new()));
    let mut hub = Hub::new();
    hub.set_trace({
        let sink = sink.clone();
        move |e| sink.borrow_mut().push(e)
    });

    // Explicit registry fill at startup (ADR 0002).
    let mut builder = RegistryBuilder::new();
    builder.register::<Panel>().register::<Counter>().register::<Bridge>();
    #[cfg(feature = "dynamic-invoke")]
    builder.register::<Recorder>();
    let registry: Registry = builder.build();

    let panel = hub.insert(Panel);
    let counter = hub.insert(Counter {
        count: Property::new(0),
    });
    let bridge = hub.insert(Bridge {
        label: String::new(),
    });
    #[cfg(feature = "dynamic-invoke")]
    let recorder = hub.insert(Recorder { ticks: 0 });

    // Typed connections: click → bump; count change → label refresh.
    hub.connect(panel, Panel::BUTTON_CLICKED, counter, Counter::bump_slot);
    hub.connect(counter, Counter::COUNT_CHANGED, bridge, Bridge::refresh);
    // By-name shadow connection (feature-gated): click → recorder tick.
    #[cfg(feature = "dynamic-invoke")]
    hub.dynamic(&registry)
        .connect_by_name(panel, "button_clicked", recorder, "ticked")
        .expect("names resolve through the registry");

    // GUI leg: a completed press-release inside the button counts as a click.
    let mut gui = Gui::new(320, 240);
    for event in click_inside(&gui) {
        gui.handle_input(&event);
    }
    assert_eq!(gui.click_count(), 1);

    // The controller (here; the rt-app loop in the example) emits the click
    // and flushes to quiescence: flush #1 delivers bump (property goes
    // dirty), flush #2 evaluates the change and delivers refresh.
    hub.emit(panel, Panel::BUTTON_CLICKED, ());
    while hub.flush() > 0 {}

    // Object-model leg: the queued click bumped the property, and the change
    // token reached the bridge.
    assert_eq!(hub.get::<Counter>(counter).unwrap().count(), 1);
    let label = hub.get::<Bridge>(bridge).unwrap().label.clone();
    assert_eq!(label, "clicks: 1");

    // Repaint leg: the bridge label schedules exactly one repaint.
    gui.set_label(&label);
    assert!(gui.paint().is_some());
    assert!(gui.paint().is_none(), "repaint delivered exactly once");

    // By-name parity: the shadow connection delivered too.
    #[cfg(feature = "dynamic-invoke")]
    assert_eq!(hub.get::<Recorder>(recorder).unwrap().ticks, 1);

    // Audit transcript: connect → emit → deliver, property change included.
    let transcript = sink.borrow().clone();
    assert!(transcript.contains(&TraceEvent::Connected {
        emitter: panel,
        receiver: counter,
        signal_index: 0,
    }));
    assert!(transcript.contains(&TraceEvent::Emitted {
        emitter: panel,
        signal_index: 0,
        reached: if cfg!(feature = "dynamic-invoke") { 2 } else { 1 },
    }));
    assert!(transcript.contains(&TraceEvent::Delivered {
        receiver: bridge,
        signal_index: 0,
    }));

    // Handles stay honest after the chain: destroy the counter and a further
    // click reaches nothing, by name or by type.
    hub.destroy(counter);
    #[cfg(feature = "dynamic-invoke")]
    assert_eq!(hub.emit(panel, Panel::BUTTON_CLICKED, ()), 1, "recorder still connected");
    #[cfg(not(feature = "dynamic-invoke"))]
    assert_eq!(hub.emit(panel, Panel::BUTTON_CLICKED, ()), 0, "only the typed connection existed");
    while hub.flush() > 0 {}
    #[cfg(feature = "dynamic-invoke")]
    assert_eq!(hub.get::<Recorder>(recorder).unwrap().ticks, 2);
}

/// The same chain, running as posted work on a real rt-app loop: the RT
/// state is created ON the loop thread (it is not `Send` and never crosses
/// the posting boundary — ADR 0003's thread line made visible).
#[test]
fn chain_runs_as_posted_loop_work() {
    use rt_app::{Event, EventLoop};
    use std::sync::{Arc, Mutex};

    let (loop_, sender) = EventLoop::new();
    let transcript: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = transcript.clone();
    let quitter = sender.clone();

    sender.post_event(Event::Call(Box::new(move || {
        let mut hub = Hub::new();
        let panel = hub.insert(Panel);
        let counter = hub.insert(Counter {
            count: Property::new(0),
        });
        let bridge = hub.insert(Bridge {
            label: String::new(),
        });
        hub.connect(panel, Panel::BUTTON_CLICKED, counter, Counter::bump_slot);
        hub.connect(counter, Counter::COUNT_CHANGED, bridge, Bridge::refresh);

        hub.emit(panel, Panel::BUTTON_CLICKED, ());
        while hub.flush() > 0 {}

        let mut out = sink.lock().unwrap();
        out.push(format!(
            "count={}",
            hub.get::<Counter>(counter).unwrap().count()
        ));
        out.push(hub.get::<Bridge>(bridge).unwrap().label.clone());
        quitter.quit(0);
    })));

    assert_eq!(loop_.run(), 0, "posted chain ran and quit the loop");
    assert_eq!(*transcript.lock().unwrap(), ["count=1", "clicks: 1"]);
}
