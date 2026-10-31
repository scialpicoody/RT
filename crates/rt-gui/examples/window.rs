//! Spike: prove the winit <-> rt-app bridge before building the real crate API.
//!
//! Bridge (intended final shape):
//! - winit owns the main thread (`EventLoop::run_app`).
//! - rt_app::EventLoop runs on a second thread; all RT state lives there.
//! - Input: winit WindowEvents post work through rt_app::EventSender.
//! - Output: the rt-app thread paints into an owned Vec<u32> frame, drops it in
//!   a shared slot, and wakes winit via EventLoopProxy::send_event; the main
//!   thread blits the frame in RedrawRequested and presents via softbuffer.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

/// Main-thread-bound wake-ups from the rt-app loop thread.
#[derive(Debug)]
enum ToWinit {
    FrameReady,
    Quit,
}

static TICKS: AtomicUsize = AtomicUsize::new(0);

/// App-thread side: paint the scene into a fresh frame and ship it to winit.
fn ship_frame(scene: &Arc<Mutex<Scene>>, slot: Arc<Mutex<Option<Vec<u32>>>>, proxy: EventLoopProxy<ToWinit>) {
    let s = scene.lock().unwrap();
    let red = s.cursor_x.clamp(0, 255) as u32;
    let frame: Vec<u32> = vec![red << 16 | 0x00_20_40; s.w.max(1) * s.h.max(1)];
    drop(s);
    *slot.lock().unwrap() = Some(frame);
    let _ = proxy.send_event(ToWinit::FrameReady);
}

/// RT state, owned by the rt-app loop thread (Mutex only to satisfy `Send`
/// for posted closures; only the loop thread ever locks it).
#[derive(Default, Debug)]
struct Scene {
    cursor_x: i32,
    w: usize,
    h: usize,
}

struct Spike {
    sender: rt_app::EventSender,
    scene: Arc<Mutex<Scene>>,
    frame_slot: Arc<Mutex<Option<Vec<u32>>>>,
    proxy: EventLoopProxy<ToWinit>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
}

impl Spike {
    fn present_latest(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let Some(frame) = self.frame_slot.lock().unwrap().take() else {
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

impl ApplicationHandler<ToWinit> for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("rt-gui spike")
                    .with_inner_size(LogicalSize::new(320, 240)),
            )
            .expect("create window");
        let size = window.inner_size();
        let (w, h) = (size.width as usize, size.height as usize);
        let sender = self.sender.clone();
        let scene = self.scene.clone();
        let slot = self.frame_slot.clone();
        let proxy = self.proxy.clone();
        sender.call(move || {
            {
                let mut s = scene.lock().unwrap();
                s.w = w;
                s.h = h;
            }
            ship_frame(&scene, slot, proxy);
        });
        let window = Arc::new(window);
        let context = softbuffer::Context::new(window.clone()).expect("context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.sender.quit(0);
                event_loop.exit();
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Input leg: post into the rt-app loop; repaint flows back.
                let sender = self.sender.clone();
                let scene = self.scene.clone();
                let slot = self.frame_slot.clone();
                let proxy = self.proxy.clone();
                let x = position.x as i32;
                sender.call(move || {
                    scene.lock().unwrap().cursor_x = x;
                    ship_frame(&scene, slot, proxy);
                });
            }
            WindowEvent::RedrawRequested => {
                self.present_latest();
            }
            _ => {}
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

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Safety net for wake-ups that raced window creation: present any
        // frame left in the slot.
        if self.frame_slot.lock().unwrap().is_some()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}

fn main() {
    let (loop_, sender) = rt_app::EventLoop::new();

    let event_loop = EventLoop::<ToWinit>::with_user_event()
        .build()
        .expect("winit event loop");
    let proxy = event_loop.create_proxy();

    // rt-app loop thread: paint the initial frame, then serve posted input.
    let scene = Arc::new(Mutex::new(Scene::default()));
    let frame_slot: Arc<Mutex<Option<Vec<u32>>>> = Arc::new(Mutex::new(None));
    let app_sender = sender.clone();
    let app_proxy = proxy.clone();
    std::thread::spawn(move || {
        // Auto-quit so the spike runs unattended.
        app_sender.add_repeating(Duration::from_millis(500), {
            let sender = app_sender.clone();
            move |_| {
                if TICKS.fetch_add(1, Ordering::Relaxed) >= 3 {
                    sender.quit(0);
                }
            }
        });
        let code = loop_.run();
        println!("rt-app loop exited with {code}");
        let _ = app_proxy.send_event(ToWinit::Quit);
    });

    let mut app = Spike {
        sender,
        scene,
        frame_slot,
        proxy,
        window: None,
        surface: None,
    };

    let started = std::time::Instant::now();
    let result = event_loop.run_app(&mut app);
    println!(
        "winit exited after {:?}, result={result:?}",
        started.elapsed()
    );
}
