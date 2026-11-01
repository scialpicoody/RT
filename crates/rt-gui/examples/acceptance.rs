//! Acceptance demo (#15), window leg: the full stack visible on screen.
//!
//! One [`run_window`] call owns every thread: the platform window lives on
//! the main thread, the RT loop (Hub + objects + `Gui`) runs on the
//! presenter's own loop thread holding the [`WindowShell`], and everything
//! that crosses between is a typed [`WindowMsg`] — input, window events,
//! user payloads; painted frames come back for presentation (vello_cpu →
//! softbuffer on the default CPU path). No hand-rolled bridges: no mutex
//! slots, no condvar pumps, no poll timeouts — the presenter owns the
//! threading.
//!
//! The accepted chain, end to end on a real window: a timer-synthesized
//! click lands through the shell's hit-tested input routing → the
//! controller emits `button_clicked` → the cycle's `Hub::flush()` (the
//! Flush seam) delivers `bump` from the queue → the dirty `count` property
//! evaluates → `COUNT_CHANGED` reaches the label slot → one label sync →
//! exactly one damage-gated repaint. The [`Window`] object rides the same
//! flush: the synthesized close request is queued as a `close_requested`
//! emission, delivered to an approver slot, and the application — having
//! audited the end state — confirms it, so the presenter quits with code 0.
//!
//! The audit trace is the differentiator vs Qt: the Hub records every
//! dispatch step, and this demo asserts the exact transcript — event by
//! event, in delivery order — as it happens. The headless leg of the same
//! chain lives in `tests/acceptance.rs`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use rt_app::{AppHandle, Ctx, TimerId};
use rt_core::{Handle, Object, Property, TraceEvent, object_impl};
use rt_gui::input::InputEvent;
use rt_gui::presenter::{WindowApplication, WindowConfig, WindowMsg, WindowShell, run_window};
use rt_gui::window::{CloseState, Window, WindowEvent};

// ---------------------------------------------------------------- objects

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

/// Observes the Window object's close request through the Hub — the same
/// object model the click chain uses, applied to window lifecycle.
#[derive(Object)]
struct Approver {
    close_requests: u32,
}
#[object_impl]
impl Approver {
    #[slot]
    fn on_close_requested(&mut self) {
        self.close_requests += 1;
    }
}

// ------------------------------------------------------------ application

/// The demo's object identity, handed over as a plain-data message once the
/// bootstrap filled the Hub (handles are `Copy`; the objects themselves stay
/// loop-thread state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handles {
    panel: Handle,
    counter: Handle,
    bridge: Handle,
    approver: Handle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Demo {
    Start(Handles),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Click,
    Close,
}

struct AcceptanceApp {
    objects: Option<Handles>,
    timers: Vec<(TimerId, Step)>,
    clicks: u32,
    /// The label the UI last synced from the flushed bridge state.
    painted_label: String,
    label_syncs: u32,
}

impl WindowApplication<Demo> for AcceptanceApp {
    fn on_window_message(
        &mut self,
        ctx: &mut Ctx<WindowMsg<Demo>>,
        msg: WindowMsg<Demo>,
        _shell: &mut WindowShell,
    ) {
        if let WindowMsg::User(Demo::Start(objects)) = msg {
            self.objects = Some(objects);
            // Auto-play schedule: three clicks, then the close request.
            self.timers = vec![
                (ctx.after(Duration::from_millis(150)), Step::Click),
                (ctx.after(Duration::from_millis(300)), Step::Click),
                (ctx.after(Duration::from_millis(450)), Step::Click),
                (ctx.after(Duration::from_millis(750)), Step::Close),
            ];
        }
    }

    fn on_window_timer(
        &mut self,
        _ctx: &mut Ctx<WindowMsg<Demo>>,
        timer: TimerId,
        shell: &mut WindowShell,
    ) {
        let Some(pos) = self.timers.iter().position(|(id, _)| *id == timer) else {
            return;
        };
        let (_, step) = self.timers.remove(pos);
        let objects = self.objects.expect("objects known before any timer fires");
        match step {
            Step::Click => {
                // Click the button programmatically: synthetic mouse input
                // routed through the shell (hit test on post-layout geometry).
                let rect = shell
                    .gui()
                    .rect_of(shell.gui().button_id(0))
                    .expect("button laid out");
                let (x, y) = (
                    (rect.x0 + rect.width() / 2.0) as i32,
                    (rect.y0 + rect.height() / 2.0) as i32,
                );
                shell.handle_input(InputEvent::MouseMoved { x, y });
                shell.handle_input(InputEvent::MousePressed { x, y });
                let before = shell.gui().click_count();
                shell.handle_input(InputEvent::MouseReleased { x, y });
                assert_eq!(
                    shell.gui().click_count(),
                    before + 1,
                    "synthetic click must land on the button"
                );
                // The widget-layer fact becomes an object-model emission:
                // queued now, delivered at this cycle's flush.
                let reached = shell
                    .hub_mut()
                    .emit(objects.panel, Panel::BUTTON_CLICKED, ());
                assert_eq!(reached, 1, "panel → counter connection must be live");
                self.clicks += 1;
            }
            Step::Close => {
                // As the window manager would: the Window object records the
                // request and queues close_requested for the flush.
                shell.handle_window_event(WindowEvent::CloseRequested);
            }
        }
    }

