//! The channel the events of a conversation travel through.
//!
//! A backend works on its own thread and pushes [`AssistantEvent`]s into an
//! [`EventSink`]; the interface reads them from the [`EventStream`] on the main thread.
//! The stream can be read in three ways, and none of them blocks the main thread:
//!
//! * **awaited**: [`EventStream::next`] and [`EventStream::next_batch`] are futures. They
//!   need no particular executor: the sink wakes the task that waits. This is how a GPUI
//!   view reads the stream (see the example on [`EventStream::next_batch`]).
//! * **polled**: [`EventStream::try_next`] and [`EventStream::drain`] return what is there.
//! * **blocking**: [`EventStream::wait`] sleeps the calling thread for at most a given
//!   time. For tests and for threads that are not the main thread.
//!
//! The stream ends (`None`) when every sink is gone and everything sent was read.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::backend::AssistantEvent;

struct State {
    events: VecDeque<AssistantEvent>,
    waker: Option<Waker>,
    sinks: usize,
}

struct Shared {
    state: Mutex<State>,
    arrived: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves the queue as it was: still usable.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A new channel: the end a backend writes to and the end the interface reads.
pub fn event_channel() -> (EventSink, EventStream) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            events: VecDeque::new(),
            waker: None,
            sinks: 1,
        }),
        arrived: Condvar::new(),
    });
    (
        EventSink {
            shared: shared.clone(),
        },
        EventStream { shared },
    )
}

/// The writing end. Cloned freely; the stream ends when the last clone is dropped.
pub struct EventSink {
    shared: Arc<Shared>,
}

impl EventSink {
    /// Queues `event` and wakes whoever waits for it. Never blocks.
    pub fn send(&self, event: AssistantEvent) {
        let waker = {
            let mut state = self.shared.lock();
            state.events.push_back(event);
            state.waker.take()
        };
        self.shared.arrived.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl Clone for EventSink {
    fn clone(&self) -> Self {
        self.shared.lock().sinks += 1;
        EventSink {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for EventSink {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.shared.lock();
            state.sinks = state.sinks.saturating_sub(1);
            if state.sinks == 0 {
                state.waker.take()
            } else {
                None
            }
        };
        self.shared.arrived.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

/// The reading end. One reader at a time is the intended use; a clone reads from the
/// same queue (an event goes to whichever reader takes it first).
#[derive(Clone)]
pub struct EventStream {
    shared: Arc<Shared>,
}

impl EventStream {
    /// The next event if one is waiting. Never blocks.
    pub fn try_next(&self) -> Option<AssistantEvent> {
        self.shared.lock().events.pop_front()
    }

    /// Everything that is waiting, oldest first. Never blocks.
    pub fn drain(&self) -> Vec<AssistantEvent> {
        self.shared.lock().events.drain(..).collect()
    }

    /// Whether the stream has ended: every sink is gone and nothing is left to read.
    pub fn is_finished(&self) -> bool {
        let state = self.shared.lock();
        state.sinks == 0 && state.events.is_empty()
    }

    /// The next event, awaited. `None` when the stream has ended.
    pub fn next(&self) -> Next<'_> {
        Next { stream: self }
    }

    /// Everything that is waiting, awaited: at least one event, or `None` when the stream
    /// has ended. An answer arrives in many small pieces; reading them a batch at a time
    /// costs one redraw per batch instead of one per piece.
    ///
    /// In a GPUI view (gpui 0.2.2), with a [`ChatSession`](crate::ChatSession):
    ///
    /// ```ignore
    /// let events = self.session.events();
    /// cx.spawn(async move |this, cx| {
    ///     while let Some(batch) = events.next_batch().await {
    ///         let alive = this.update(cx, |this, cx| {
    ///             for event in batch {
    ///                 this.session.apply(event);
    ///             }
    ///             cx.notify();
    ///         });
    ///         if alive.is_err() {
    ///             break; // the view is gone
    ///         }
    ///     }
    /// })
    /// .detach();
    /// ```
    pub fn next_batch(&self) -> NextBatch<'_> {
        NextBatch { stream: self }
    }

    /// Blocks the calling thread until an event arrives, the stream ends or `timeout`
    /// has passed. Not for the main thread of the app.
    pub fn wait(&self, timeout: Duration) -> Option<AssistantEvent> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        loop {
            if let Some(event) = state.events.pop_front() {
                return Some(event);
            }
            if state.sinks == 0 {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            if left.is_zero() {
                return None;
            }
            state = self
                .shared
                .arrived
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Blocks like [`wait`](Self::wait) and collects events until `done` says the last
    /// one was the one waited for (it is included), the stream ends or `timeout` has
    /// passed. Returns what arrived and whether `done` was satisfied.
    pub fn wait_until(
        &self,
        timeout: Duration,
        mut done: impl FnMut(&AssistantEvent) -> bool,
    ) -> (Vec<AssistantEvent>, bool) {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.wait(left) {
                Some(event) => {
                    let finished = done(&event);
                    seen.push(event);
                    if finished {
                        return (seen, true);
                    }
                }
                None => return (seen, false),
            }
        }
    }
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventStream")
    }
}

/// The future of [`EventStream::next`].
pub struct Next<'a> {
    stream: &'a EventStream,
}

impl Future for Next<'_> {
    type Output = Option<AssistantEvent>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.stream.shared.lock();
        if let Some(event) = state.events.pop_front() {
            return Poll::Ready(Some(event));
        }
        if state.sinks == 0 {
            return Poll::Ready(None);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// The future of [`EventStream::next_batch`].
pub struct NextBatch<'a> {
    stream: &'a EventStream,
}

impl Future for NextBatch<'_> {
    type Output = Option<Vec<AssistantEvent>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.stream.shared.lock();
        if !state.events.is_empty() {
            return Poll::Ready(Some(state.events.drain(..).collect()));
        }
        if state.sinks == 0 {
            return Poll::Ready(None);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
