//! Ticket #19: Window object + Flush seam, headless end to end.
//!
//! Chain under test: marshalled [`WindowEvent`] message → `Window` object
//! state → `Hub::flush()` in the cycle hook → signals/property change →
//! connected observer slots, observable in the audit TraceEvent stream.

use std::cell::RefCell;
use std::rc::Rc;

use rt_core::{Handle, Hub, Object, RegistryBuilder, TraceEvent, object_impl};
use rt_gui::input::InputEvent;
use rt_gui::window::{CloseState, Window, WindowEvent, window_event_from_input};

type TraceLog = Rc<RefCell<Vec<TraceEvent>>>;

fn traced_hub() -> (Hub, TraceLog) {
    let log: TraceLog = Rc::new(RefCell::new(Vec::new()));
    let mut hub = Hub::new();
    let sink = Rc::clone(&log);
    hub.set_trace(move |event| sink.borrow_mut().push(event));
    (hub, log)
}

/// Message-driven application shell: the loop-thread owner of the Hub and the
/// Window handle. `on_message` applies window events; `on_cycle` is the Flush
/// hook (one `Hub::flush()` per cycle), exactly as a real presenter would run.
struct WinApp {
    hub: Hub,
    window: Handle,
    confirm: bool,
}

impl rt_app::Application for WinApp {
    type Msg = WindowEvent;

    fn on_message(&mut self, _ctx: &mut rt_app::Ctx<Self::Msg>, msg: Self::Msg) {
        rt_gui::window::apply_window_event(&mut self.hub, self.window, msg);
    }

    fn on_cycle(&mut self, ctx: &mut rt_app::Ctx<Self::Msg>) {
        self.hub.flush();
        if self.hub.get::<Window>(self.window).unwrap().close_state() == CloseState::CloseConfirmed
        {
            ctx.quit(0);
        }
        // Answer a pending close out-of-band, like an unsaved-changes dialog:
        // confirm (next cycle) or veto, per the test's wiring.
        if self.hub.get::<Window>(self.window).unwrap().close_state() == CloseState::PendingClose {
            let confirm = self.confirm;
            let window = self.window;
            let hub = &mut self.hub;
            if confirm {
                hub.get_mut::<Window>(window).unwrap().confirm_close();
            } else {
                hub.get_mut::<Window>(window).unwrap().veto_close();
            }
            self.hub.flush();
        }
    }
}

#[derive(Object)]
struct WindowObserver {
    resizes: Vec<(u32, u32)>,
    scale_factors: Vec<f64>,
    close_requests: u32,
}

#[object_impl]
impl WindowObserver {
    #[slot]
    fn on_resized(&mut self, width: u32, height: u32) {
        self.resizes.push((width, height));
    }

    #[slot]
    fn on_scale_factor(&mut self, factor: f64) {
        self.scale_factors.push(factor);
    }

    #[slot]
    fn on_close_requested(&mut self) {
        self.close_requests += 1;
    }
}

fn setup(hub: &mut Hub) -> (Handle, Handle) {
    let window = hub.insert(Window::new(800, 600));
    let observer = hub.insert(WindowObserver {
        resizes: vec![],
        scale_factors: vec![],
        close_requests: 0,
    });
    hub.connect(
        window,
        Window::RESIZED,
        observer,
        WindowObserver::on_resized_slot,
    );
    hub.connect(
        window,
        Window::SCALE_FACTOR_CHANGED,
        observer,
        WindowObserver::on_scale_factor,
    );
    hub.connect(
        window,
        Window::CLOSE_REQUESTED,
        observer,
        WindowObserver::on_close_requested_slot,
    );
    (window, observer)
}

#[test]
fn registry_registration_holds() {
    let mut builder = RegistryBuilder::new();
    rt_gui::window::register(&mut builder).register::<WindowObserver>();
    let registry = builder.build();
    assert!(registry.by_name("Window").is_some());
    assert!(registry.by_name("NoSuchType").is_none());
}

#[test]
fn window_event_from_input_maps_only_window_level_events() {
    assert_eq!(
        window_event_from_input(InputEvent::Resized {
            width: 10,
            height: 20
        }),
        Some(WindowEvent::Resized {
            width: 10,
            height: 20
        })
    );
    assert_eq!(
        window_event_from_input(InputEvent::CloseRequested),
        Some(WindowEvent::CloseRequested)
    );
    assert_eq!(
        window_event_from_input(InputEvent::MouseMoved { x: 1, y: 2 }),
        None
    );
}

