//! RT app: the event loop and application shell.
//!
//! Runs the single-threaded core loop: deliver posted events and timers,
//! then run one Flush step. Other threads participate only by posting
//! events through [`EventSender`] (explicit marshalling, ADR 0003).

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// A unit of work delivered to the event loop, in FIFO order.
pub enum Event {
    /// Stops the loop; [`EventLoop::run`] returns `code`.
    Quit { code: i32 },
    /// Runs `f` once on the loop thread.
    Call(Box<dyn FnOnce() + Send>),
}

/// Thread-safe, cloneable handle for posting into an [`EventLoop`] from any thread.
#[derive(Clone)]
pub struct EventSender {
    tx: Sender<Message>,
}

impl EventSender {
    /// Post `event`; delivered on the loop thread in FIFO order.
    pub fn post_event(&self, event: Event) {
        self.send(Message::Event(event));
    }

    /// Post [`Event::Quit`] with `code`.
    pub fn quit(&self, code: i32) {
        self.post_event(Event::Quit { code });
    }

    /// Post `f` to run on the loop thread.
    pub fn call<F: FnOnce() + Send + 'static>(&self, f: F) {
        self.post_event(Event::Call(Box::new(f)));
    }

    /// Arm a timer that fires once on the loop thread after `delay`.
    pub fn add_one_shot<F: FnOnce() + Send + 'static>(&self, delay: Duration, f: F) -> TimerHandle {
        let canceled = Arc::new(AtomicBool::new(false));
        let timer = Timer {
            kind: TimerKind::Once {
                delay,
                f: Box::new(f),
            },
            canceled: canceled.clone(),
        };
        self.send(Message::AddTimer { timer });
        TimerHandle { canceled }
    }

    /// Arm a timer that fires on the loop thread every `interval` until
    /// cancelled (via the returned handle or `timer.cancel()` inside the callback).
    pub fn add_repeating<F>(&self, interval: Duration, mut f: F) -> TimerHandle
    where
        F: FnMut(&TimerHandle) + Send + 'static,
    {
        let canceled = Arc::new(AtomicBool::new(false));
        let handle = TimerHandle {
            canceled: canceled.clone(),
        };
        let timer = Timer {
            kind: TimerKind::Repeat {
                interval,
                f: Box::new(move |timer| f(timer)),
            },
            canceled,
        };
        self.send(Message::AddTimer { timer });
        handle
    }

    fn send(&self, msg: Message) {
        self.tx.send(msg).expect("event loop receiver gone");
    }
}

/// Cancels an armed timer from any thread; safe to call more than once.
#[derive(Clone)]
pub struct TimerHandle {
    canceled: Arc<AtomicBool>,
}

impl TimerHandle {
    pub fn cancel(&self) {
        self.canceled.store(true, AtomicOrdering::Release);
    }
}

enum Message {
    Event(Event),
    AddTimer { timer: Timer },
}

struct Timer {
    kind: TimerKind,
    canceled: Arc<AtomicBool>,
}

enum TimerKind {
    Once {
        delay: Duration,
        f: Box<dyn FnOnce() + Send>,
    },
    Repeat {
        interval: Duration,
        f: Box<dyn FnMut(&TimerHandle) + Send>,
    },
}

struct TimerEntry {
    deadline: Instant,
    seq: u64,
    kind: TimerKind,
    canceled: Arc<AtomicBool>,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.seq == other.seq
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.deadline, self.seq).cmp(&(other.deadline, other.seq))
    }
}

/// The single-threaded event loop.
pub struct EventLoop {
    rx: Receiver<Message>,
    timers: BinaryHeap<Reverse<TimerEntry>>,
    next_seq: u64,
}

