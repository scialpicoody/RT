//! The application layer: [`App`] owns a typed message inbox and a timer
//! wheel, driven on the calling thread through the [`Application`] trait.
//! Only `Send` message data ever crosses a thread boundary — application
//! state (e.g. a `Hub`) lives on the loop thread and never moves.
//!
//! The cycle: drain messages (cross-thread sends, then self-posts deferred
//! from earlier cycles) → fire due timers → `on_cycle` (the Flush hook:
//! one `Hub::flush()` per cycle belongs there) → block until the next
//! message or timer deadline.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// Identity of a timer scheduled through [`Ctx`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId(u64);

/// Cloneable, thread-safe handle for sending messages into an [`App`].
#[derive(Clone)]
pub struct AppHandle<Msg> {
    tx: Sender<Msg>,
}

impl<Msg> AppHandle<Msg> {
    /// Send one message; delivered to `on_message` on the loop thread.
    /// Panics when the app is no longer running.
    pub fn send(&self, msg: Msg) {
        self.tx.send(msg).expect("rt-app: application not running");
    }

    /// Like [`AppHandle::send`], but hands the message back instead of
    /// panicking when the app is no longer running.
    pub fn try_send(&self, msg: Msg) -> Result<(), Msg> {
        self.tx.send(msg).map_err(|e| e.0)
    }
}

/// Loop-thread access while dispatching: quit, self-posting, and timer
/// scheduling.
pub struct Ctx<'a, Msg> {
    state: &'a mut LoopState<Msg>,
}

impl<Msg> Ctx<'_, Msg> {
    /// Stop the loop; [`App::run`] returns `code`.
    pub fn quit(&mut self, code: i32) {
        self.state.quit = Some(code);
    }

    /// Enqueue a message to this app's own inbox. Delivered to
    /// `on_message` on a later cycle than the one that posts it, FIFO
    /// among self-posts and after that cycle's cross-thread messages.
    /// Never fails.
    pub fn post(&mut self, msg: Msg) {
        self.state.posted.push_back(msg);
    }

    /// Arm a one-shot timer; `on_timer(id)` fires once after `delay`.
    pub fn after(&mut self, delay: Duration) -> TimerId {
        self.arm(delay, None)
    }

    /// Arm a repeating timer; `on_timer(id)` fires every `interval`.
    pub fn every(&mut self, interval: Duration) -> TimerId {
        self.arm(interval, Some(interval))
    }

    pub fn cancel_timer(&mut self, id: TimerId) {
        self.state.canceled.insert(id);
    }

    fn arm(&mut self, delay: Duration, interval: Option<Duration>) -> TimerId {
        self.state.next_timer_id += 1;
        let id = TimerId(self.state.next_timer_id);
        self.state.seq += 1;
        let deadline = Instant::now().checked_add(delay).unwrap_or_else(Instant::now);
        self.state.timers.push(Reverse(TimerEntry {
            deadline,
            seq: self.state.seq,
            id,
            interval,
        }));
        id
    }
}

struct TimerEntry {
    deadline: Instant,
    seq: u64,
    id: TimerId,
    interval: Option<Duration>,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.seq == other.seq
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.deadline, self.seq).cmp(&(other.deadline, other.seq))
    }
}

struct LoopState<Msg> {
    quit: Option<i32>,
    timers: BinaryHeap<Reverse<TimerEntry>>,
    canceled: HashSet<TimerId>,
    next_timer_id: u64,
    seq: u64,
    /// Self-posts: messages queued by [`Ctx::post`], dispatched on a later
    /// cycle than the one that queued them.
    posted: VecDeque<Msg>,
}

/// An application: message handling, timer reactions, and the per-cycle hook.
pub trait Application {
    type Msg: Send + 'static;

    /// One message arrived (from this thread or any other). Post back via
    /// [`Ctx::post`] to defer work to a later cycle.
    fn on_message(&mut self, ctx: &mut Ctx<Self::Msg>, msg: Self::Msg);

    /// A timer scheduled through [`Ctx`] fired. Timers are identified by id —
    /// the application maps ids to behavior itself.
    fn on_timer(&mut self, ctx: &mut Ctx<Self::Msg>, timer: TimerId) {
        let _ = (ctx, timer);
    }

    /// End of one pump cycle. The Flush hook: drive per-cycle work (e.g. one
    /// `Hub::flush()`) here.
    fn on_cycle(&mut self, ctx: &mut Ctx<Self::Msg>) {
        let _ = ctx;
    }

    /// Called exactly once when the loop exits, with the final quit code,
    /// before the application state is returned. The loop no longer pumps:
    /// timers are inert and posting is unavailable.
    fn on_shutdown(&mut self, code: i32) {
        let _ = code;
    }
}

