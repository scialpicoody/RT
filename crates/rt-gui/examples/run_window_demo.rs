//! Unattended `run_window` demo (ticket #22): drives the real presenter end
//! to end on the CPU presentation path — the window opens, a timer clicks
//! button 2 programmatically through the shell's hit-tested input routing,
//! a close request is confirmed through the close-confirmation chain, and
//! the process exits 0. Painted frames are asserted every cycle.
//!
//! No hand-rolled bridges: every thread crossing is a
//! [`WindowMsg`](rt_gui::WindowMsg) into the loop or a painted frame out.

use std::collections::HashSet;
use std::time::Duration;

use rt_app::{AppHandle, Ctx, TimerId};
use rt_gui::input::InputEvent;
use rt_gui::presenter::{run_window, WindowApplication, WindowConfig, WindowMsg, WindowShell};
use rt_gui::window::WindowEvent;

/// Bootstrap sends this once the loop thread is up; the handler arms the
/// demo timers (`Ctx` is only reachable in callbacks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Demo {
    Start,
}

struct DemoApp {
    click_timer: Option<TimerId>,
    close_timer: Option<TimerId>,
    clicked: bool,
}

impl WindowApplication<Demo> for DemoApp {
    fn on_window_message(
        &mut self,
        ctx: &mut Ctx<WindowMsg<Demo>>,
        msg: WindowMsg<Demo>,
        _shell: &mut WindowShell,
    ) {
        if msg == WindowMsg::User(Demo::Start) {
            // ~300ms: click button 2; ~600ms: request + confirm the close.
            self.click_timer = Some(ctx.after(Duration::from_millis(300)));
            self.close_timer = Some(ctx.after(Duration::from_millis(600)));
        }
    }

    fn on_window_timer(&mut self, _ctx: &mut Ctx<WindowMsg<Demo>>, timer: TimerId, shell: &mut WindowShell) {
        if Some(timer) == self.click_timer {
            // Click button 2 programmatically: synthetic mouse input routed
            // through the shell (hit test on post-layout geometry).
            let rect = shell
                .gui()
                .rect_of(shell.gui().button_id(1))
                .expect("button 2 laid out");
            let (x, y) = (
                (rect.x0 + rect.width() / 2.0) as i32,
                (rect.y0 + rect.height() / 2.0) as i32,
            );
            shell.handle_input(InputEvent::MouseMoved { x, y });
            shell.handle_input(InputEvent::MousePressed { x, y });
            shell.handle_input(InputEvent::MouseReleased { x, y });
            let clicks = shell.gui().click_count();
            assert_eq!(clicks, 1, "programmatic click on button 2 must land");
            shell
                .gui_mut()
                .set_label(format!("clicks: {clicks} — closing soon"));
            self.clicked = true;
        } else if Some(timer) == self.close_timer {
            // The close-confirmation chain: request (as the window manager
            // would), then the application confirms.
            shell.handle_window_event(WindowEvent::CloseRequested);
            assert!(shell.window_mut().confirm_close(), "close must be pending");
        }
    }

    fn on_window_cycle(&mut self, _ctx: &mut Ctx<WindowMsg<Demo>>, shell: &mut WindowShell) {
        // Assert the painted frame on the real CPU presentation path.
        let frame = shell.gui().frame();
        assert_eq!(
            frame.len(),
            (shell.window().width() * shell.window().height()) as usize,
            "frame must match the window's physical size"
        );
        if self.clicked {
            let distinct = frame.iter().collect::<HashSet<_>>().len();
            assert!(
                distinct > 2,
                "frame must show background, buttons, and label pixels"
            );
        }
    }
}

fn main() {
    let code = run_window(
        WindowConfig {
            title: "rt-gui run_window demo".to_owned(),
            width: 480,
            height: 320,
            ..WindowConfig::default()
        },
        DemoApp {
            click_timer: None,
            close_timer: None,
            clicked: false,
        },
        |shell: &mut WindowShell, handle: &AppHandle<WindowMsg<Demo>>| {
            // Two buttons in the root column (Gui starts with one).
            shell.gui_mut().push_button("Button 2");
            shell.gui_mut().set_label("auto-playing: click + close");
            handle.send(WindowMsg::User(Demo::Start));
        },
    );
    println!("run_window demo exited with code {code}");
    assert_eq!(code, 0, "the close chain must quit with the configured code");
}