#[test]
fn resize_message_dirty_flush_signal_observer_and_trace() {
    let (mut hub, _log) = traced_hub();
    let (window, _observer) = setup(&mut hub);

    // Nothing synchronous: the message lands as state + queued emission, the
    // delivery waits for the Flush step.
    rt_gui::window::apply_window_event(
        &mut hub,
        window,
        WindowEvent::Resized {
            width: 1024,
            height: 768,
        },
    );
    let win = hub.get::<Window>(window).unwrap();
    assert_eq!((win.width(), win.height()), (1024, 768));
    assert_eq!(hub.pending(), 1);

    assert_eq!(hub.flush(), 1);
    let obs = hub.get::<WindowObserver>(_observer).unwrap();
    assert_eq!(obs.resizes, [(1024, 768)]);

    // Trace: Emitted { RESIZED } at emit, Delivered at flush (after the
    // three Connected records from setup).
    let log = _log.borrow();
    assert!(matches!(
        log[log.len() - 2..],
        [
            TraceEvent::Emitted {
                signal_index: 0,
                reached: 1,
                ..
            },
            TraceEvent::Delivered {
                signal_index: 0,
                ..
            },
        ]
    ));
}

#[test]
fn scale_factor_property_change_delivers_at_flush_and_coalesces() {
    let (mut hub, _log) = traced_hub();
    let (window, observer) = setup(&mut hub);

    rt_gui::window::apply_window_event(&mut hub, window, WindowEvent::ScaleFactorChanged(1.5));
    rt_gui::window::apply_window_event(&mut hub, window, WindowEvent::ScaleFactorChanged(2.0));
    assert_eq!(hub.flush(), 1);
    let obs = hub.get::<WindowObserver>(observer).unwrap();
    assert_eq!(obs.scale_factors, [2.0]);
    assert_eq!(hub.get::<Window>(window).unwrap().scale_factor(), 2.0);
}

#[test]
fn close_chain_confirm_via_app_quit() {
    let (mut hub, _log) = traced_hub();
    let (window, observer) = setup(&mut hub);
    let (app, handle) = rt_app::App::new();
    let win_app = WinApp {
        hub,
        window,
        confirm: true,
    };
    // Another window's user click; ignored, mouse stays InputEvent.
    handle.send(WindowEvent::Resized {
        width: 640,
        height: 480,
    });
    handle.send(WindowEvent::CloseRequested);
    drop(handle);
    let (win_app, code) = rt_app::App::run(app, win_app);
    assert_eq!(code, 0);
    let obs = win_app.hub.get::<WindowObserver>(observer).unwrap();
    assert_eq!(obs.resizes, [(640, 480)]);
    assert_eq!(obs.close_requests, 1);
    assert_eq!(
        win_app.hub.get::<Window>(window).unwrap().close_state(),
        CloseState::CloseConfirmed
    );
}

#[test]
fn close_chain_veto_then_second_request_asks_again() {
    let (mut hub, _log) = traced_hub();
    let (window, observer) = setup(&mut hub);
    let (app, handle) = rt_app::App::new();
    let win_app = WinApp {
        hub,
        window,
        confirm: false,
    };
    handle.send(WindowEvent::CloseRequested);
    drop(handle);
    let (app, _code_unused) = {
        let (a, code) = rt_app::App::run(app, win_app);
        // The veto path does not quit; the loop ended because the handle
        // dropped. Assert the veto took effect before continuing.
        assert_eq!(code, 0);
        (a, code)
    };
    {
        let obs = app.hub.get::<WindowObserver>(observer).unwrap();
        assert_eq!(obs.close_requests, 1);
        let win = app.hub.get::<Window>(window).unwrap();
        assert_eq!(win.close_state(), CloseState::Open);
    }
    // After a veto, a new close request must ask again (state reset to Open).
    let (app2, handle2) = rt_app::App::new();
    let second = WinApp {
        hub: app.hub,
        window,
        confirm: true,
    };
    handle2.send(WindowEvent::CloseRequested);
    drop(handle2);
    let (second, code) = rt_app::App::run(app2, second);
    assert_eq!(code, 0);
    assert_eq!(
        second
            .hub
            .get::<WindowObserver>(observer)
            .unwrap()
            .close_requests,
        2
    );
    assert_eq!(
        second.hub.get::<Window>(window).unwrap().close_state(),
        CloseState::CloseConfirmed
    );
}

#[test]
fn reentrant_close_request_coalesces_while_pending() {
    let mut hub = Hub::new();
    let window = hub.insert(Window::new(100, 100));
    let win = hub.get_mut::<Window>(window).unwrap();
    win.handle_event(WindowEvent::CloseRequested);
    assert_eq!(win.close_state(), CloseState::PendingClose);
    win.handle_event(WindowEvent::CloseRequested);
    assert_eq!(win.close_state(), CloseState::PendingClose);
    assert!(win.veto_close());
    assert_eq!(win.close_state(), CloseState::Open);
    assert!(!win.veto_close());
    assert!(!win.confirm_close());
    win.handle_event(WindowEvent::CloseRequested);
    assert!(win.confirm_close());
    assert_eq!(win.close_state(), CloseState::CloseConfirmed);
    assert!(!win.confirm_close());
}