/// A running application's loop-thread side.
pub struct App<Msg> {
    rx: Receiver<Msg>,
}

impl<Msg> App<Msg>
where
    Msg: Send + 'static,
{
    /// Create the app and its sending handle.
    pub fn new() -> (Self, AppHandle<Msg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { rx }, AppHandle { tx })
    }

    /// Run to completion on the calling thread; returns the application
    /// state and the quit code. The code is 0 when every handle is dropped
    /// and no timer remains armed. [`Application::on_shutdown`] runs once
    /// on every exit path before the state is returned.
    pub fn run<A: Application<Msg = Msg>>(self, app: A) -> (A, i32) {
        let mut state = LoopState {
            quit: None,
            timers: BinaryHeap::new(),
            canceled: HashSet::new(),
            next_timer_id: 0,
            seq: 0,
            posted: VecDeque::new(),
        };
        let (mut app, code) = self.pump(&mut state, app);
        app.on_shutdown(code);
        (app, code)
    }

    /// The pump cycle: drain messages -> fire due timers -> cycle hook ->
    /// block until a message or timer deadline.
    fn pump<A: Application<Msg = Msg>>(self, state: &mut LoopState<Msg>, mut app: A) -> (A, i32) {
        loop {
            // 1. Drain queued messages: self-posts deferred from earlier
            // cycles first (they predate this cycle's sends), then
            // cross-thread sends. Posts made during dispatch queue up for
            // the next cycle.
            for msg in std::mem::take(&mut state.posted) {
                dispatch_message(&mut app, state, msg);
                if let Some(code) = state.quit.take() {
                    return (app, code);
                }
            }
            while let Ok(msg) = self.rx.try_recv() {
                dispatch_message(&mut app, state, msg);
                if let Some(code) = state.quit.take() {
                    return (app, code);
                }
            }

            // 2. Fire due timers (rearm repeating ones drift-free).
            let now = Instant::now();
            let mut due = Vec::new();
            while next_deadline(state).is_some_and(|d| d <= now) {
                let Reverse(entry) = state.timers.pop().expect("deadline present");
                if state.canceled.remove(&entry.id) {
                    continue;
                }
                if let Some(interval) = entry.interval {
                    let mut deadline = entry.deadline;
                    while let Some(next) = deadline.checked_add(interval)
                        && next <= now
                    {
                        deadline = next;
                    }
                    if let Some(next) = deadline.checked_add(interval) {
                        state.seq += 1;
                        state.timers.push(Reverse(TimerEntry {
                            deadline: next,
                            seq: state.seq,
                            id: entry.id,
                            interval: Some(interval),
                        }));
                    }
                }
                due.push(entry.id);
            }
            for id in due {
                dispatch_timer(&mut app, state, id);
                if let Some(code) = state.quit.take() {
                    return (app, code);
                }
            }

            // 3. Cycle hook (the Flush seam).
            {
                let mut ctx = Ctx { state: &mut *state };
                app.on_cycle(&mut ctx);
            }
            if let Some(code) = state.quit.take() {
                return (app, code);
            }

            // 4. Block until a message, a deadline, or channel close.
            let timeout =
                next_deadline(state).map(|d| d.saturating_duration_since(Instant::now()));
            let received = match timeout {
                None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(t) => self.rx.recv_timeout(t),
            };
            match received {
                Ok(msg) => {
                    dispatch_message(&mut app, state, msg);
                    if let Some(code) = state.quit.take() {
                        return (app, code);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // All senders are gone. Stay only if timers remain armed
                    // or self-posts from this cycle still await dispatch.
                    if next_deadline(state).is_none() && state.posted.is_empty() {
                        return (app, 0);
                    }
                    if let Some(deadline) = next_deadline(state) {
                        // No sender can ever wake us; coast to the deadline.
                        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    }
                }
            }
        }
    }
}

fn dispatch_message<A: Application>(app: &mut A, state: &mut LoopState<A::Msg>, msg: A::Msg) {
    let mut ctx = Ctx { state };
    app.on_message(&mut ctx, msg);
}

fn dispatch_timer<A: Application>(app: &mut A, state: &mut LoopState<A::Msg>, id: TimerId) {
    let mut ctx = Ctx { state };
    app.on_timer(&mut ctx, id);
}

/// Earliest live deadline, discarding canceled timers along the way.
fn next_deadline<Msg>(state: &mut LoopState<Msg>) -> Option<Instant> {
    while let Some(Reverse(entry)) = state.timers.peek() {
        if state.canceled.contains(&entry.id) {
            state.timers.pop();
        } else {
            return Some(entry.deadline);
        }
    }
    // Heap exhausted: every id in `canceled` is now unreferenced garbage.
    state.canceled.clear();
    None
}
