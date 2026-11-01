//! Window-as-object (spec #16 decision 6, ticket #19): the loop-thread
//! [`Window`] is a derive-generated RT object registered in the [`Registry`];
//! platform window events reach it as typed, `Send` [`WindowEvent`] messages.
//!
//! # Contract (ADR 0003)
//!
//! - Platform windows live on the main thread and are not `Send`. Only
//!   [`WindowEvent`] values — plain data — cross the thread boundary, handed
//!   to the loop through the application's own `rt_app::AppHandle`. rt-gui
//!   owns no channel and no shared state.
//! - On the loop thread, the application's `on_message` calls
//!   [`Window::handle_event`]; `on_cycle` performs the one `Hub::flush()` of
//!   the cycle, which delivers the [`Window::RESIZED`] and
//!   [`Window::CLOSE_REQUESTED`] signals and the
//!   [`Window::SCALE_FACTOR_CHANGED`] property change to connected slots.
//! - Mouse input stays in [`InputEvent`]; only window-level events (resize,
//!   scale change, close request) marshal through this protocol.
//!
//! # Close-confirmation chain
//!
//! `CloseRequested` marks the window [`CloseState::PendingClose`] and emits
//! `close_requested` — the application decides. It answers by calling
//! [`Window::confirm_close`] (state becomes [`CloseState::CloseConfirmed`];
//! the typical application then posts a quit) or [`Window::veto_close`]
//! (back to [`CloseState::Open`], e.g. while unsaved state exists). Until the
//! application answers, the state stays `PendingClose`, so re-entrant close
//! requests coalesce: [`Window::close_state`] exposes the answer point.

#![forbid(unsafe_code)]

use rt_core::{Object, Property, RegistryBuilder, object_impl};

/// A window-level event, marshalled as a plain `Send` message into the loop
/// thread. This is the whole protocol: no shared state, no channel — the
/// application wires its own `rt_app::AppHandle<WindowEvent>` (or wraps this
/// enum in its own message type).
///
/// Mouse events are *not* part of this protocol; they stay
/// [`InputEvent`](crate::input::InputEvent) (spec #16 decision 8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowEvent {
    /// The platform window's size changed, in physical pixels.
    Resized { width: u32, height: u32 },
    /// The display scale factor changed (HiDPI move between monitors).
    ScaleFactorChanged(f64),
    /// The user asked to close the window; the application confirms or vetoes
    /// through the Window's close-confirmation chain.
    CloseRequested,
}

const _: () = {
    const fn assert_send<T: Send + 'static>() {}
    assert_send::<WindowEvent>();
};

/// Free-function seam for presenters and tests: the window-level subset of an
/// [`InputEvent`](crate::input::InputEvent), or `None` for pointer input
/// (which stays widget-facing).
pub fn window_event_from_input(event: crate::input::InputEvent) -> Option<WindowEvent> {
    match event {
        crate::input::InputEvent::Resized { width, height } => {
            Some(WindowEvent::Resized { width, height })
        }
        crate::input::InputEvent::CloseRequested => Some(WindowEvent::CloseRequested),
        _ => None,
    }
}

/// The application's answer to a close request. `Open` is the steady state;
/// `PendingClose` means the question is on the table; `CloseConfirmed` means
/// the application accepted (the presenter quits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseState {
    #[default]
    Open,
    PendingClose,
    CloseConfirmed,
}

/// The loop-thread Window object: state plus the signal/property surface the
/// application observes. Lives in the [`Hub`](rt_core::Hub); constructed
/// plainly, inserted with `Hub::insert`.
#[derive(Object)]
pub struct Window {
    /// Device pixel ratio; observers recompute layout/paint scale at flush.
    #[property]
    scale_factor: Property<f64>,
    width: u32,
    height: u32,
    close_state: CloseState,
}

#[object_impl]
impl Window {
    /// Emitted (queued; delivered at flush) after the size state updated.
    #[signal]
    fn resized(&self, width: u32, height: u32);

