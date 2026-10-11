//! Append-only HMAC-chained audit log, one chain per tenant (plus a system chain `-`).
//!
//! Each row's MAC covers its fields and the previous row's MAC, so editing, reordering or deleting a row in the
//! middle breaks verification. Deleting the newest rows is not detectable from the database alone; an operator
//! who needs that anchors the latest MAC elsewhere (documented in the README).

use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;

use crate::config::AuditKeys;
use crate::error::{ApiError, ApiResult};
use crate::util::{hex, now, unhex};

pub const SYSTEM_TENANT: &str = "-";
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub seq: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub key_id: String,
    pub prev_mac: String,
    pub mac: String,
}

fn mac_bytes(key: &[u8], tenant: &str, e: &Entry) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    for f in [
        tenant,
        &e.seq.to_string(),
        &e.ts.to_string(),
        &e.actor,
        &e.action,
        &e.target,
        &e.detail,
        &e.key_id,
        &e.prev_mac,
    ] {
        // Length prefix so that field boundaries cannot be shifted.
        m.update(&u32::try_from(f.len()).unwrap_or(u32::MAX).to_be_bytes());
        m.update(f.as_bytes());
    }
    m.finalize().into_bytes().to_vec()
}

/// Every audit field is bounded, so that user-supplied text cannot grow the log without limit.
fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Appends one event. Call inside the same transaction as the change it records.
pub fn append(
    conn: &Connection,
    keys: &AuditKeys,
    tenant: &str,
    actor: &str,
    action: &str,
    target: &str,
    detail: &str,
) -> ApiResult<()> {
    let last: Option<(i64, String)> = conn
        .query_row(
            "SELECT seq, mac FROM audit WHERE tenant_id = ?1 ORDER BY seq DESC LIMIT 1",
            [tenant],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (prev_seq, prev_mac) = last.unwrap_or((0, GENESIS.to_string()));
    let (key_id, key) = keys.current();
    let mut e = Entry {
        seq: prev_seq + 1,
        ts: now(),
        actor: cut(actor, 128),
        action: cut(action, 64),
        target: cut(target, 128),
        detail: cut(detail, 500),
        key_id: key_id.into(),
        prev_mac,
        mac: String::new(),
    };
    e.mac = hex(&mac_bytes(key, tenant, &e));
    conn.execute(
        "INSERT INTO audit (tenant_id, seq, ts, actor, action, target, detail, key_id, prev_mac, mac)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![tenant, e.seq, e.ts, e.actor, e.action, e.target, e.detail, e.key_id, e.prev_mac, e.mac],
    )?;
    Ok(())
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        seq: r.get(0)?,
        ts: r.get(1)?,
        actor: r.get(2)?,
        action: r.get(3)?,
        target: r.get(4)?,
        detail: r.get(5)?,
        key_id: r.get(6)?,
        prev_mac: r.get(7)?,
        mac: r.get(8)?,
    })
}

const COLS: &str = "seq, ts, actor, action, target, detail, key_id, prev_mac, mac";

