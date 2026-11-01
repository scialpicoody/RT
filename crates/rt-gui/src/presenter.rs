//! The `run_window` presenter (spec #16 decision 7, ticket #22): one call owns
//! the whole windowed-app structure — windowing on the main thread, the RT
//! loop on its own thread, input marshalled as typed messages, frames returned
//! for presentation (wgpu surface behind the `gpu` feature, vello_cpu →
//! softbuffer otherwise), and the close-requested confirmation chain with
//! quit-code propagation.
//!
//! # Backend selection (ticket #23)
//!
//! Rasterization is always vello_cpu — one engine representation, no widget
//! or application change. With the `gpu` feature enabled, the presenter
//! probes for a wgpu adapter at startup and presents the painted frame
//! through a wgpu surface (blit to the swapchain); probe failure is silent
//! and any wgpu error mid-run permanently falls back to CPU presentation.
//! Without the feature (the default), everything is CPU: vello_cpu →
//! softbuffer.
//!
//! # Contract (ADR 0003)
//!
//! - [`WindowShell`] — the `Window` object, the widget [`Gui`], the `Hub`, and
//!   the Registry fill phase — is loop-thread state and never `Send`. Only
//!   plain values cross a thread boundary: [`WindowMsg`]s into the loop,
//!   painted `Vec<u32>` frames out.
//! - [`WindowShell::flush_and_paint`] is THE Flush seam: one `Hub::flush()`
//!   per cycle, then layout (at most once per cycle) and a damage-gated paint.
//! - The application never sees winit: it implements [`WindowApplication`],
//!   and the presenter adapts it to `rt_app::Application` internally.

#![forbid(unsafe_code)]

use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use rt_app::{App, AppHandle, Application, Ctx, TimerId};
use rt_core::{Handle, Hub, RegistryBuilder};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent as PlatformWindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window as PlatformWindow, WindowAttributes, WindowId};

use crate::gui::Gui;
use crate::input::{InputEvent, InputTranslator};
use crate::window::{
    CloseState, Window, WindowEvent, apply_window_event, register as register_window_object,
};

/// One marshalled message delivered to
/// [`WindowApplication::on_window_message`]: a window-level event, a
/// widget-facing input event, or the application's own payload. Plain `Send`
/// data — the whole protocol crossing the thread boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowMsg<T> {
    /// A window-level event (resize, scale change, close request).
    Window(WindowEvent),
    /// Widget-facing (mouse) input, translated from the platform stream.
    Input(InputEvent),
    /// The application's own payload.
    User(T),
}

/// The presenter's window settings. `Default` is a sane 800x600 window that
/// exits 0 through the close chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowConfig {
    /// Platform window title.
    pub title: String,
    /// Initial size, physical pixels.
    pub width: u32,
    /// Initial size, physical pixels.
    pub height: u32,
    /// Quit code the presenter uses when the close chain confirms (the
    /// application answered a pending close with [`Window::confirm_close`]).
    pub close_exit_code: i32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "RT window".to_owned(),
            width: 800,
            height: 600,
            close_exit_code: 0,
        }
    }
}

/// The loop-thread window + widget state the presenter owns and hands the
/// application as `&mut` in every callback: the `Window` object (in the Hub),
/// the widget tree root ([`Gui`]), the Hub, and the Registry fill phase.
///
/// Not `Send` (the Hub is not): construct on the loop thread — `run_window`
/// does. The application composes nothing of it; every access is a method.
pub struct WindowShell {
    hub: Hub,
    window: Handle,
    gui: Gui,
    registry: RegistryBuilder,
}

impl WindowShell {
    /// A shell with a `Window` object at `width` x `height` physical pixels
    /// and a `Gui` root column (one default button, laid out on first paint).
    pub fn new(width: u32, height: u32) -> Self {
        let mut hub = Hub::new();
        let window = hub.insert(Window::new(width, height));
        let mut registry = RegistryBuilder::new();
        register_window_object(&mut registry);
        Self {
            hub,
            window,
            gui: Gui::new(width, height),
            registry,
        }
    }

