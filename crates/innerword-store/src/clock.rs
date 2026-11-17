//! The time source used for every timestamp the store writes.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the store reads "now" from, in unix milliseconds.
///
/// [`Clock::system`] (the default) reads the system clock. [`Clock::manual`] returns a clock
/// that only moves when told to; clones share the same time, so a test can keep a clone and
/// advance the store's clock from outside:
///
/// ```
/// use innerword_store::{Clock, Store};
/// let clock = Clock::manual(1_000);
/// let mut store = Store::open_in_memory_with_clock(clock.clone()).unwrap();
/// clock.advance_ms(60_000);
/// assert_eq!(store.clock().now_ms(), 61_000);
/// ```
#[derive(Clone, Debug, Default)]
pub struct Clock {
    manual: Option<Arc<AtomicI64>>,
}

impl Clock {
    /// A clock that reads the system time.
    pub fn system() -> Self {
        Self { manual: None }
    }

    /// A controllable clock starting at `start_ms` (unix milliseconds).
    pub fn manual(start_ms: i64) -> Self {
        Self {
            manual: Some(Arc::new(AtomicI64::new(start_ms))),
        }
    }

    /// Whether this is a manual clock.
    pub fn is_manual(&self) -> bool {
        self.manual.is_some()
    }

    /// The current time in unix milliseconds.
    pub fn now_ms(&self) -> i64 {
        match &self.manual {
            Some(t) => t.load(Ordering::SeqCst),
            None => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0),
        }
    }

    /// Sets a manual clock to `ms`. Does nothing on a system clock.
    pub fn set_ms(&self, ms: i64) {
        if let Some(t) = &self.manual {
            t.store(ms, Ordering::SeqCst);
        }
    }

    /// Moves a manual clock forward by `delta_ms` (negative moves it back). Does nothing on a
    /// system clock.
    pub fn advance_ms(&self, delta_ms: i64) {
        if let Some(t) = &self.manual {
            t.fetch_add(delta_ms, Ordering::SeqCst);
        }
    }
}
