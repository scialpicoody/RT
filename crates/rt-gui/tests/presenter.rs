//! Ticket #22: the `run_window` presenter's loop-thread shell, headless.
//!
//! [`WindowShell`] is the presenter's whole loop-thread surface: window
//! events update the `Window` object + widget tree, input routes to widgets,
//! `flush_and_paint` is the single Flush step per cycle with a damage-gated
//! repaint. The presenter's threading (winit main thread + RT loop thread +
//! frame channel) is proven by the unattended `run_window_demo` example, per
//! spec #16 testing decisions — never in CI.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use rt_core::{Handle, Object, TraceEvent, object_impl};
use rt_gui::input::InputEvent;
use rt_gui::presenter::WindowShell;
use rt_gui::window::{CloseState, Window, WindowEvent};

type TraceLog = Rc<RefCell<Vec<TraceEvent>>>;

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

/// Shell with an observer connected to every Window signal and a trace log.
fn traced_shell() -> (WindowShell, Handle, TraceLog) {
    let mut shell = WindowShell::new(320, 240);
    let observer = shell.hub_mut().insert(WindowObserver {
        resizes: Vec::new(),
        scale_factors: Vec::new(),
        close_requests: 0,
    });
    let window_handle = shell.window_handle();
    shell.hub_mut().connect(
        window_handle,
        Window::RESIZED,
        observer,
        WindowObserver::on_resized_slot,
    );
    shell.hub_mut().connect(
        window_handle,
        Window::SCALE_FACTOR_CHANGED,
        observer,
        WindowObserver::on_scale_factor_slot,
    );
    shell.hub_mut().connect(
        window_handle,
        Window::CLOSE_REQUESTED,
        observer,
        WindowObserver::on_close_requested_slot,
    );
    let log: TraceLog = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&log);
    shell
        .hub_mut()
        .set_trace(move |event| sink.borrow_mut().push(event));
    (shell, observer, log)
}

fn distinct_colors(frame: &[u32]) -> usize {
    frame.iter().collect::<HashSet<_>>().len()
}

#[test]
fn resize_message_updates_window_object_gui_and_damage() {
    let (mut shell, observer, _) = traced_shell();

    shell.handle_window_event(WindowEvent::Resized {
        width: 640,
        height: 480,
    });
    // Not delivered yet: the flush step owns delivery.
    assert_eq!(
        shell.hub().get::<WindowObserver>(observer).unwrap().resizes,
        Vec::<(u32, u32)>::new()
    );

    assert!(shell.flush_and_paint(), "a resize must schedule a repaint");
    let observed = shell.hub().get::<WindowObserver>(observer).unwrap();
    assert_eq!(observed.resizes, vec![(640, 480)]);

    // Window object state updated…
    assert_eq!(shell.window().width(), 640);
    assert_eq!(shell.window().height(), 480);
    // …the Gui followed (relayout resolved into the bigger surface; buttons
    // are fixed-size 200x88, so the viewport is the observable)…
    let rect = shell
        .gui()
        .rect_of(shell.gui().button_id(0))
        .expect("layout resolved after resize");
    assert!(rect.x1 <= 640.0 && rect.y1 <= 480.0);
    // …the repaint happened (damage drained, real pixels out)…
    assert!(!shell.gui().is_dirty());
    let frame = shell.gui().frame();
    assert_eq!(frame.len(), (640 * 480) as usize);
    assert!(
        distinct_colors(frame) > 1,
        "buttons must be visible on black"
    );
}

#[test]
fn scale_factor_change_delivers_once_at_flush() {
    let (mut shell, observer, _) = traced_shell();

    shell.handle_window_event(WindowEvent::ScaleFactorChanged(1.5));
    shell.handle_window_event(WindowEvent::ScaleFactorChanged(2.0));
    assert!(shell.flush_and_paint());
    let observed = shell.hub().get::<WindowObserver>(observer).unwrap();
    // Latest value wins; the property change is delivered, not queued twice.
    assert_eq!(observed.scale_factors, vec![2.0]);
}

#[test]
fn close_chain_through_the_shell() {
    let (mut shell, observer, _) = traced_shell();

    shell.handle_window_event(WindowEvent::CloseRequested);
    assert_eq!(shell.window().close_state(), CloseState::PendingClose);
    // A second request while pending coalesces (no duplicate asks). No
    // damage is scheduled, but the flush still delivers the signal.
    shell.handle_window_event(WindowEvent::CloseRequested);
    shell.flush_and_paint();
    assert_eq!(
        shell
            .hub()
            .get::<WindowObserver>(observer)
            .unwrap()
            .close_requests,
        1
    );

    // Veto: back to Open; the window keeps running.
    assert!(shell.window_mut().veto_close());
    assert_eq!(shell.window().close_state(), CloseState::Open);

    // Re-request: the user is asked again.
    shell.handle_window_event(WindowEvent::CloseRequested);
    shell.flush_and_paint();
    assert_eq!(
        shell
            .hub()
            .get::<WindowObserver>(observer)
            .unwrap()
            .close_requests,
        2
    );
    assert_eq!(shell.window().close_state(), CloseState::PendingClose);

    // Confirm: quit-eligible — the presenter maps this state to ctx.quit.
    assert!(shell.window_mut().confirm_close());
    assert_eq!(shell.window().close_state(), CloseState::CloseConfirmed);
}

#[test]
fn input_routes_to_widgets_through_the_shell() {
    let (mut shell, _, _) = traced_shell();
    assert!(shell.flush_and_paint(), "initial paint");

    let rect = shell.gui().rect_of(shell.gui().button_id(0)).unwrap();
    let center = (
        (rect.x0 + rect.width() / 2.0) as i32,
        (rect.y0 + rect.height() / 2.0) as i32,
    );
    assert!(
        shell.handle_input(InputEvent::MouseMoved {
            x: center.0,
            y: center.1,
        }),
        "entering a button flips it to hovered: damage"
    );
    assert!(shell.handle_input(InputEvent::MousePressed {
        x: center.0,
        y: center.1,
    }));
    assert!(shell.handle_input(InputEvent::MouseReleased {
        x: center.0,
        y: center.1,
    }));
    assert_eq!(shell.gui().click_count(), 1);
    // The click's damage repaints at the next flush-and-paint…
    assert!(shell.flush_and_paint());
    // …and a clean cycle paints nothing.
    assert!(!shell.flush_and_paint());
}

#[test]
fn flush_and_paint_is_the_single_flush_step() {
    let (mut shell, observer, trace) = traced_shell();

    // Queue two window events; both must deliver at the one flush.
    shell.handle_window_event(WindowEvent::Resized {
        width: 100,
        height: 80,
    });
    shell.handle_window_event(WindowEvent::Resized {
        width: 200,
        height: 160,
    });
    assert!(shell.flush_and_paint());

    let delivered = trace
        .borrow()
        .iter()
        .filter(|event| matches!(event, TraceEvent::Delivered { receiver, .. } if *receiver == observer))
        .count();
    assert_eq!(
        delivered, 2,
        "one flush delivers every queued signal exactly once"
    );
    assert_eq!(shell.hub().pending(), 0);

    // A clean cycle: no queued emissions, no damage — nothing happens.
    let before = trace.borrow().len();
    assert!(!shell.flush_and_paint());
    assert_eq!(trace.borrow().len(), before);
    assert_eq!(
        shell.hub().get::<WindowObserver>(observer).unwrap().resizes,
        vec![(100, 80), (200, 160)]
    );
}