    /// The Window object: size, scale factor, and [`CloseState`] reads.
    pub fn window(&self) -> &Window {
        self.hub
            .get::<Window>(self.window)
            .expect("rt-gui: shell window handle stale")
    }

    /// The Window object mutably — the application's answer point for the
    /// close-confirmation chain ([`Window::confirm_close`] /
    /// [`Window::veto_close`]).
    pub fn window_mut(&mut self) -> &mut Window {
        self.hub
            .get_mut::<Window>(self.window)
            .expect("rt-gui: shell window handle stale")
    }

    /// The Window object's Handle, for connecting Hub slots at bootstrap.
    pub fn window_handle(&self) -> Handle {
        self.window
    }

    /// The widget root (read-only: geometry, frame, click count).
    pub fn gui(&self) -> &Gui {
        &self.gui
    }

    /// The widget root mutably (compose widgets, set the status label).
    pub fn gui_mut(&mut self) -> &mut Gui {
        &mut self.gui
    }

    /// The Hub: insert objects and connect signals at bootstrap.
    pub fn hub_mut(&mut self) -> &mut Hub {
        &mut self.hub
    }

    /// The Hub (read-only: observer slots, property reads, trace log).
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The Registry fill phase (`Window` is pre-registered): register
    /// application object kinds here at bootstrap.
    pub fn registry_mut(&mut self) -> &mut RegistryBuilder {
        &mut self.registry
    }

    /// Loop-thread window-event entry: apply to the `Window` object (state
    /// update + queued emissions, delivered at the next flush) and keep the
    /// widget tree in step — a resize relayouts and marks damage for the next
    /// [`WindowShell::flush_and_paint`].
    pub fn handle_window_event(&mut self, event: WindowEvent) {
        apply_window_event(&mut self.hub, self.window, event);
        if let WindowEvent::Resized { width, height } = event {
            // Zero sizes are minimize artifacts, not layouts: keep the last
            // real size (winit re-reports a real size on restore).
            if width > 0 && height > 0 {
                self.gui
                    .handle_input(&InputEvent::Resized { width, height });
            }
        }
    }

    /// Loop-thread input entry: route mouse input through the widget tree
    /// (hit test on post-layout geometry). Returns whether new damage was
    /// scheduled. Window-level events arrive as
    /// [`WindowMsg::Window`], not here.
    pub fn handle_input(&mut self, event: InputEvent) -> bool {
        self.gui.handle_input(&event)
    }

    /// THE Flush seam: one `Hub::flush()` per cycle — queued signal
    /// deliveries out, dirty properties (e.g. the scale factor) evaluated —
    /// then layout (at most once per cycle) and a damage-gated paint.
    /// Returns whether a frame was produced.
    pub fn flush_and_paint(&mut self) -> bool {
        self.hub.flush();
        self.gui.paint().is_some()
    }
}

/// The window-aware application contract; the presenter adapts it to
/// `rt_app::Application`. All callbacks run on the loop thread with `&mut`
/// access to the [`WindowShell`].
pub trait WindowApplication<T: Send + 'static>: Send + 'static {
    /// One marshalled message (window event, input event, or user payload),
    /// after the presenter applied it to the shell.
    fn on_window_message(
        &mut self,
        ctx: &mut Ctx<WindowMsg<T>>,
        msg: WindowMsg<T>,
        shell: &mut WindowShell,
    );

    /// End of cycle: the shell has already flushed the Hub and painted when
    /// dirty (dirty frames already shipped to presentation).
    fn on_window_cycle(&mut self, ctx: &mut Ctx<WindowMsg<T>>, shell: &mut WindowShell);

