//! App application layer (#13/#34): typed messages, self-posts, timers,
//! cycle hook, shutdown hook, quit — with state that never crosses a
//! thread boundary.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::thread;
use std::time::Duration;

use rt_app::{App, Application, Ctx, TimerId};

struct Counter {
    seen: Vec<u32>,
    cycles: u32,
    timer_hits: Vec<TimerId>,
    quit_after: Option<u32>,
}

impl Application for Counter {
    type Msg = u32;

    fn on_message(&mut self, ctx: &mut Ctx<u32>, msg: u32) {
        self.seen.push(msg);
        if self.seen.len() as u32 == self.quit_after.unwrap_or(u32::MAX) {
            ctx.quit(7);
        }
    }

    fn on_timer(&mut self, _ctx: &mut Ctx<u32>, timer: TimerId) {
        self.timer_hits.push(timer);
    }

    fn on_cycle(&mut self, _ctx: &mut Ctx<u32>) {
        self.cycles += 1;
    }
}

#[test]
fn messages_reach_state_and_quit_returns_the_code() {
    let (app, handle) = App::new();
    handle.send(1);
    handle.send(2);
    drop(handle);

    let (counter, code) = app.run(Counter {
        seen: vec![],
        cycles: 0,
        timer_hits: vec![],
        quit_after: Some(2),
    });
    assert_eq!(counter.seen, [1, 2]);
    assert_eq!(code, 7);
    // Quit fired during the drain, so the cycle ended before the hook —
    // matching EventLoop::run, where a quit in the drain skips flush().
    assert_eq!(counter.cycles, 0);
}

#[test]
fn cross_thread_sends_arrive_in_order() {
    let (app, handle) = App::new();
    let sender = thread::spawn(move || {
        for i in 0..5 {
            handle.send(i);
        }
    });

    let (counter, _) = app.run(Counter {
        seen: vec![],
        cycles: 0,
        timer_hits: vec![],
        quit_after: Some(5),
    });
    sender.join().unwrap();
    assert_eq!(counter.seen, [0, 1, 2, 3, 4]);
}

#[test]
fn dropped_handles_return_zero_when_idle() {
    let (app, handle) = App::<u32>::new();
    drop(handle);
    let (counter, code) = app.run(Counter {
        seen: vec![],
        cycles: 0,
        timer_hits: vec![],
        quit_after: None,
    });
    assert!(counter.seen.is_empty());
    assert_eq!(code, 0);
}

#[test]
fn one_shot_timer_fires_once_and_can_quit() {
    struct Once {
        id: Option<TimerId>,
        hits: u32,
    }
    impl Application for Once {
        type Msg = ();
        fn on_message(&mut self, _ctx: &mut Ctx<()>, _msg: ()) {}
        fn on_cycle(&mut self, ctx: &mut Ctx<()>) {
            if self.id.is_none() {
                self.id = Some(ctx.after(Duration::from_millis(20)));
            }
        }
        fn on_timer(&mut self, ctx: &mut Ctx<()>, timer: TimerId) {
            assert_eq!(Some(timer), self.id);
            self.hits += 1;
            ctx.quit(3);
        }
    }

    let (app, handle) = App::new();
    drop(handle); // disconnected; the armed timer still keeps the loop alive
    let (once, code) = app.run(Once {
        id: None,
        hits: 0,
    });
    assert_eq!(code, 3);
    assert_eq!(once.hits, 1);
}

#[test]
fn repeating_timer_fires_until_cancelled() {
    struct Tick {
        timer: Option<TimerId>,
        hits: u32,
    }
    impl Application for Tick {
        type Msg = ();
        fn on_message(&mut self, _ctx: &mut Ctx<()>, _msg: ()) {}
        fn on_cycle(&mut self, ctx: &mut Ctx<()>) {
            if self.timer.is_none() {
                self.timer = Some(ctx.every(Duration::from_millis(15)));
            }
        }
        fn on_timer(&mut self, ctx: &mut Ctx<()>, timer: TimerId) {
            assert_eq!(self.timer, Some(timer));
            self.hits += 1;
            if self.hits >= 3 {
                ctx.cancel_timer(timer);
                ctx.quit(0);
            }
        }
    }

    let (app, handle) = App::new();
    drop(handle); // disconnected; the timer keeps the loop alive
    let (tick, out) = app.run(Tick {
        timer: None,
        hits: 0,
    });
    assert_eq!(out, 0);
    assert_eq!(tick.hits, 3);
}