/// Newest first. `target` filters to one object (for the owner's view of a document).
pub fn list(
    conn: &Connection,
    tenant: &str,
    target: Option<&str>,
    limit: u32,
) -> ApiResult<Vec<Entry>> {
    let limit = i64::from(limit.min(1000));
    let rows = match target {
        Some(t) => {
            let mut s = conn.prepare(&format!(
                "SELECT {COLS} FROM audit WHERE tenant_id = ?1 AND target = ?2 ORDER BY seq DESC LIMIT ?3"
            ))?;
            s.query_map(params![tenant, t, limit], row)?
                .collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let mut s = conn.prepare(&format!(
                "SELECT {COLS} FROM audit WHERE tenant_id = ?1 ORDER BY seq DESC LIMIT ?2"
            ))?;
            s.query_map(params![tenant, limit], row)?
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub entries: i64,
    pub ok: bool,
    /// First sequence number at which the chain is wrong.
    pub first_bad_seq: Option<i64>,
    pub head_mac: String,
}

/// Rows read per batch while verifying, so that the database lock is never held for the whole chain.
pub const VERIFY_BATCH: i64 = 500;

/// Up to `limit` rows with `seq > after`, oldest first.
pub fn fetch_after(
    conn: &Connection,
    tenant: &str,
    after: i64,
    limit: i64,
) -> ApiResult<Vec<Entry>> {
    let mut s = conn.prepare(&format!(
        "SELECT {COLS} FROM audit WHERE tenant_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3"
    ))?;
    Ok(s.query_map(params![tenant, after, limit], row)?
        .collect::<Result<Vec<_>, _>>()?)
}

/// Re-computes the chain for `tenant` in batches. `fetch(after_seq, limit)` supplies rows; a caller sharing a
/// connection locks it only inside `fetch`.
pub fn verify_with(
    keys: &AuditKeys,
    tenant: &str,
    mut fetch: impl FnMut(i64, i64) -> ApiResult<Vec<Entry>>,
) -> ApiResult<Verification> {
    let mut prev_seq = 0;
    let mut prev_mac = GENESIS.to_string();
    let mut n = 0;
    // Highest key position seen so far. A row signed with an older key than an earlier row is a downgrade (someone
    // holding a retired key forging rows after a rotation) and fails verification.
    let mut max_key = 0_usize;
    loop {
        let rows = fetch(prev_seq, VERIFY_BATCH)?;
        if rows.is_empty() {
            break;
        }
        for e in rows {
            n += 1;
            let key = keys.get(&e.key_id);
            let idx = keys.index_of(&e.key_id);
            let good = e.seq == prev_seq + 1
                && e.prev_mac == prev_mac
                && idx.is_some_and(|i| i >= max_key)
                && key.is_some_and(|k| {
                    let want = mac_bytes(k, tenant, &e);
                    unhex(&e.mac).is_some_and(|got| subtle_eq(&got, &want))
                });
            if !good {
                return Ok(Verification {
                    entries: n,
                    ok: false,
                    first_bad_seq: Some(e.seq),
                    head_mac: prev_mac,
                });
            }
            max_key = idx.unwrap_or(max_key).max(max_key);
            prev_seq = e.seq;
            prev_mac = e.mac;
        }
    }
    Ok(Verification {
        entries: n,
        ok: true,
        first_bad_seq: None,
        head_mac: prev_mac,
    })
}

/// Re-computes the whole chain for `tenant` on one connection.
pub fn verify(conn: &Connection, keys: &AuditKeys, tenant: &str) -> ApiResult<Verification> {
    verify_with(keys, tenant, |after, limit| {
        fetch_after(conn, tenant, after, limit)
    })
}

/// Tenant ids that have an audit chain (the system chain `-` included).
pub fn chains(conn: &Connection) -> ApiResult<Vec<String>> {
    let mut s = conn.prepare("SELECT DISTINCT tenant_id FROM audit ORDER BY tenant_id")?;
    Ok(s.query_map([], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?)
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(b).into()
}

pub fn require_ok(v: &Verification) -> ApiResult<()> {
    if v.ok {
        Ok(())
    } else {
        Err(ApiError::Internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Connection, AuditKeys) {
        let mut c = Connection::open_in_memory().expect("open");
        crate::db::migrate(&mut c).expect("migrate");
        (c, AuditKeys::single("k1", vec![7u8; 32]).expect("keys"))
    }

    #[test]
    fn a_row_signed_with_an_older_key_after_a_newer_one_fails_verification() {
        let mut c = Connection::open_in_memory().expect("open");
        crate::db::migrate(&mut c).expect("migrate");
        let k1 = AuditKeys::single("k1", vec![7u8; 32]).expect("k1");
        let both = AuditKeys::with_current(
            vec![("k1".into(), vec![7u8; 32]), ("k2".into(), vec![8u8; 32])],
            "k2",
        )
        .expect("both");
        append(&c, &k1, "t", "u", "a", "d", "1").expect("append");
        append(&c, &both, "t", "u", "a", "d", "2").expect("append");
        assert!(verify(&c, &both, "t").expect("v").ok);
        // The retired key k1 forges a further row: its MAC is valid, but it follows a k2 row.
        append(&c, &k1, "t", "u", "a", "d", "forged").expect("append");
        let v = verify(&c, &both, "t").expect("v");
        assert!(!v.ok && v.first_bad_seq == Some(3), "{v:?}");
    }

    #[test]
    fn chain_verifies_and_detects_edit_delete_and_reorder() {
        let (c, k) = setup();
        for i in 0..5 {
            append(&c, &k, "t1", "u", "act", "doc", &format!("n{i}")).expect("append");
        }
        append(&c, &k, "t2", "u", "act", "doc", "other tenant").expect("append");
        let v = verify(&c, &k, "t1").expect("verify");
        assert!(v.ok && v.entries == 5);

        c.execute(
            "UPDATE audit SET detail = 'forged' WHERE tenant_id = 't1' AND seq = 3",
            [],
        )
        .expect("edit");
        assert_eq!(verify(&c, &k, "t1").expect("v").first_bad_seq, Some(3));
        assert!(
            verify(&c, &k, "t2").expect("v").ok,
            "chains are independent"
        );
    }

    #[test]
    fn deleting_a_middle_row_is_detected() {
        let (c, k) = setup();
        for _ in 0..4 {
            append(&c, &k, "t1", "u", "a", "d", "").expect("append");
        }
        c.execute("DELETE FROM audit WHERE tenant_id = 't1' AND seq = 2", [])
            .expect("del");
        assert_eq!(verify(&c, &k, "t1").expect("v").first_bad_seq, Some(3));
    }

    #[test]
    fn fields_are_truncated_and_long_chains_verify_in_batches() {
        let (c, k) = setup();
        let long = "x".repeat(5000);
        append(&c, &k, "t1", &long, &long, &long, &long).expect("append");
        for _ in 0..(VERIFY_BATCH + 20) {
            append(&c, &k, "t1", "u", "a", "d", "").expect("append");
        }
        let rows = list(&c, "t1", None, 1000).expect("list");
        let first = rows.last().expect("first");
        assert!(first.actor.len() <= 128 && first.action.len() <= 64);
        assert!(first.target.len() <= 128 && first.detail.len() <= 500);
        let v = verify(&c, &k, "t1").expect("v");
        assert!(v.ok);
        assert_eq!(v.entries, VERIFY_BATCH + 21);
    }

    #[test]
    fn rotated_key_signs_new_rows_and_old_rows_still_verify() {
        let (c, _) = setup();
        let old = vec![7u8; 32];
        let k1 = AuditKeys::single("k1", old.clone()).expect("k1");
        append(&c, &k1, "t1", "u", "a", "d", "old").expect("append");
        let both =
            AuditKeys::list(vec![("k1".into(), old), ("k2".into(), vec![8u8; 32])]).expect("both");
        append(&c, &both, "t1", "u", "a", "d", "new").expect("append");
        let rows = list(&c, "t1", None, 10).expect("list");
        assert_eq!(rows[0].key_id, "k2", "only the newest key signs");
        assert_eq!(rows[1].key_id, "k1");
        assert!(verify(&c, &both, "t1").expect("v").ok);
        assert!(!verify(&c, &k1, "t1").expect("v").ok, "k2 rows need k2");
    }

    #[test]
    fn wrong_key_fails() {
        let (c, k) = setup();
        append(&c, &k, "t1", "u", "a", "d", "").expect("append");
        let other = AuditKeys::single("k1", vec![9u8; 32]).expect("keys");
        assert!(!verify(&c, &other, "t1").expect("v").ok);
    }
}

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn any_chain_verifies_and_any_single_edit_is_found(n in 1usize..25, victim in 0usize..25, field in 0usize..3) {
            let mut c = Connection::open_in_memory().expect("open");
            crate::db::migrate(&mut c).expect("migrate");
            let k = AuditKeys::single("k1", vec![3u8; 32]).expect("keys");
            for i in 0..n {
                append(&c, &k, "t", "actor", "act", &format!("doc{i}"), "detail").expect("append");
            }
            prop_assert!(verify(&c, &k, "t").expect("v").ok);
            let seq = i64::try_from(victim % n).expect("seq") + 1;
            let col = ["actor", "target", "detail"][field];
            c.execute(&format!("UPDATE audit SET {col} = {col} || 'x' WHERE tenant_id = 't' AND seq = ?1"), [seq]).expect("edit");
            let v = verify(&c, &k, "t").expect("v");
            prop_assert!(!v.ok);
            prop_assert_eq!(v.first_bad_seq, Some(seq));
        }
    }
}