    fn on_window_cycle(&mut self, _ctx: &mut Ctx<WindowMsg<Demo>>, shell: &mut WindowShell) {
        // Frame sanity on the real CPU presentation path.
        let (width, height) = (shell.window().width(), shell.window().height());
        let frame = shell.gui().frame();
        assert_eq!(
            frame.len(),
            (width * height) as usize,
            "frame must match the window's physical size"
        );
        assert!(
            frame.iter().collect::<HashSet<_>>().len() > 2,
            "frame must show background, button, and label pixels"
        );
        // Damage discipline: this cycle's single damage-gated paint consumed
        // every pending mark — the chain never leaks a repaint.
        assert!(
            !shell.gui().is_dirty(),
            "unpainted damage at cycle end"
        );

        // Model → view sync: the flushed bridge label IS the UI text. One
        // change, one damage mark, one repaint at the next cycle's paint.
        let objects = self.objects.expect("objects known before cycles run");
        let label = shell
            .hub()
            .get::<Bridge>(objects.bridge)
            .expect("bridge live")
            .label
            .clone();
        if label != self.painted_label {
            shell.gui_mut().set_label(label.clone());
            assert!(
                shell.gui().is_dirty(),
                "the label change schedules exactly its one repaint"
            );
            self.painted_label = label;
            self.label_syncs += 1;
        }

        // End of the story: every click flowed through and the close request
        // was delivered through the Window object — audit the end state,
        // then confirm the close so the presenter quits with code 0.
        let close_requests = shell
            .hub()
            .get::<Approver>(objects.approver)
            .expect("approver live")
            .close_requests;
        if self.clicks == 3 && close_requests == 1 {
            assert_eq!(self.label_syncs, 3, "one label sync per click");
            assert_eq!(
                shell.hub().get::<Counter>(objects.counter).unwrap().count(),
                3,
                "each click bumped the count exactly once"
            );
            assert_eq!(self.painted_label, "clicks: 3");
            assert_eq!(
                shell.window().close_state(),
                CloseState::PendingClose,
                "the close request is on the table"
            );
            assert!(shell.window_mut().confirm_close(), "close must confirm");
        }
    }

    fn on_window_shutdown(&mut self, code: i32) {
        println!(
            "acceptance: shutdown with code {code} after {} clicks and {} label syncs",
            self.clicks, self.label_syncs
        );
    }
}

// -------------------------------------------------------------- transcript

/// Whether a trace event belongs to the demo's objects. Window-level traces
/// (e.g. the platform resize emission, reached: 0) reference the Window
/// handle only and are not part of the asserted story.
fn touches(event: &TraceEvent, watched: &[Handle]) -> bool {
    match *event {
        TraceEvent::Connected {
            emitter,
            receiver,
            ..
        } => watched.contains(&emitter) || watched.contains(&receiver),
        TraceEvent::Emitted { emitter, .. } => watched.contains(&emitter),
        TraceEvent::Delivered { receiver, .. } | TraceEvent::DroppedStale { receiver, .. } => {
            watched.contains(&receiver)
        }
        TraceEvent::Disconnected { .. }
        | TraceEvent::Destroyed { .. }
        | TraceEvent::DynamicLookup { .. }
        | TraceEvent::BindingEvaluated { .. } => false,
    }
}

