//! Acceptance demo (#10), window leg: the full stack visible on screen.
//!
//! Bridge (as proven by the #9 spike, `examples/window.rs`):
//! - winit owns the main thread; softbuffer presents.
//! - The RT domain (Hub + objects + Gui) lives on a dedicated pump thread —
//!   it is single-threaded and not `Send` (ADR 0003's thread line), so input
//!   crosses as messages into an inbox, and frames cross back through a slot
//!   plus an `EventLoopProxy` wake-up.
//! - rt-app's `EventLoop` hosting `Send` work (and the whole object chain as
//!   posted work) is exercised headlessly in `tests/acceptance.rs`.
//!
//! The demo auto-plays: after startup it synthesizes a click inside the
//! button, the label increments through the object model, and the window
//! auto-quits after a few seconds. Real clicks work too.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rt_core::{Hub, Object, Property, RegistryBuilder, object_impl};
use rt_gui::{Gui, InputEvent, InputTranslator};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

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

enum ToWinit {
    FrameReady,
    Quit,
}

/// Shared wiring between the winit thread and the RT pump thread.
struct Wiring {
    inbox: Mutex<Vec<InputEvent>>,
    inbox_signal: Condvar,
    frame: Mutex<Option<Vec<u32>>>,
    quit: AtomicBool,
    clicks_seen: Mutex<u32>,
}

fn main() {
    let wiring = Arc::new(Wiring {
        inbox: Mutex::new(Vec::new()),
        inbox_signal: Condvar::new(),
        frame: Mutex::new(None),
        quit: AtomicBool::new(false),
        clicks_seen: Mutex::new(0),
    });

    let event_loop = EventLoop::<ToWinit>::with_user_event()
        .build()
        .expect("winit event loop");
    let proxy = event_loop.create_proxy();

    // RT domain thread: registry fill, objects, typed connections, then the
    // pump — drain input, drive the chain, paint, ship the frame.
    let rt_wiring = wiring.clone();
    let rt_proxy = proxy.clone();
    std::thread::spawn(move || {
        let mut registry = RegistryBuilder::new();
        registry.register::<Panel>().register::<Counter>().register::<Bridge>();
        let _registry = registry.build();

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

        let mut gui = Gui::new(320, 240);
        gui.set_label("clicks: 0");

        // Auto-play: one synthetic click so the demo shows itself.
        let b = gui.button_rect();
        let (x, y) = (b.top_left.x + 5, b.top_left.y + 5);
        let mut inbox = rt_wiring.inbox.lock().unwrap();
        inbox.extend([
            InputEvent::MouseMoved { x, y },
            InputEvent::MousePressed { x, y },
            InputEvent::MouseReleased { x, y },
        ]);
        drop(inbox);
        rt_wiring.inbox_signal.notify_all();

        let started = std::time::Instant::now();
        loop {
            if rt_wiring.quit.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(4)
            {
                let _ = rt_proxy.send_event(ToWinit::Quit);
                return;
            }

            let events = {
                let mut inbox = rt_wiring.inbox.lock().unwrap();
                std::mem::take(&mut *inbox)
            };
            for event in &events {
                gui.handle_input(event);
            }

            let mut seen = rt_wiring.clicks_seen.lock().unwrap();
            if gui.click_count() > *seen {
                *seen = gui.click_count();
                drop(seen);
                hub.emit(panel, Panel::BUTTON_CLICKED, ());
                while hub.flush() > 0 {}
                gui.set_label(hub.get::<Bridge>(bridge).unwrap().label.clone());
            }

            if let Some(frame) = gui.paint() {
                *rt_wiring.frame.lock().unwrap() = Some(frame.to_vec());
                let _ = rt_proxy.send_event(ToWinit::FrameReady);
            }

            // Park until new input arrives (no busy spin beyond wakeups).
            let timeout = Duration::from_millis(15);
            let inbox = rt_wiring.inbox.lock().unwrap();
            if inbox.is_empty() {
                let (guard, _) = rt_writing_wait(&rt_wiring, inbox, timeout);
                drop(guard);
            }
        }
    });

    let mut app = Presenter {
        wiring: wiring.clone(),
        window: None,
        surface: None,
    };
    let _ = event_loop.run_app(&mut app);
}

fn rt_writing_wait<'a>(
    wiring: &'a Wiring,
    guard: std::sync::MutexGuard<'a, Vec<InputEvent>>,
    timeout: Duration,
) -> (std::sync::MutexGuard<'a, Vec<InputEvent>>, std::sync::WaitTimeoutResult) {
    wiring.inbox_signal.wait_timeout(guard, timeout).unwrap()
}

struct Presenter {
    wiring: Arc<Wiring>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
}

impl Presenter {
    fn present_latest(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let Some(frame) = self.wiring.frame.lock().unwrap().take() else {
            return;
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        surface.resize(w, h).expect("surface resize");
        let mut buffer = surface.buffer_mut().expect("buffer");
        if buffer.len() == frame.len() {
            buffer.copy_from_slice(&frame);
            buffer.present().expect("present");
        }
    }
}

impl ApplicationHandler<ToWinit> for Presenter {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("RT acceptance demo — click counter")
                        .with_inner_size(LogicalSize::new(320, 240)),
                )
                .expect("create window"),
        );
        let context = softbuffer::Context::new(window.clone()).expect("context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event == WindowEvent::RedrawRequested {
            self.present_latest();
        }
        let mut translator = InputTranslator::new();
        if let Some(input) = translator.translate(&event) {
            if let InputEvent::CloseRequested = input {
                self.wiring.quit.store(true, Ordering::Release);
                event_loop.exit();
                return;
            }
            self.wiring.inbox.lock().unwrap().push(input);
            self.wiring.inbox_signal.notify_all();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, msg: ToWinit) {
        match msg {
            ToWinit::FrameReady => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            ToWinit::Quit => event_loop.exit(),
        }
    }
}
