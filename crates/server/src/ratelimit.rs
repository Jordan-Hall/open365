//! Persistent sliding-window rate limiter (T15).
//!
//! State lives in its own SQLite file (`ratelimit.db`, separate from the main database so that a busy main database
//! cannot turn into refusals, and the reverse), so limits survive a restart and are shared by every process that
//! opens the same file. Calls run on the blocking pool ([`Limiter`]'s async methods), never on a tokio worker. A
//! window slides because only rows younger than `window_secs` count; denied attempts are not stored, so storage per
//! key is bounded by the limit. Keys are stored as HMAC-SHA-256 under a server key derived from the oldest audit key
//! (kept outside the data directory): a copy of the file cannot be used to test guesses of addresses or e-mail
//! addresses, but the file is still pseudonymous data (it shows how often one pseudonym was seen and when).
//!
//! Lockout with backoff ([`Store::hit_backoff`]): the first time a key is over its limit it is locked until the
//! window would have slid; each later lockout inside 24 hours doubles the lock (up to 24 hours). The callers key
//! sign-in limits on address + account, never on the account alone, so an attacker can only lock their own pair
//! (account-lockout DoS). A looser account-wide count applies only to addresses and devices that are not known for
//! that account: an address that signed in before ([`Store::note_success`]) or a browser holding a signed
//! known-device cookie ([`Store::device_token`]).
//!
//! Fail closed: when the store cannot be used, the attempt is denied.

use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::Sha256;

use crate::config::Rate;
use crate::error::{ApiError, ApiResult};
use crate::util::{ct_eq, hex};

/// Result of counting an attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Verdict {
    Allowed,
    /// Over the limit; try again in `retry_after_secs` (at least 1).
    Denied {
        retry_after_secs: u64,
    },
}

impl Verdict {
    pub fn is_allowed(self) -> bool {
        self == Self::Allowed
    }

    /// `Err(RateLimited)` (HTTP 429 with `Retry-After`) when denied.
    pub fn into_result(self) -> ApiResult<()> {
        match self {
            Self::Allowed => Ok(()),
            Self::Denied { retry_after_secs } => Err(ApiError::RateLimited(retry_after_secs)),
        }
    }
}

/// An address in the form used for limiter keys: IPv6 addresses collapse to their /64 (one subscriber holds a
/// whole /64, so per-address keys would be free to rotate); anything else is used as given.
pub fn ip_key(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            format!("{}/64", std::net::Ipv6Addr::from(o))
        }
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Err(_) => ip.to_string(),
    }
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