/// The exact Hub dispatch transcript the demo will produce: three Connected
/// records at bootstrap; per click one Emitted (at the emit) and one
/// Delivered for bump (at the flush's queued phase); the property-change
/// deliveries (dirty phase) land one flush ahead of the click that caused
/// them, so click 2's flush delivers click 1's COUNT_CHANGED before its own
/// queued bump; the close request is delivered last. Signal indices: 0 for
/// the declared-free Panel and the property-only Counter; the Window's
/// declared signals come first — resized = 0, close_requested = 1.
fn transcript(window: Handle, handles: Handles) -> Vec<TraceEvent> {
    let Handles {
        panel,
        counter,
        bridge,
        approver,
    } = handles;
    let emitted = || TraceEvent::Emitted {
        emitter: panel,
        signal_index: 0,
        reached: 1,
    };
    let bumped = || TraceEvent::Delivered {
        receiver: counter,
        signal_index: 0,
    };
    let changed = || TraceEvent::Delivered {
        receiver: bridge,
        signal_index: 0,
    };
    vec![
        TraceEvent::Connected {
            emitter: panel,
            receiver: counter,
            signal_index: 0,
        },
        TraceEvent::Connected {
            emitter: counter,
            receiver: bridge,
            signal_index: 0,
        },
        TraceEvent::Connected {
            emitter: window,
            receiver: approver,
            signal_index: 1,
        },
        emitted(),
        bumped(),
        emitted(),
        changed(),
        bumped(),
        emitted(),
        changed(),
        bumped(),
        changed(),
        TraceEvent::Delivered {
            receiver: approver,
            signal_index: 1,
        },
    ]
}

/// Assert-and-print the audit trace: every demo-object event must match the
/// expected transcript prefix, in order, with no extras. Loop-thread only —
/// the `Rc` never leaves the thread the Hub lives on.
fn watch_trace(hub: &mut rt_core::Hub, window: Handle, handles: Handles) {
    let expected = transcript(window, handles);
    let watched = [
        handles.panel,
        handles.counter,
        handles.bridge,
        handles.approver,
    ];
    let log: Rc<RefCell<Vec<TraceEvent>>> = Rc::new(RefCell::new(Vec::new()));
    hub.set_trace({
        let log = Rc::clone(&log);
        move |event| {
            if !touches(&event, &watched) {
                return;
            }
            let mut log = log.borrow_mut();
            let step = log.len();
            assert!(
                step < expected.len(),
                "audit trace: unexpected extra event {event:?}"
            );
            assert_eq!(
                &event, &expected[step],
                "audit trace: event {step} diverged from the transcript"
            );
            println!("  trace {step:2}: {event:?}");
            log.push(event);
            if log.len() == expected.len() {
                println!(
                    "audit transcript: {} events, exact match — click → property → flush → change → repaint, close through the Window object",
                    expected.len()
                );
            }
        }
    });
}

// -------------------------------------------------------------------- main

fn main() {
    let code = run_window(
        WindowConfig {
            title: "rt-gui acceptance".to_owned(),
            width: 480,
            height: 320,
            ..WindowConfig::default()
        },
        AcceptanceApp {
            objects: None,
            timers: Vec::new(),
            clicks: 0,
            painted_label: "clicks: 0".to_owned(),
            label_syncs: 0,
        },
        |shell: &mut WindowShell, handle: &AppHandle<WindowMsg<Demo>>| {
            // Registry fill phase (ADR 0002): every object kind the demo uses.
            shell
                .registry_mut()
                .register::<Panel>()
                .register::<Counter>()
                .register::<Bridge>()
                .register::<Approver>();

            let window = shell.window_handle();
            let hub = shell.hub_mut();
            let panel = hub.insert(Panel);
            let counter = hub.insert(Counter {
                count: Property::new(0),
            });
            let bridge = hub.insert(Bridge {
                label: "clicks: 0".to_owned(),
            });
            let approver = hub.insert(Approver {
                close_requests: 0,
            });

            // The audit transcript: the soul of the example — determinism
            // you can assert. Installed before the connects so the
            // Connected records are on the record too.
            let handles = Handles {
                panel,
                counter,
                bridge,
                approver,
            };
            watch_trace(hub, window, handles);

            // Typed connections: click → bump; count change → label refresh;
            // window close request → approver.
            hub.connect(panel, Panel::BUTTON_CLICKED, counter, Counter::bump_slot);
            hub.connect(counter, Counter::COUNT_CHANGED, bridge, Bridge::refresh);
            hub.connect(
                window,
                Window::CLOSE_REQUESTED,
                approver,
                Approver::on_close_requested_slot,
            );

            shell.gui_mut().set_label("clicks: 0");
            // The only cross-thread send: hand the loop its objects; its
            // message handler arms the auto-play timers.
            handle.send(WindowMsg::User(Demo::Start(handles)));
        },
    );
    println!("acceptance demo exited with code {code}");
    assert_eq!(
        code, 0,
        "the confirmed close must quit with the configured code"
    );
}
