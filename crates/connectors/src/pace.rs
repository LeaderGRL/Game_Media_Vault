//! Spacing the requests a Source limits per minute.

use std::{
    sync::{Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// The time a pace keeps to, and the waits it takes.
pub trait PaceClock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

/// The system's clock, waiting for real.
pub struct SystemClock;

impl PaceClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// Requests that leave at least `interval` apart, a figure the Source may restate.
pub(crate) struct RequestPace {
    clock: Box<dyn PaceClock>,
    state: Mutex<PaceState>,
}

struct PaceState {
    interval: Duration,
    /// When the previous request left.
    last: Option<Instant>,
}

impl RequestPace {
    pub(crate) fn new(interval: Duration, clock: Box<dyn PaceClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(PaceState {
                interval,
                last: None,
            }),
        }
    }

    /// Waits until a request may leave, and counts it as leaving then.
    pub(crate) fn wait_for_turn(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(last) = state.last {
            let turn = last + state.interval;
            let now = self.clock.now();
            if turn > now {
                self.clock.sleep(turn - now);
            }
        }
        state.last = Some(self.clock.now());
    }

    /// Keeps the next requests `requests_per_minute` a minute, as the Source restated it.
    pub(crate) fn allow_per_minute(&self, requests_per_minute: u64) {
        if requests_per_minute == 0 {
            return;
        }
        let interval =
            Duration::from_secs(60) / u32::try_from(requests_per_minute).unwrap_or(u32::MAX);
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .interval = interval;
    }
}

/// A clock shared with its caller, so a test can read the waits it took.
impl<C: PaceClock + ?Sized> PaceClock for std::sync::Arc<C> {
    fn now(&self) -> Instant {
        (**self).now()
    }

    fn sleep(&self, duration: Duration) {
        (**self).sleep(duration);
    }
}