    /// A timer armed through [`Ctx::after`](rt_app::Ctx::after) /
    /// [`Ctx::every`](rt_app::Ctx::every) fired.
    fn on_window_timer(
        &mut self,
        ctx: &mut Ctx<WindowMsg<T>>,
        timer: TimerId,
        shell: &mut WindowShell,
    ) {
        let _ = (ctx, timer, shell);
    }

    /// Exactly once at exit with the final quit code (mirrors rt-app's
    /// `on_shutdown`).
    fn on_window_shutdown(&mut self, code: i32) {
        let _ = code;
    }
}

/// Run a windowed application to completion; returns the quit code.
///
/// One call owns the whole structure: the platform window and presentation
/// live on the main thread (winit owns it), the RT loop runs on its own
/// thread with the [`WindowShell`], platform input is marshalled as
/// [`WindowMsg`]s through the `rt_app` inbox, and painted frames come back
/// for presentation — through a wgpu surface when the `gpu` feature is on
/// and an adapter is available, through softbuffer otherwise. The choice is
/// made at runtime inside this call and is invisible to application code.
///
/// `bootstrap` runs once on the loop thread before the cycle loop starts:
/// insert objects into the Hub ([`WindowShell::hub_mut`]), register object
/// kinds ([`WindowShell::registry_mut`]), connect signals, compose widgets,
/// and send initial messages through the handle. Timers are armed in message
/// or cycle callbacks (`Ctx` is only reachable there).
///
/// # Closing
///
/// A platform close request arrives as
/// [`WindowEvent::CloseRequested`]; the application answers through
/// [`WindowShell::window_mut`]: [`Window::confirm_close`] and the presenter
/// quits with [`WindowConfig::close_exit_code`], or [`Window::veto_close`]
/// and the window keeps running. A pending close that is never answered
/// keeps the window open.
pub fn run_window<T: Send + 'static, A: WindowApplication<T>>(
    config: WindowConfig,
    app: A,
    bootstrap: impl FnOnce(&mut WindowShell, &AppHandle<WindowMsg<T>>) + Send + 'static,
) -> i32 {
    fn assert_send<M: Send + 'static>() {}
    assert_send::<WindowMsg<T>>();

    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .expect("rt-gui: winit event loop");
    let proxy = event_loop.create_proxy();
    let (app_rt, app_handle) = App::<WindowMsg<T>>::new();
    // The loop thread and the main thread each need a sending handle.
    // `AppHandle`'s derived `Clone` over-constrains (`T: Clone`), so share
    // through an `Arc` — the sender is a channel end either way.
    let app_handle = Arc::new(app_handle);
    let (frame_tx, frame_rx) = channel::<Vec<u32>>();
    let (code_tx, code_rx) = channel::<i32>();

    let loop_thread = {
        let thread_handle = Arc::clone(&app_handle);
        let proxy = proxy.clone();
        let frame_tx = frame_tx.clone();
        std::thread::Builder::new()
            .name("rt-loop".to_owned())
            .spawn(move || {
                // The shell (Hub etc.) is loop-thread state: constructed here,
                // never moved across a thread boundary.
                let shell = WindowShell::new(config.width, config.height);
                let sink = ChannelSink {
                    tx: frame_tx,
                    proxy: proxy.clone(),
                };
                let mut presenter = PresenterApp {
                    shell,
                    app,
                    sink,
                    close_exit_code: config.close_exit_code,
                    _msg: PhantomData,
                };
                let result = catch_unwind(AssertUnwindSafe(|| {
                    bootstrap(&mut presenter.shell, &thread_handle);
                    app_rt.run(presenter).1
                }));
                match result {
                    Ok(code) => {
                        let _ = code_tx.send(code);
                    }
                    Err(panic) => {
                        // Never leave the main thread hanging on a panicked
                        // loop; the panic resurfaces through the join below.
                        let _ = code_tx.send(PANIC_EXIT_CODE);
                        let _ = proxy.send_event(());
                        resume_unwind(panic);
                    }
                }
                let _ = proxy.send_event(());
            })
            .expect("rt-gui: spawn rt loop thread")
    };

    let mut main = MainThreadPresenter {
        handle: app_handle,
        title: config.title,
        width: config.width,
        height: config.height,
        frame_rx,
        code_rx,
        final_code: None,
        window: None,
        context: None,
        surface: None,
        #[cfg(feature = "gpu")]
        gpu: None,
        translator: InputTranslator::new(),
    };
    let _ = event_loop.run_app(&mut main);

    let code = main
        .final_code
        .expect("rt-gui: rt loop sent no quit code before the loop exited");
    loop_thread.join().expect("rt-gui: rt loop thread panicked");
    code
}

