//! In-memory fixed-window rate limiter (T15). Per-process; a multi-node deployment would move this to a shared store.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::config::Rate;
use crate::util::now;

#[derive(Debug, Default)]
pub struct Limiter {
    map: Mutex<HashMap<String, (i64, u32)>>,
}

const MAX_KEYS: usize = 100_000;

impl Limiter {
    fn with<R>(&self, f: impl FnOnce(&mut HashMap<String, (i64, u32)>) -> R) -> R {
        f(&mut self.map.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Counts one event; returns false when the limit is already exceeded.
    pub fn hit(&self, key: &str, rate: Rate) -> bool {
        let t = now();
        self.with(|m| {
            if m.len() > MAX_KEYS {
                m.retain(|_, (start, _)| t - *start < 3600);
            }
            let e = m.entry(key.to_string()).or_insert((t, 0));
            if t - e.0 >= rate.window_secs {
                *e = (t, 0);
            }
            e.1 = e.1.saturating_add(1);
            e.1 <= rate.max
        })
    }

    /// True when `key` has already used up its allowance (no counting).
    pub fn blocked(&self, key: &str, rate: Rate) -> bool {
        let t = now();
        self.with(|m| {
            m.get(key)
                .is_some_and(|(start, n)| t - *start < rate.window_secs && *n >= rate.max)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max() {
        let l = Limiter::default();
        let r = Rate {
            max: 3,
            window_secs: 60,
        };
        assert!(l.hit("a", r) && l.hit("a", r) && l.hit("a", r));
        assert!(!l.hit("a", r));
        assert!(l.blocked("a", r));
        assert!(l.hit("b", r), "keys are independent");
    }
}
