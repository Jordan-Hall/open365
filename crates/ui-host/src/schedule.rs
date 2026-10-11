//! Frame scheduling. The host never spins: it redraws only when something asked for a frame
//! and otherwise sleeps until the next timed wake-up (a caret blink, an animation step).

use std::time::{Duration, Instant};

/// Collects redraw requests and the earliest wake-up time.
#[derive(Debug, Default)]
pub struct Scheduler {
    redraw: bool,
    wake: Option<Instant>,
    exit: bool,
}

impl Scheduler {
    /// Ask for one frame as soon as the event loop is idle.
    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    /// Wake at `at` (the earliest pending wake-up wins).
    pub fn wake_at(&mut self, at: Instant) {
        self.wake = Some(self.wake.map_or(at, |w| w.min(at)));
    }

    pub fn wake_after(&mut self, now: Instant, delay: Duration) {
        self.wake_at(now + delay);
    }

    /// Ask the host to close the window and leave the event loop.
    pub fn request_exit(&mut self) {
        self.exit = true;
    }

    pub fn take_exit(&mut self) -> bool {
        std::mem::take(&mut self.exit)
    }

    /// Returns and clears the redraw request.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    pub fn redraw_pending(&self) -> bool {
        self.redraw
    }

    /// True once, when the pending wake-up time has been reached.
    pub fn take_due_wake(&mut self, now: Instant) -> bool {
        match self.wake {
            Some(w) if w <= now => {
                self.wake = None;
                true
            }
            _ => false,
        }
    }

    pub fn next_wake(&self) -> Option<Instant> {
        self.wake
    }
}

/// Caret blink phase. Windows reports the caret blink time as the interval between
/// toggles, so a full cycle is twice that.
#[derive(Debug, Clone, Copy)]
pub struct CaretBlink {
    interval: Option<Duration>,
    epoch: Instant,
}

impl CaretBlink {
    /// `interval` is the time between on/off toggles; `None` means the caret does not blink.
    pub fn new(interval: Option<Duration>, now: Instant) -> Self {
        Self {
            interval: interval.filter(|d| !d.is_zero()),
            epoch: now,
        }
    }

    /// Show the caret immediately and restart the cycle (call on every keystroke or click).
    pub fn restart(&mut self, now: Instant) {
        self.epoch = now;
    }

    pub fn visible(&self, now: Instant) -> bool {
        let Some(i) = self.interval else { return true };
        let elapsed = now.saturating_duration_since(self.epoch);
        (elapsed.as_nanos() / i.as_nanos()).is_multiple_of(2)
    }

    /// When the caret next changes state, or `None` when it never does.
    pub fn next_toggle(&self, now: Instant) -> Option<Instant> {
        let i = self.interval?;
        let elapsed = now.saturating_duration_since(self.epoch);
        let n = elapsed.as_nanos() / i.as_nanos() + 1;
        Some(self.epoch + i * u32::try_from(n).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn idle_scheduler_asks_for_nothing() {
        let mut s = Scheduler::default();
        assert!(!s.take_redraw());
        assert_eq!(s.next_wake(), None);
        assert!(!s.take_due_wake(Instant::now()));
    }

    #[test]
    fn redraw_request_is_consumed_once() {
        let mut s = Scheduler::default();
        s.request_redraw();
        s.request_redraw();
        assert!(s.redraw_pending());
        assert!(s.take_redraw());
        assert!(!s.take_redraw());
    }

    #[test]
    fn earliest_wake_wins_and_fires_once() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        s.wake_after(t0, MS(500));
        s.wake_after(t0, MS(200));
        s.wake_after(t0, MS(900));
        assert_eq!(s.next_wake(), Some(t0 + MS(200)));
        assert!(!s.take_due_wake(t0 + MS(199)));
        assert!(s.take_due_wake(t0 + MS(200)));
        assert!(!s.take_due_wake(t0 + MS(300)));
        assert_eq!(s.next_wake(), None);
    }

    #[test]
    fn caret_blinks_at_the_os_interval() {
        let t0 = Instant::now();
        let c = CaretBlink::new(Some(MS(530)), t0);
        assert!(c.visible(t0));
        assert!(c.visible(t0 + MS(529)));
        assert!(!c.visible(t0 + MS(530)));
        assert!(!c.visible(t0 + MS(1059)));
        assert!(c.visible(t0 + MS(1060)));
        assert_eq!(c.next_toggle(t0), Some(t0 + MS(530)));
        assert_eq!(c.next_toggle(t0 + MS(530)), Some(t0 + MS(1060)));
    }

    #[test]
    fn restart_shows_the_caret_again() {
        let t0 = Instant::now();
        let mut c = CaretBlink::new(Some(MS(500)), t0);
        assert!(!c.visible(t0 + MS(600)));
        c.restart(t0 + MS(600));
        assert!(c.visible(t0 + MS(600)));
        assert_eq!(c.next_toggle(t0 + MS(600)), Some(t0 + MS(1100)));
    }

    #[test]
    fn a_non_blinking_caret_is_always_visible_and_never_wakes() {
        let t0 = Instant::now();
        for interval in [None, Some(Duration::ZERO)] {
            let c = CaretBlink::new(interval, t0);
            assert!(c.visible(t0 + MS(10_000)));
            assert_eq!(c.next_toggle(t0), None);
        }
    }
}