/// Rust's own exit code for an unwinding panic; used when the RT loop panics
/// so the process never hangs on a dead loop thread.
const PANIC_EXIT_CODE: i32 = 101;

/// Presentation handoff: the loop thread gives a freshly painted frame to the
/// main thread. The real sink posts through an mpsc channel and wakes winit;
/// headless tests record frames instead.
trait FrameSink {
    fn ship(&mut self, frame: Vec<u32>);
}

struct ChannelSink {
    tx: Sender<Vec<u32>>,
    proxy: EventLoopProxy<()>,
}

impl FrameSink for ChannelSink {
    fn ship(&mut self, frame: Vec<u32>) {
        // No backpressure modeling: a slow main thread lets frames queue and
        // presentation takes the latest anyway.
        let _ = self.tx.send(frame);
        let _ = self.proxy.send_event(());
    }
}

/// The presenter's internal `rt_app::Application`: applies every message to
/// the shell, runs the Flush+paint step once per cycle, ships dirty frames,
/// and maps the close chain's confirmation to `ctx.quit`.
struct PresenterApp<A: WindowApplication<T>, T: Send + 'static, S: FrameSink> {
    shell: WindowShell,
    app: A,
    sink: S,
    close_exit_code: i32,
    /// Carries the message payload type for the `Application` impl.
    _msg: PhantomData<fn() -> T>,
}

impl<A, T, S> Application for PresenterApp<A, T, S>
where
    A: WindowApplication<T>,
    T: Send + 'static,
    S: FrameSink,
{
    type Msg = WindowMsg<T>;

    fn on_message(&mut self, ctx: &mut Ctx<Self::Msg>, msg: Self::Msg) {
        match &msg {
            WindowMsg::Window(event) => self.shell.handle_window_event(*event),
            WindowMsg::Input(event) => {
                self.shell.handle_input(*event);
            }
            WindowMsg::User(_) => {}
        }
        self.app.on_window_message(ctx, msg, &mut self.shell);
    }

    fn on_timer(&mut self, ctx: &mut Ctx<Self::Msg>, timer: TimerId) {
        self.app.on_window_timer(ctx, timer, &mut self.shell);
    }

    fn on_cycle(&mut self, ctx: &mut Ctx<Self::Msg>) {
        if self.shell.flush_and_paint() {
            let frame = self.shell.gui().frame().to_vec();
            self.sink.ship(frame);
        }
        self.app.on_window_cycle(ctx, &mut self.shell);
        // End-of-cycle decision point of the close-confirmation chain:
        // confirmed → quit with the configured code; vetoed or pending →
        // keep running.
        if self.shell.window().close_state() == CloseState::CloseConfirmed {
            ctx.quit(self.close_exit_code);
        }
    }

    fn on_shutdown(&mut self, code: i32) {
        self.app.on_window_shutdown(code);
    }
}

