//! Behavior tests for the rt-app event loop, exercised only through its
//! public seam: `EventLoop`, `EventSender`, `TimerHandle`.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use rt_app::{Event, EventLoop};

#[test]
fn quit_via_posted_event_returns_exit_code() {
    let (loop_, sender) = EventLoop::new();
    sender.post_event(Event::Quit { code: 7 });
    assert_eq!(loop_.run(), 7);
}

#[test]
fn quit_from_within_loop_via_sender_handle() {
    let (loop_, sender) = EventLoop::new();
    sender.call({
        let sender = sender.clone();
        move || sender.quit(3)
    });
    assert_eq!(loop_.run(), 3);
}

#[test]
fn events_delivered_in_fifo_order() {
    let (loop_, sender) = EventLoop::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    for i in 0..10 {
        let order = order.clone();
        sender.call(move || order.lock().unwrap().push(i));
    }
    sender.quit(0);
    loop_.run();
    assert_eq!(*order.lock().unwrap(), (0..10).collect::<Vec<_>>());
}

#[test]
fn cross_thread_post_wakes_idle_blocked_loop() {
    let (loop_, sender) = EventLoop::new();
    let quitter = sender.clone();
    let handle = std::thread::spawn(move || loop_.run());

    // Give the loop generous time to go idle and block.
    std::thread::sleep(Duration::from_millis(200));

    let delivered = Arc::new(Mutex::new(false));
    let flag = delivered.clone();
    let poster = sender.clone();
    std::thread::spawn(move || {
        poster.call(move || *flag.lock().unwrap() = true);
        poster.quit(0);
    });

    join_within(handle, Duration::from_secs(10));
    assert!(
        *delivered.lock().unwrap(),
        "event posted cross-thread was never delivered"
    );
    let _ = quitter; // sender kept alive until the loop exits
}

fn join_within(handle: std::thread::JoinHandle<i32>, limit: Duration) -> i32 {
    let deadline = std::time::Instant::now() + limit;
    while !handle.is_finished() {
        if std::time::Instant::now() > deadline {
            panic!("event loop did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    handle.join().expect("event loop thread panicked")
}

#[test]
fn one_shot_timer_fires_after_delay_with_margin() {
    let (loop_, sender) = EventLoop::new();
    let fired_at = Arc::new(Mutex::new(None));
    let recorded = fired_at.clone();
    sender.add_one_shot(Duration::from_millis(100), {
        let sender = sender.clone();
        move || {
            *recorded.lock().unwrap() = Some(std::time::Instant::now());
            sender.quit(0);
        }
    });
    let started = std::time::Instant::now();
    let handle = std::thread::spawn(move || loop_.run());

    let code = join_within(handle, Duration::from_secs(10));
    assert_eq!(code, 0);
    let fired = fired_at.lock().unwrap().expect("one-shot never fired");
    let elapsed = fired.duration_since(started);
    // Never early; generous upper margin for scheduler jitter.
    assert!(
        elapsed >= Duration::from_millis(95),
        "fired early: {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_millis(2000),
        "fired late: {elapsed:?}"
    );
}

#[test]
fn repeating_timer_fires_periodically_until_self_cancel() {
    let (loop_, sender) = EventLoop::new();
    let interval = Duration::from_millis(60);
    let fires = Arc::new(Mutex::new(Vec::new()));
    sender.add_repeating(interval, {
        let fires = fires.clone();
        let sender = sender.clone();
        move |timer| {
            let mut fires = fires.lock().unwrap();
            fires.push(std::time::Instant::now());
            if fires.len() == 3 {
                drop(fires);
                timer.cancel();
                sender.quit(0);
            }
        }
    });
    let runner = std::thread::spawn(move || loop_.run());
    let code = join_within(runner, Duration::from_secs(10));

    assert_eq!(code, 0);
    let fires = fires.lock().unwrap();
    assert_eq!(
        fires.len(),
        3,
        "repeating timer fired {} times",
        fires.len()
    );
    let span = fires[2].duration_since(fires[0]);
    // Two intervals between first and third fire; tolerant margins.
    assert!(span >= Duration::from_millis(100), "too fast: {span:?}");
    assert!(span <= Duration::from_millis(1000), "too slow: {span:?}");
}

#[test]
fn cancel_from_another_thread_stops_repeating_timer() {
    let (loop_, sender) = EventLoop::new();
    let count = Arc::new(Mutex::new(0u32));
    let handle = sender.add_repeating(Duration::from_millis(50), {
        let count = count.clone();
        move |_| {
            *count.lock().unwrap() += 1;
        }
    });
    let runner = std::thread::spawn(move || loop_.run());

    // Let at least one fire happen, then cancel from this thread.
    std::thread::sleep(Duration::from_millis(130));
    handle.cancel();
    let at_cancel = *count.lock().unwrap();
    assert!(at_cancel >= 1, "timer never fired before cancel");

    std::thread::sleep(Duration::from_millis(250));
    sender.quit(0);
    assert_eq!(join_within(runner, Duration::from_secs(10)), 0);
    assert_eq!(
        *count.lock().unwrap(),
        at_cancel,
        "timer fired after cancel"
    );
}

#[test]
fn queued_events_deliver_before_newly_armed_timer_fires() {
    let (loop_, sender) = EventLoop::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    sender.call({
        let order = order.clone();
        let sender = sender.clone();
        move || {
            order.lock().unwrap().push("A");
            sender.add_one_shot(Duration::from_millis(20), {
                let order = order.clone();
                let sender = sender.clone();
                move || {
                    order.lock().unwrap().push("T");
                    sender.quit(0);
                }
            });
        }
    });
    sender.call({
        let order = order.clone();
        move || order.lock().unwrap().push("B")
    });
    let handle = std::thread::spawn(move || loop_.run());

    assert_eq!(join_within(handle, Duration::from_secs(10)), 0);
    assert_eq!(*order.lock().unwrap(), vec!["A", "B", "T"]);
}

#[test]
fn run_returns_zero_after_senders_dropped_and_timers_drain() {
    let (loop_, sender) = EventLoop::new();
    let fired = Arc::new(Mutex::new(false));
    let recorded = fired.clone();
    sender.add_one_shot(Duration::from_millis(50), move || {
        *recorded.lock().unwrap() = true
    });
    let handle = std::thread::spawn(move || loop_.run());

    std::thread::sleep(Duration::from_millis(30));
    drop(sender);
    let started = std::time::Instant::now();
    assert_eq!(join_within(handle, Duration::from_secs(10)), 0);
    assert!(*fired.lock().unwrap(), "pending timer was dropped early");
    assert!(
        started.elapsed() >= Duration::from_millis(15),
        "loop exited before timer drained"
    );
}