    /// Emitted (queued; delivered at flush) when a close request arrives.
    /// The connected application slot is the decision point of the
    /// close-confirmation chain.
    #[signal]
    fn close_requested(&self);

    /// A new Window at `width` x `height` physical pixels, scale factor 1.0,
    /// not closing.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            scale_factor: Property::new(1.0),
            width,
            height,
            close_state: CloseState::Open,
        }
    }

    /// Apply one marshalled window event: update state and return the
    /// emissions to deliver. Objects never self-emit (ADR 0004: no Hub
    /// back-reference); the loop-thread driver
    /// (`apply_window_event`, or the application's `on_message`) emits the
    /// returned emissions through the [`Hub`](rt_core::Hub), and the flush
    /// step delivers them.
    pub fn handle_event(&mut self, event: WindowEvent) -> Vec<WindowEmission> {
        match event {
            WindowEvent::Resized { width, height } => {
                self.width = width;
                self.height = height;
                vec![WindowEmission::Resized { width, height }]
            }
            WindowEvent::ScaleFactorChanged(factor) => {
                // Property change: delivered by the flush's dirty-property
                // phase, no explicit emission.
                self.set_scale_factor(factor);
                Vec::new()
            }
            WindowEvent::CloseRequested => {
                // Coalesce re-entrant requests while the app deliberates.
                if self.close_state == CloseState::Open {
                    self.close_state = CloseState::PendingClose;
                    vec![WindowEmission::CloseRequested]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The application's answer point: accept the pending close. Returns
    /// `true` exactly when a pending close was confirmed; the typical
    /// application then posts its quit.
    pub fn confirm_close(&mut self) -> bool {
        if self.close_state == CloseState::PendingClose {
            self.close_state = CloseState::CloseConfirmed;
            true
        } else {
            false
        }
    }

    /// Refuse the pending close (e.g. unsaved state); the window returns to
    /// [`CloseState::Open`] and later close requests ask again. Returns
    /// `true` when a pending close was actually vetoed.
    pub fn veto_close(&mut self) -> bool {
        if self.close_state == CloseState::PendingClose {
            self.close_state = CloseState::Open;
            true
        } else {
            false
        }
    }

    /// Where the close-confirmation chain stands.
    pub fn close_state(&self) -> CloseState {
        self.close_state
    }
}

/// Register the Window object kind in the application's Registry fill phase:
/// `registry_builder.register::<Window>()` via this helper keeps the type
/// import one-sided. Chainable, like [`RegistryBuilder::register`].
pub fn register(builder: &mut RegistryBuilder) -> &mut RegistryBuilder {
    builder.register::<Window>()
}

/// The emissions a [`WindowEvent`] produces: the loop-thread driver feeds
/// these to [`Hub::emit`](rt_core::Hub::emit) right after
/// [`Window::handle_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEmission {
    /// Emit [`Window::RESIZED`] with the new size.
    Resized { width: u32, height: u32 },
    /// Emit [`Window::CLOSE_REQUESTED`].
    CloseRequested,
}

/// Loop-thread entry point: apply `event` to the Window at `handle` (state
/// update) and emit the resulting emissions through the Hub. The queued
/// deliveries and the property change reach observers at the next
/// `Hub::flush()` — the cycle hook, per ADR 0003. Returns the number of
/// emissions queued (a coalesced close request returns 0).
///
/// Panics when `handle` is stale or not a `Window`.
pub fn apply_window_event(
    hub: &mut rt_core::Hub,
    handle: rt_core::Handle,
    event: WindowEvent,
) -> usize {
    let emissions = hub
        .get_mut::<Window>(handle)
        .expect("rt-gui: window handle stale")
        .handle_event(event);
    for emission in &emissions {
        match emission {
            WindowEmission::Resized { width, height } => {
                hub.emit(handle, Window::RESIZED, (*width, *height));
            }
            WindowEmission::CloseRequested => {
                hub.emit(handle, Window::CLOSE_REQUESTED, ());
            }
        }
    }
    emissions.len()
}