const MAX_ROWS: i64 = 500_000;
const PURGE_EVERY: u64 = 256;
const STRIKE_DECAY_MS: i64 = 24 * 3600 * 1000;
const MAX_LOCK_MS: i64 = 24 * 3600 * 1000;
const KNOWN_TTL_SECS: i64 = 30 * 24 * 3600;
const KNOWN_PER_ACCOUNT: i64 = 32;
/// Lifetime of a known-device cookie.
pub const DEVICE_TTL_SECS: i64 = 30 * 24 * 3600;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS rl_events (
    key TEXT NOT NULL,
    ts INTEGER NOT NULL,
    exp INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS rl_events_key ON rl_events(key, ts);
CREATE INDEX IF NOT EXISTS rl_events_exp ON rl_events(exp);
CREATE TABLE IF NOT EXISTS rl_state (
    key TEXT PRIMARY KEY,
    strikes INTEGER NOT NULL,
    locked_until INTEGER NOT NULL,
    last INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS rl_known (
    account TEXT NOT NULL,
    ip TEXT NOT NULL,
    seen INTEGER NOT NULL,
    PRIMARY KEY (account, ip)
);";

fn hmac_bytes(key: &[u8], parts: &[&str]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    for p in parts {
        m.update(&u32::try_from(p.len()).unwrap_or(u32::MAX).to_be_bytes());
        m.update(p.as_bytes());
    }
    m.finalize().into_bytes().to_vec()
}

/// The synchronous store. Use [`Limiter`] from request handlers.
pub struct Store {
    conn: Mutex<Connection>,
    clock: Clock,
    hits: Mutex<u64>,
    /// Keys the stored limiter keys.
    key_mac: Vec<u8>,
    /// Keys known-device cookies.
    key_dev: Vec<u8>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

fn system_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn secs_ceil(ms: i64) -> u64 {
    u64::try_from((ms.max(1) + 999) / 1000).unwrap_or(1).max(1)
}

impl Store {
    fn from_conn(conn: Connection, clock: Clock, root_key: &[u8]) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
            clock,
            hits: Mutex::new(0),
            key_mac: hmac_bytes(root_key, &["open365 ratelimit keys v1"]),
            key_dev: hmac_bytes(root_key, &["open365 known-device cookie v1"]),
        })
    }

    /// Opens (creating the tables) the store in the SQLite file at `path`. `root_key` is a server secret.
    pub fn open(path: &Path, root_key: &[u8]) -> rusqlite::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Self::from_conn(conn, Arc::new(system_ms), root_key)
    }

    /// Non-persistent store with a caller-supplied clock in milliseconds (tests).
    pub fn in_memory_with_clock(clock: Clock, root_key: &[u8]) -> Self {
        Self::from_conn(
            Connection::open_in_memory().expect("in-memory sqlite"),
            clock,
            root_key,
        )
        .expect("limiter schema")
    }

    pub fn in_memory() -> Self {
        Self::in_memory_with_clock(Arc::new(system_ms), &[7; 32])
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now_ms(&self) -> i64 {
        (self.clock)()
    }

    /// The stored form of a limiter key, account or address.
    fn h(&self, domain: &str, s: &str) -> String {
        hex(&hmac_bytes(&self.key_mac, &[domain, s]))
    }

    /// Counts one event for `key`; `Denied` when the key already has `rate.max` events inside the window.
    /// Count before the work being limited, so that concurrent requests cannot all pass a check-then-act gap.
    pub fn hit(&self, key: &str, rate: Rate) -> Verdict {
        self.run(key, rate, Mode::Count, false)
    }

    /// Like [`hit`](Self::hit), and a denial starts a lockout that doubles on each repeat within 24 hours.
    pub fn hit_backoff(&self, key: &str, rate: Rate) -> Verdict {
        self.run(key, rate, Mode::Count, true)
    }

    /// Checks `key` without counting an event.
    pub fn peek(&self, key: &str, rate: Rate) -> Verdict {
        self.run(key, rate, Mode::Peek, false)
    }

    fn run(&self, key: &str, rate: Rate, mode: Mode, backoff: bool) -> Verdict {
        let denied = Verdict::Denied {
            retry_after_secs: 1,
        };
        let kh = self.h("key", key);
        let now = self.now_ms();
        let v = {
            let mut c = self.conn();
            match Self::decide(&mut c, &kh, rate, now, mode, backoff) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("rate limiter store error: {e}");
                    return denied;
                }
            }
        };
        if mode == Mode::Count {
            self.maybe_purge(now);
        }
        v
    }

    fn decide(
        c: &mut Connection,
        kh: &str,
        rate: Rate,
        now: i64,
        mode: Mode,
        backoff: bool,
    ) -> rusqlite::Result<Verdict> {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(i64, i64, i64)> = tx
            .query_row(
                "SELECT strikes, locked_until, last FROM rl_state WHERE key = ?1",
                [kh],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((_, locked_until, _)) = state
            && locked_until > now
        {
            return Ok(Verdict::Denied {
                retry_after_secs: secs_ceil(locked_until - now),
            });
        }
        let window = rate.window_secs.saturating_mul(1000);
        tx.execute(
            "DELETE FROM rl_events WHERE key = ?1 AND ts <= ?2",
            params![kh, now.saturating_sub(window)],
        )?;
        let (n, oldest): (i64, Option<i64>) = tx.query_row(
            "SELECT COUNT(*), MIN(ts) FROM rl_events WHERE key = ?1",
            [kh],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if n >= i64::from(rate.max) {
            let natural = oldest
                .map_or(window, |o| o.saturating_add(window) - now)
                .max(1);
            let mut lock = natural;
            if backoff && mode == Mode::Count {
                let strikes = match state {
                    Some((s, _, last)) if now - last <= STRIKE_DECAY_MS => s,
                    _ => 0,
                } + 1;
                let mult = 1_i64 << (strikes - 1).clamp(0, 16);
                lock = natural.saturating_mul(mult).min(MAX_LOCK_MS.max(natural));
                tx.execute(
                    "INSERT INTO rl_state (key, strikes, locked_until, last) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(key) DO UPDATE SET strikes = ?2, locked_until = ?3, last = ?4",
                    params![kh, strikes, now + lock, now],
                )?;
            }
            tx.commit()?;
            return Ok(Verdict::Denied {
                retry_after_secs: secs_ceil(lock),
            });
        }
        if mode == Mode::Count {
            tx.execute(
                "INSERT INTO rl_events (key, ts, exp) VALUES (?1, ?2, ?3)",
                params![kh, now, now.saturating_add(window)],
            )?;
        }
        tx.commit()?;
        Ok(Verdict::Allowed)
    }

    /// Forgets the count and any lockout for `key` (a successful attempt).
    pub fn reset(&self, key: &str) {
        let kh = self.h("key", key);
        let c = self.conn();
        let _ = c.execute("DELETE FROM rl_events WHERE key = ?1", [&kh]);
        let _ = c.execute("DELETE FROM rl_state WHERE key = ?1", [&kh]);
    }

    /// Number of stored events (all keys).
    pub fn len(&self) -> usize {
        self.conn()
            .query_row("SELECT COUNT(*) FROM rl_events", [], |r| r.get::<_, i64>(0))
            .map_or(0, |n| usize::try_from(n).unwrap_or(0))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Records that `ip` signed in to `account`, so that the account-wide failure limit does not apply to it.
    pub fn note_success(&self, account: &str, ip: &str) {
        let (a, i) = (self.h("acct", account), self.h("ip", ip));
        let seen = self.now_ms() / 1000;
        let c = self.conn();
        let _ = c.execute(
            "INSERT INTO rl_known (account, ip, seen) VALUES (?1, ?2, ?3)
             ON CONFLICT(account, ip) DO UPDATE SET seen = ?3",
            params![a, i, seen],
        );
        let _ = c.execute(
            "DELETE FROM rl_known WHERE account = ?1 AND ip NOT IN
             (SELECT ip FROM rl_known WHERE account = ?1 ORDER BY seen DESC LIMIT ?2)",
            params![a, KNOWN_PER_ACCOUNT],
        );
    }

    /// True when `ip` signed in to `account` within the last 30 days.
    pub fn is_known(&self, account: &str, ip: &str) -> bool {
        let (a, i) = (self.h("acct", account), self.h("ip", ip));
        let min = self.now_ms() / 1000 - KNOWN_TTL_SECS;
        self.conn()
            .query_row(
                "SELECT 1 FROM rl_known WHERE account = ?1 AND ip = ?2 AND seen > ?3",
                params![a, i, min],
                |_| Ok(()),
            )
            .optional()
            .is_ok_and(|r| r.is_some())
    }

    /// Name of the known-device cookie for `account`: `__Host-` (so it is Secure, host-only, Path=/) plus a keyed
    /// pseudonym of the account.
    pub fn device_cookie_name(&self, account: &str) -> String {
        format!("__Host-od-{}", &self.h("dev-name", account)[..24])
    }

    /// A signed known-device token for `account` valid until `now_secs + DEVICE_TTL_SECS`: `<expiry>.<hex mac>`.
    pub fn device_token(&self, account: &str, now_secs: i64) -> String {
        let exp = (now_secs + DEVICE_TTL_SECS).to_string();
        let mac = hex(&hmac_bytes(&self.key_dev, &["dev", account, &exp]));
        format!("{exp}.{mac}")
    }

    /// True for an unexpired token made by [`device_token`](Self::device_token) for exactly this `account`.
    pub fn device_valid(&self, account: &str, token: &str, now_secs: i64) -> bool {
        let Some((exp, mac)) = token.split_once('.') else {
            return false;
        };
        let want = hex(&hmac_bytes(&self.key_dev, &["dev", account, exp]));
        let mac_ok = ct_eq(&want, mac);
        mac_ok && exp.parse::<i64>().is_ok_and(|e| e > now_secs)
    }

    fn maybe_purge(&self, now: i64) {
        {
            let mut h = self.hits.lock().unwrap_or_else(|e| e.into_inner());
            *h += 1;
            if !(*h).is_multiple_of(PURGE_EVERY) {
                return;
            }
        }
        let c = self.conn();
        let _ = c.execute("DELETE FROM rl_events WHERE exp <= ?1", [now]);
        let _ = c.execute(
            "DELETE FROM rl_state WHERE locked_until <= ?1 AND last <= ?2",
            params![now, now - STRIKE_DECAY_MS],
        );
        let _ = c.execute(
            "DELETE FROM rl_known WHERE seen <= ?1",
            [now / 1000 - KNOWN_TTL_SECS],
        );
        // Hard bound on storage: under a flood of distinct keys the oldest rows go first.
        if let Ok(total) = c.query_row("SELECT COUNT(*) FROM rl_events", [], |r| r.get::<_, i64>(0))
            && total > MAX_ROWS
        {
            let _ = c.execute(
                "DELETE FROM rl_events WHERE rowid IN
                 (SELECT rowid FROM rl_events ORDER BY ts LIMIT ?1)",
                [total / 10],
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Count,
    Peek,
}

/// Handle used by request handlers: every call runs on the blocking pool, so SQLite waits never occupy a tokio
/// worker. If the blocking task cannot run, the attempt is denied (fail closed).
#[derive(Clone, Debug)]
pub struct Limiter {
    store: Arc<Store>,
}

impl Limiter {
    pub fn open(path: &Path, root_key: &[u8]) -> rusqlite::Result<Self> {
        Ok(Self {
            store: Arc::new(Store::open(path, root_key)?),
        })
    }

    /// The synchronous store (tests, and code already on a blocking thread).
    pub fn store(&self) -> &Store {
        &self.store
    }

    async fn blocking<R: Send + 'static>(
        &self,
        fallback: R,
        f: impl FnOnce(&Store) -> R + Send + 'static,
    ) -> R {
        let s = self.store.clone();
        tokio::task::spawn_blocking(move || f(&s))
            .await
            .unwrap_or(fallback)
    }

    const DENIED: Verdict = Verdict::Denied {
        retry_after_secs: 1,
    };

    pub async fn hit(&self, key: &str, rate: Rate) -> Verdict {
        let k = key.to_string();
        self.blocking(Self::DENIED, move |s| s.hit(&k, rate)).await
    }

    pub async fn hit_backoff(&self, key: &str, rate: Rate) -> Verdict {
        let k = key.to_string();
        self.blocking(Self::DENIED, move |s| s.hit_backoff(&k, rate))
            .await
    }

    pub async fn reset(&self, key: &str) {
        let k = key.to_string();
        self.blocking((), move |s| s.reset(&k)).await;
    }

    pub async fn note_success(&self, account: &str, ip: &str) {
        let (a, i) = (account.to_string(), ip.to_string());
        self.blocking((), move |s| s.note_success(&a, &i)).await;
    }

    pub async fn is_known(&self, account: &str, ip: &str) -> bool {
        let (a, i) = (account.to_string(), ip.to_string());
        self.blocking(false, move |s| s.is_known(&a, &i)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    const R3: Rate = Rate {
        max: 3,
        window_secs: 60,
    };

    fn clocked() -> (Store, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1_000_000));
        let t2 = t.clone();
        (
            Store::in_memory_with_clock(Arc::new(move || t2.load(Ordering::SeqCst)), &[3; 32]),
            t,
        )
    }

    #[test]
    fn blocks_after_max_and_keys_are_independent() {
        let l = Store::in_memory();
        assert!(
            l.hit("a", R3).is_allowed()
                && l.hit("a", R3).is_allowed()
                && l.hit("a", R3).is_allowed()
        );
        assert!(!l.hit("a", R3).is_allowed());
        assert!(!l.peek("a", R3).is_allowed());
        assert!(l.hit("b", R3).is_allowed());
    }

    #[test]
    fn window_slides_and_retry_after_is_accurate() {
        let (l, t) = clocked();
        for _ in 0..3 {
            assert!(l.hit("a", R3).is_allowed());
            t.fetch_add(10_000, Ordering::SeqCst);
        }
        // Events at 0s, 10s, 20s; now 30s. The first leaves the window at 60s, so 30s from now.
        assert_eq!(
            l.hit("a", R3),
            Verdict::Denied {
                retry_after_secs: 30
            }
        );
        t.fetch_add(29_000, Ordering::SeqCst);
        assert!(!l.hit("a", R3).is_allowed(), "not yet");
        t.fetch_add(1_001, Ordering::SeqCst);
        assert!(
            l.hit("a", R3).is_allowed(),
            "oldest event slid out, one slot free"
        );
        assert!(!l.hit("a", R3).is_allowed(), "only one");
    }

    #[test]
    fn denied_attempts_are_not_stored() {
        let l = Store::in_memory();
        for _ in 0..50 {
            let _ = l.hit("a", R3);
        }
        assert_eq!(l.len(), 3);
    }

    #[test]
    fn reset_clears_count_and_lockout() {
        let l = Store::in_memory();
        for _ in 0..4 {
            let _ = l.hit_backoff("a", R3);
        }
        assert!(!l.hit_backoff("a", R3).is_allowed());
        l.reset("a");
        assert!(l.hit_backoff("a", R3).is_allowed());
    }

    #[test]
    fn lockout_doubles_and_decays() {
        let (l, t) = clocked();
        for _ in 0..3 {
            assert!(l.hit_backoff("a", R3).is_allowed());
        }
        assert_eq!(
            l.hit_backoff("a", R3),
            Verdict::Denied {
                retry_after_secs: 60
            }
        );
        // Still locked just before it ends, and the lock does not extend while locked.
        t.fetch_add(59_000, Ordering::SeqCst);
        assert_eq!(
            l.hit_backoff("a", R3),
            Verdict::Denied {
                retry_after_secs: 1
            }
        );
        t.fetch_add(1_001, Ordering::SeqCst);
        for _ in 0..3 {
            assert!(l.hit_backoff("a", R3).is_allowed());
        }
        // Second strike: twice as long.
        assert_eq!(
            l.hit_backoff("a", R3),
            Verdict::Denied {
                retry_after_secs: 120
            }
        );
        // A day of quiet forgets the strikes.
        t.fetch_add(2 * 24 * 3600 * 1000, Ordering::SeqCst);
        for _ in 0..3 {
            assert!(l.hit_backoff("a", R3).is_allowed());
        }
        assert_eq!(
            l.hit_backoff("a", R3),
            Verdict::Denied {
                retry_after_secs: 60
            }
        );
    }

    #[test]
    fn lockout_is_capped() {
        let (l, t) = clocked();
        let r = Rate {
            max: 1,
            window_secs: 3600,
        };
        let mut last = 0;
        for _ in 0..20 {
            let _ = l.hit_backoff("a", r);
            if let Verdict::Denied { retry_after_secs } = l.hit_backoff("a", r) {
                last = retry_after_secs;
                t.fetch_add(
                    i64::try_from(retry_after_secs).unwrap_or(0) * 1000 + 1,
                    Ordering::SeqCst,
                );
            }
        }
        assert!(last <= 24 * 3600, "capped at a day, got {last}");
    }

    #[test]
    fn state_survives_a_restart() {
        let dir = tempfile::tempdir().expect("dir");
        let p = dir.path().join("rl.db");
        {
            let l = Store::open(&p, &[1; 32]).expect("open");
            for _ in 0..3 {
                assert!(l.hit_backoff("k", R3).is_allowed());
            }
            assert!(!l.hit_backoff("k", R3).is_allowed());
            l.note_success("acct", "1.2.3.4");
        }
        let l = Store::open(&p, &[1; 32]).expect("reopen");
        assert!(!l.hit("k", R3).is_allowed(), "count and lockout persisted");
        assert!(l.is_known("acct", "1.2.3.4"));
    }

    #[test]
    fn nothing_identifying_is_stored() {
        let dir = tempfile::tempdir().expect("dir");
        let p = dir.path().join("rl.db");
        let l = Store::open(&p, &[1; 32]).expect("open");
        let _ = l.hit("login:203.0.113.9:acme:alice@example.test", R3);
        l.note_success("acme:alice@example.test", "203.0.113.9");
        let c = l.conn();
        let dump: String = c
            .query_row(
                "SELECT (SELECT group_concat(key) FROM rl_events) || (SELECT group_concat(account || ip) FROM rl_known)",
                [],
                |r| r.get(0),
            )
            .expect("dump");
        assert!(!dump.contains("alice") && !dump.contains("203.0.113"));
    }

    fn stored_keys(s: &Store) -> Vec<String> {
        let c = s.conn();
        let mut st = c.prepare("SELECT key FROM rl_events").expect("prepare");
        st.query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect()
    }

    #[test]
    fn stored_keys_are_keyed_not_a_plain_hash() {
        let a = Store::in_memory_with_clock(Arc::new(|| 1_000), &[1; 32]);
        let b = Store::in_memory_with_clock(Arc::new(|| 1_000), &[2; 32]);
        let key = "login:203.0.113.9:acme:alice@example.test";
        let _ = a.hit(key, R3);
        let _ = b.hit(key, R3);
        let (ka, kb) = (stored_keys(&a), stored_keys(&b));
        assert_ne!(ka, kb, "a different server key gives different stored keys");
        assert_ne!(
            ka[0],
            crate::util::sha256_hex(key.as_bytes()),
            "not an unkeyed SHA-256"
        );
    }

    #[test]
    fn device_tokens_are_signed_bound_to_the_account_and_expire() {
        let s = Store::in_memory();
        let tok = s.device_token("acme:alice", 1_000);
        assert!(s.device_valid("acme:alice", &tok, 1_001));
        assert!(
            !s.device_valid("acme:bob", &tok, 1_001),
            "bound to the account"
        );
        assert!(
            !s.device_valid("acme:alice", &tok, 1_000 + DEVICE_TTL_SECS),
            "expired"
        );
        let (exp, mac) = tok.split_once('.').expect("shape");
        let later = format!("{}.{mac}", exp.parse::<i64>().expect("n") + 99_999);
        assert!(
            !s.device_valid("acme:alice", &later, 1_001),
            "expiry is covered by the MAC"
        );
        assert!(!s.device_valid("acme:alice", "", 1) && !s.device_valid("acme:alice", "x.y", 1));
        let other = Store::in_memory_with_clock(Arc::new(|| 0), &[9; 32]);
        assert!(
            !other.device_valid("acme:alice", &tok, 1_001),
            "another server key"
        );
        assert!(s.device_cookie_name("acme:alice").starts_with("__Host-"));
        assert_ne!(
            s.device_cookie_name("acme:alice"),
            s.device_cookie_name("acme:bob")
        );
    }

    #[tokio::test]
    async fn async_handle_counts_on_the_blocking_pool() {
        let dir = tempfile::tempdir().expect("dir");
        let l = Limiter::open(&dir.path().join("rl.db"), &[1; 32]).expect("open");
        assert!(l.hit("a", R3).await.is_allowed());
        let tasks: Vec<_> = (0..20)
            .map(|_| {
                let l = l.clone();
                tokio::spawn(async move { l.hit("a", R3).await.is_allowed() })
            })
            .collect();
        let mut ok = 0;
        for t in tasks {
            ok += usize::from(t.await.expect("task"));
        }
        assert_eq!(ok, 2, "3 allowed in all, however the requests interleave");
    }

    #[test]
    fn known_addresses_expire_and_are_bounded() {
        let (l, t) = clocked();
        l.note_success("a", "ip0");
        assert!(l.is_known("a", "ip0"));
        assert!(!l.is_known("b", "ip0"), "per account");
        t.fetch_add(31 * 24 * 3600 * 1000, Ordering::SeqCst);
        assert!(!l.is_known("a", "ip0"));
        for i in 0..100 {
            t.fetch_add(1000, Ordering::SeqCst);
            l.note_success("a", &format!("ip{i}"));
        }
        let n: i64 = l
            .conn()
            .query_row("SELECT COUNT(*) FROM rl_known", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, KNOWN_PER_ACCOUNT);
        assert!(l.is_known("a", "ip99") && !l.is_known("a", "ip1"));
    }

    #[test]
    fn ipv6_collapses_to_its_64() {
        assert_eq!(
            ip_key("2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
            "2001:db8:1:2::/64"
        );
        assert_eq!(ip_key("2001:db8:1:2::1"), ip_key("2001:db8:1:2:ffff::9"));
        assert_ne!(ip_key("2001:db8:1:3::1"), ip_key("2001:db8:1:2::1"));
        assert_eq!(ip_key("203.0.113.5"), "203.0.113.5");
        assert_eq!(ip_key("unknown"), "unknown");
    }

    #[test]
    fn storage_is_bounded_by_purge() {
        let (l, t) = clocked();
        for i in 0..PURGE_EVERY {
            let _ = l.hit(&format!("k{i}"), R3);
        }
        t.fetch_add(120_000, Ordering::SeqCst);
        for i in 0..PURGE_EVERY {
            let _ = l.hit(&format!("n{i}"), R3);
        }
        assert!(
            l.len() <= usize::try_from(PURGE_EVERY).unwrap_or(0) + 1,
            "expired rows purged"
        );
    }
}