/// Main-thread side: owns the platform window + presentation, translates
/// winit events into [`WindowMsg`]s, and presents the latest loop-thread
/// frame on redraw. Presentation backend: wgpu surface behind the `gpu`
/// feature (probe at resume), softbuffer otherwise — and softbuffer is
/// created lazily at first CPU present, so a healthy GPU path never touches
/// it (on macOS both attach a layer to the same view; the fallback case is
/// exactly the case where softbuffer must be the last creator).
struct MainThreadPresenter<T> {
    handle: Arc<AppHandle<WindowMsg<T>>>,
    title: String,
    width: u32,
    height: u32,
    frame_rx: Receiver<Vec<u32>>,
    code_rx: Receiver<i32>,
    final_code: Option<i32>,
    window: Option<Arc<PlatformWindow>>,
    context: Option<softbuffer::Context<Arc<PlatformWindow>>>,
    surface: Option<softbuffer::Surface<Arc<PlatformWindow>, Arc<PlatformWindow>>>,
    /// GPU presentation; `None` once the probe failed or wgpu errored —
    /// the CPU path takes over permanently from that frame on.
    #[cfg(feature = "gpu")]
    gpu: Option<crate::gpu::GpuPresenter>,
    translator: InputTranslator,
}

impl<T: Send + 'static> MainThreadPresenter<T> {
    /// Present the latest loop-thread frame: drain the channel, keep last —
    /// intermediate frames are stale by definition.
    fn present_latest(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let mut latest = None;
        while let Ok(frame) = self.frame_rx.try_recv() {
            latest = Some(frame);
        }
        let Some(frame) = latest else {
            return;
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        #[cfg(feature = "gpu")]
        if let Some(gpu) = self.gpu.as_mut() {
            if gpu.present(&frame, width, height) {
                return;
            }
            // wgpu failed (already logged inside): CPU presentation takes
            // over permanently.
            self.gpu = None;
        }
        self.present_latest_cpu(window, frame, width, height);
    }

    /// The CPU presentation path: softbuffer surface, created lazily so a
    /// healthy GPU path never creates one (see the struct doc).
    fn present_latest_cpu(
        &mut self,
        window: Arc<PlatformWindow>,
        frame: Vec<u32>,
        width: NonZeroU32,
        height: NonZeroU32,
    ) {
        if self.surface.is_none() {
            let context =
                softbuffer::Context::new(window.clone()).expect("rt-gui: softbuffer context");
            let surface = softbuffer::Surface::new(&context, window.clone())
                .expect("rt-gui: softbuffer surface");
            self.context = Some(context);
            self.surface = Some(surface);
        }
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        surface
            .resize(width, height)
            .expect("rt-gui: surface resize");
        let mut buffer = surface.buffer_mut().expect("rt-gui: surface buffer");
        if buffer.len() == frame.len() {
            buffer.copy_from_slice(&frame);
            buffer.present().expect("rt-gui: present");
        }
    }
}