impl EventLoop {
    /// Create the loop and its posting handle.
    pub fn new() -> (Self, EventSender) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            Self {
                rx,
                timers: BinaryHeap::new(),
                next_seq: 0,
            },
            EventSender { tx },
        )
    }

    /// Deliver events until [`Event::Quit`] arrives; returns its exit code.
    /// Also returns 0 once all senders are dropped and no timers remain armed
    /// (armed repeating timers keep the loop running).
    pub fn run(mut self) -> i32 {
        loop {
            let mut quit = None;
            while let Ok(msg) = self.rx.try_recv() {
                if let Some(code) = self.dispatch(msg) {
                    quit = Some(code);
                    break;
                }
            }
            if let Some(code) = quit {
                return code;
            }
            self.fire_due_timers();
            self.flush();
            if let Some(code) = self.block_until_wake() {
                return code;
            }
        }
    }

    /// Flush seam: rt-core signal delivery and dirty-property evaluation (tickets #5/#6) run at this point in the cycle.
    fn flush(&mut self) {}

    fn dispatch(&mut self, msg: Message) -> Option<i32> {
        match msg {
            Message::Event(Event::Quit { code }) => Some(code),
            Message::Event(Event::Call(f)) => {
                f();
                None
            }
            Message::AddTimer { timer } => {
                self.register(timer);
                None
            }
        }
    }

    fn register(&mut self, timer: Timer) {
        let delay = match &timer.kind {
            TimerKind::Once { delay, .. } => *delay,
            TimerKind::Repeat { interval, .. } => *interval,
        };
        // Clamp an overflowing deadline to "as soon as possible".
        let deadline = Instant::now()
            .checked_add(delay)
            .unwrap_or_else(Instant::now);
        self.next_seq += 1;
        self.timers.push(Reverse(TimerEntry {
            deadline,
            seq: self.next_seq,
            kind: timer.kind,
            canceled: timer.canceled,
        }));
    }

    fn fire_due_timers(&mut self) {
        let now = Instant::now();
        while self.next_deadline().is_some_and(|d| d <= now) {
            let Reverse(entry) = self.timers.pop().expect("deadline present");
            match entry.kind {
                TimerKind::Once { f, .. } => f(),
                TimerKind::Repeat { interval, mut f } => {
                    f(&TimerHandle {
                        canceled: entry.canceled.clone(),
                    });
                    // Rearm drift-free from the previous deadline, coalescing
                    // missed intervals; drop on overflow.
                    let mut deadline = entry.deadline;
                    loop {
                        match deadline.checked_add(interval) {
                            Some(next) if next > now => {
                                self.next_seq += 1;
                                self.timers.push(Reverse(TimerEntry {
                                    deadline: next,
                                    seq: self.next_seq,
                                    kind: TimerKind::Repeat { interval, f },
                                    canceled: entry.canceled,
                                }));
                                break;
                            }
                            Some(next) => deadline = next,
                            None => break,
                        }
                    }
                }
            }
        }
    }

    /// Earliest live deadline, discarding canceled timers along the way.
    fn next_deadline(&mut self) -> Option<Instant> {
        while let Some(Reverse(entry)) = self.timers.peek() {
            if entry.canceled.load(AtomicOrdering::Acquire) {
                self.timers.pop();
            } else {
                return Some(entry.deadline);
            }
        }
        None
    }

    /// Block until a message arrives or the next timer deadline passes.
    /// Returns an exit code only when the loop should stop.
    fn block_until_wake(&mut self) -> Option<i32> {
        let timeout = self
            .next_deadline()
            .map(|d| d.saturating_duration_since(Instant::now()));
        let received = match timeout {
            None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(t) => self.rx.recv_timeout(t),
        };
        match received {
            Ok(msg) => self.dispatch(msg),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                if self.next_deadline().is_none() {
                    Some(0)
                } else {
                    // No sender can ever wake us; coast to the deadline.
                    std::thread::sleep(
                        self.next_deadline()
                            .expect("deadline checked above")
                            .saturating_duration_since(Instant::now()),
                    );
                    None
                }
            }
        }
    }
}