#[test]
fn cli_shape_is_small() {
    // The promise: a whole CLI app in a few lines, state on this thread.
    struct Echo {
        total: u32,
    }
    impl Application for Echo {
        type Msg = u32;
        fn on_message(&mut self, ctx: &mut Ctx<u32>, n: u32) {
            self.total += n;
            if self.total >= 10 {
                ctx.quit(self.total as i32);
            }
        }
    }

    let (app, handle) = App::new();
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let sink = observed.clone();
    thread::spawn(move || {
        for n in [1u32, 2, 3, 4] {
            handle.send(n);
            sink.lock().unwrap().push(n);
        }
    });

    let (echo, code) = app.run(Echo { total: 0 });
    assert_eq!(echo.total, 10);
    assert_eq!(code, 10);
    assert_eq!(*observed.lock().unwrap(), [1, 2, 3, 4]);
}

#[test]
fn try_send_after_exit_hands_the_message_back() {
    struct QuitNow;
    impl Application for QuitNow {
        type Msg = u32;
        fn on_message(&mut self, _ctx: &mut Ctx<u32>, _msg: u32) {}
        fn on_cycle(&mut self, ctx: &mut Ctx<u32>) {
            ctx.quit(4);
        }
    }

    let (app, handle) = App::new();
    let (_, code) = app.run(QuitNow);
    assert_eq!(code, 4);
    assert_eq!(handle.try_send(42), Err(42));
}

#[test]
fn post_defers_to_a_later_cycle_in_fifo_order() {
    struct Post {
        got: Vec<u32>,
        at_cycle: Vec<u32>,
        cycles: u32,
        posted: bool,
    }
    impl Application for Post {
        type Msg = u32;
        fn on_message(&mut self, ctx: &mut Ctx<u32>, msg: u32) {
            self.at_cycle.push(self.cycles);
            self.got.push(msg);
            if !self.posted {
                self.posted = true;
                ctx.post(10);
                ctx.post(20);
            }
            if self.got.len() == 4 {
                ctx.quit(4);
            }
        }
        fn on_cycle(&mut self, _ctx: &mut Ctx<u32>) {
            self.cycles += 1;
        }
    }

    let (app, handle) = App::new();
    handle.send(1);
    handle.send(2);
    drop(handle);
    let (post, code) = app.run(Post {
        got: vec![],
        at_cycle: vec![],
        cycles: 0,
        posted: false,
    });
    assert_eq!(code, 4);
    assert_eq!(post.got, [1, 2, 10, 20]);
    // 1 and 2 delivered in cycle 1 (the cycle counter is still 0); the
    // self-posts land in cycle 2 — deferral is structural, not best-effort.
    assert_eq!(post.at_cycle, [0, 0, 1, 1]);
}

#[test]
fn on_shutdown_runs_once_with_the_quit_code() {
    struct Probe {
        codes: Vec<i32>,
    }
    impl Application for Probe {
        type Msg = u32;
        fn on_message(&mut self, ctx: &mut Ctx<u32>, _msg: u32) {
            ctx.quit(7);
        }
        fn on_shutdown(&mut self, code: i32) {
            self.codes.push(code);
        }
    }

    let (app, handle) = App::new();
    handle.send(1);
    drop(handle);
    let (probe, code) = app.run(Probe { codes: vec![] });
    assert_eq!(code, 7);
    assert_eq!(probe.codes, [7]);
}

#[test]
fn on_shutdown_runs_on_natural_drain_with_zero() {
    struct Probe {
        codes: Vec<i32>,
    }
    impl Application for Probe {
        type Msg = u32;
        fn on_message(&mut self, _ctx: &mut Ctx<u32>, _msg: u32) {}
        fn on_shutdown(&mut self, code: i32) {
            self.codes.push(code);
        }
    }

    let (app, handle) = App::<u32>::new();
    drop(handle);
    let (probe, code) = app.run(Probe { codes: vec![] });
    assert_eq!(code, 0);
    assert_eq!(probe.codes, [0]);
}