impl<T: Send + 'static> ApplicationHandler<()> for MainThreadPresenter<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title(self.title.clone())
                    .with_inner_size(PhysicalSize::new(self.width, self.height)),
            )
            .expect("rt-gui: create platform window");
        let size = window.inner_size();
        let scale_factor = window.scale_factor();
        let window = Arc::new(window);
        // softbuffer stays uninitialized here (created lazily at first CPU
        // present); with the `gpu` feature, probe the wgpu adapter now —
        // failure is silent and leaves everything on the CPU path.
        #[cfg(feature = "gpu")]
        {
            self.gpu = crate::gpu::GpuPresenter::new(&window);
        }
        self.window = Some(window);
        // Sync the loop-thread Window object with the real platform window;
        // the resize message also drives the first paint once the loop pumps.
        self.handle.send(WindowMsg::Window(WindowEvent::Resized {
            width: size.width,
            height: size.height,
        }));
        self.handle
            .send(WindowMsg::Window(WindowEvent::ScaleFactorChanged(
                scale_factor,
            )));
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: PlatformWindowEvent,
    ) {
        match event {
            PlatformWindowEvent::RedrawRequested => self.present_latest(),
            PlatformWindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.handle.send(WindowMsg::Window(WindowEvent::Resized {
                        width: size.width,
                        height: size.height,
                    }));
                }
            }
            PlatformWindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle
                    .send(WindowMsg::Window(WindowEvent::ScaleFactorChanged(
                        scale_factor,
                    )));
            }
            // The close-confirmation chain decides: the window stays open
            // until the RT loop quits, so a veto can win.
            PlatformWindowEvent::CloseRequested => {
                self.handle
                    .send(WindowMsg::Window(WindowEvent::CloseRequested));
            }
            _ => {
                if let Some(input) = self.translator.translate(&event) {
                    self.handle.send(WindowMsg::Input(input));
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, (): ()) {
        // A wake is either a frame-ready (request a redraw → present) or the
        // RT loop's exit signal (carry its quit code out).
        if let Ok(code) = self.code_rx.try_recv() {
            self.final_code = Some(code);
            event_loop.exit();
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use rt_core::{Object, object_impl};

    use crate::input::InputEvent;

    /// Records every painted frame (headless stand-in for the channel sink).
    struct TestSink {
        frames: Vec<Vec<u32>>,
    }

    impl TestSink {
        fn new() -> Self {
            Self { frames: Vec::new() }
        }
    }

    impl FrameSink for TestSink {
        fn ship(&mut self, frame: Vec<u32>) {
            self.frames.push(frame);
        }
    }

    fn distinct_colors(frame: &[u32]) -> usize {
        frame.iter().collect::<std::collections::HashSet<_>>().len()
    }

    fn run_headless<A: WindowApplication<TestMsg>>(
        app: A,
        close_exit_code: i32,
        bootstrap: impl FnOnce(&mut WindowShell, &AppHandle<WindowMsg<TestMsg>>),
    ) -> (PresenterApp<A, TestMsg, TestSink>, i32) {
        let (app_rt, handle) = App::<WindowMsg<TestMsg>>::new();
        let mut presenter = PresenterApp {
            shell: WindowShell::new(320, 240),
            app,
            sink: TestSink::new(),
            close_exit_code,
            _msg: PhantomData,
        };
        bootstrap(&mut presenter.shell, &handle);
        app_rt.run(presenter)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestMsg {
        Start,
    }

    /// Bootstrap connects this to the Window; deliveries prove bootstrap ran
    /// before the loop pumped its first message.
    #[derive(Object)]
    struct WindowObserver {
        resizes: Vec<(u32, u32)>,
        closes: u32,
    }

    #[object_impl]
    impl WindowObserver {
        #[slot]
        fn on_resized(&mut self, width: u32, height: u32) {
            self.resizes.push((width, height));
        }

        #[slot]
        fn on_close_requested(&mut self) {
            self.closes += 1;
        }
    }

    /// Drives the demo flow headlessly: timers click button 2, then request
    /// and confirm the close. Same choreography as the unattended example.
    struct DemoApp {
        click_timer: Option<TimerId>,
        close_timer: Option<TimerId>,
        clicked: bool,
        requested: bool,
        confirmed: bool,
        cycles: u32,
        shutdown_code: Option<i32>,
    }

    impl DemoApp {
        fn new() -> Self {
            Self {
                click_timer: None,
                close_timer: None,
                clicked: false,
                requested: false,
                confirmed: false,
                cycles: 0,
                shutdown_code: None,
            }
        }
    }

    fn click_second_button(shell: &mut WindowShell) {
        let rect = shell
            .gui()
            .rect_of(shell.gui().button_id(1))
            .expect("button 2 laid out");
        let center = (
            (rect.x0 + rect.width() / 2.0) as i32,
            (rect.y0 + rect.height() / 2.0) as i32,
        );
        shell.handle_input(InputEvent::MouseMoved {
            x: center.0,
            y: center.1,
        });
        shell.handle_input(InputEvent::MousePressed {
            x: center.0,
            y: center.1,
        });
        shell.handle_input(InputEvent::MouseReleased {
            x: center.0,
            y: center.1,
        });
    }

    impl WindowApplication<TestMsg> for DemoApp {
        fn on_window_message(
            &mut self,
            ctx: &mut Ctx<WindowMsg<TestMsg>>,
            msg: WindowMsg<TestMsg>,
            _shell: &mut WindowShell,
        ) {
            if msg == WindowMsg::User(TestMsg::Start) {
                // Offset, not ZERO: the pump fires timers before the cycle
                // hook, so a ZERO timer would run ahead of the first layout.
                self.click_timer = Some(ctx.after(Duration::from_millis(1)));
                self.close_timer = Some(ctx.after(Duration::from_millis(2)));
            }
        }

        fn on_window_timer(
            &mut self,
            _ctx: &mut Ctx<WindowMsg<TestMsg>>,
            timer: TimerId,
            shell: &mut WindowShell,
        ) {
            if Some(timer) == self.click_timer {
                click_second_button(shell);
                assert_eq!(shell.gui().click_count(), 1);
                self.clicked = true;
                let clicks = shell.gui().click_count();
                shell.gui_mut().set_label(format!("clicks: {clicks}"));
            } else if Some(timer) == self.close_timer {
                shell.handle_window_event(WindowEvent::CloseRequested);
                self.requested = true;
                assert!(shell.window_mut().confirm_close());
                self.confirmed = true;
            }
        }

        fn on_window_cycle(&mut self, _ctx: &mut Ctx<WindowMsg<TestMsg>>, shell: &mut WindowShell) {
            self.cycles += 1;
            if self.clicked {
                let frame = shell.gui().frame();
                assert_eq!(
                    frame.len(),
                    (shell.window().width() * shell.window().height()) as usize
                );
                assert!(
                    distinct_colors(frame) > 1,
                    "frame must show painted content"
                );
            }
        }

        fn on_window_shutdown(&mut self, code: i32) {
            self.shutdown_code = Some(code);
        }
    }

    #[test]
    fn demo_flow_bootstrap_order_close_chain_quit_code_and_frames() {
        let observer_slot = std::cell::RefCell::new(None);
        let bootstrap_observer = &observer_slot;
        let (presenter, code) = run_headless(
            DemoApp::new(),
            7,
            |shell: &mut WindowShell, handle: &AppHandle<WindowMsg<TestMsg>>| {
                // Compose: the default root column gets a second button.
                shell.gui_mut().push_button("Button 2");
                // Connect the close/resize observer before anything pumps.
                let observer = shell.hub_mut().insert(WindowObserver {
                    resizes: Vec::new(),
                    closes: 0,
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
                    Window::CLOSE_REQUESTED,
                    observer,
                    WindowObserver::on_close_requested_slot,
                );
                *bootstrap_observer.borrow_mut() = Some(observer);
                handle.send(WindowMsg::User(TestMsg::Start));
            },
        );

        // Quit code propagates through the close chain: confirm → quit(7).
        assert_eq!(code, 7);
        assert!(presenter.app.clicked);
        assert!(presenter.app.requested);
        assert!(presenter.app.confirmed);
        assert_eq!(presenter.app.shutdown_code, Some(7));
        assert_eq!(presenter.shell.gui().click_count(), 1);

        // Bootstrap ordering: the observer was connected before the loop
        // pumped, so the close-request signal reached it exactly once.
        let observer = observer_slot.into_inner().expect("observer handle");
        let observed = presenter
            .shell
            .hub
            .get::<WindowObserver>(observer)
            .expect("observer alive");
        assert_eq!(observed.closes, 1);

        // Frames shipped: the initial paint, then click + label damage
        // coalesced into one repaint (one Flush step per cycle).
        assert_eq!(presenter.sink.frames.len(), 2);
        let last = presenter.sink.frames.last().unwrap();
        assert!(distinct_colors(last) > 1);
        // Nothing left dirty at exit: the last cycle painted everything.
        assert!(!presenter.shell.gui().is_dirty());
    }

    /// Veto keeps the app running: the first close request is refused, a
    /// re-request (posted to the next cycle) is confirmed, only then quits.
    struct VetoApp {
        requests: u32,
        vetoed: bool,
        confirmed: bool,
    }

    impl VetoApp {
        /// Post the close ask and arm a 1ms wake: the pump block only wakes
        /// on external messages or timer deadlines, never on self-posts, and
        /// a same-iteration (ZERO) deadline never reaches the block step.
        fn ask(ctx: &mut Ctx<WindowMsg<TestMsg>>) {
            ctx.post(WindowMsg::Window(WindowEvent::CloseRequested));
            ctx.after(Duration::from_millis(1));
        }
    }

    impl WindowApplication<TestMsg> for VetoApp {
        fn on_window_message(
            &mut self,
            ctx: &mut Ctx<WindowMsg<TestMsg>>,
            msg: WindowMsg<TestMsg>,
            shell: &mut WindowShell,
        ) {
            if msg == WindowMsg::User(TestMsg::Start) {
                Self::ask(ctx);
            } else if let WindowMsg::Window(WindowEvent::CloseRequested) = msg {
                self.requests += 1;
                if self.requests == 1 {
                    assert!(shell.window_mut().veto_close());
                    self.vetoed = true;
                    Self::ask(ctx);
                } else {
                    assert!(shell.window_mut().confirm_close());
                    self.confirmed = true;
                }
            }
        }

        fn on_window_cycle(
            &mut self,
            _ctx: &mut Ctx<WindowMsg<TestMsg>>,
            _shell: &mut WindowShell,
        ) {
        }
    }

    #[test]
    fn veto_keeps_running_until_a_confirmed_close() {
        let (presenter, code) = run_headless(
            VetoApp {
                requests: 0,
                vetoed: false,
                confirmed: false,
            },
            5,
            |_, handle: &AppHandle<WindowMsg<TestMsg>>| {
                handle.send(WindowMsg::User(TestMsg::Start));
            },
        );
        assert!(presenter.app.vetoed);
        assert!(presenter.app.confirmed);
        assert_eq!(
            presenter.app.requests, 2,
            "veto must not quit; re-request must ask again"
        );
        assert_eq!(code, 5);
        assert_eq!(
            presenter.shell.window().close_state(),
            CloseState::CloseConfirmed
        );
    }

    #[test]
    fn a_pending_close_that_is_never_answered_keeps_the_window_open() {
        struct IdleApp;
        impl WindowApplication<TestMsg> for IdleApp {
            fn on_window_message(
                &mut self,
                _ctx: &mut Ctx<WindowMsg<TestMsg>>,
                _msg: WindowMsg<TestMsg>,
                _shell: &mut WindowShell,
            ) {
            }

            fn on_window_cycle(
                &mut self,
                _ctx: &mut Ctx<WindowMsg<TestMsg>>,
                _shell: &mut WindowShell,
            ) {
            }
        }
        let (app_rt, handle) = App::<WindowMsg<TestMsg>>::new();
        let mut presenter = PresenterApp {
            shell: WindowShell::new(320, 240),
            app: IdleApp,
            sink: TestSink::new(),
            close_exit_code: 0,
            _msg: PhantomData,
        };
        presenter
            .shell
            .handle_window_event(WindowEvent::CloseRequested);
        assert_eq!(
            presenter.shell.window().close_state(),
            CloseState::PendingClose
        );
        drop(handle);
        let (presenter, code) = app_rt.run(presenter);
        assert_eq!(code, 0);
        assert_eq!(
            presenter.shell.window().close_state(),
            CloseState::PendingClose,
            "an unanswered close stays pending; nothing force-quits"
        );
    }
}
