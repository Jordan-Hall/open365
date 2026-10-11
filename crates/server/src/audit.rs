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
        actor: actor.into(),
        action: action.into(),
        target: target.into(),
        detail: detail.chars().take(500).collect(),
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

/// Re-computes the whole chain for `tenant`.
pub fn verify(conn: &Connection, keys: &AuditKeys, tenant: &str) -> ApiResult<Verification> {
    let mut s = conn.prepare(&format!(
        "SELECT {COLS} FROM audit WHERE tenant_id = ?1 ORDER BY seq ASC"
    ))?;
    let rows = s.query_map([tenant], row)?;
    let mut prev_seq = 0;
    let mut prev_mac = GENESIS.to_string();
    let mut n = 0;
    for r in rows {
        let e = r?;
        n += 1;
        let key = keys.get(&e.key_id);
        let good = e.seq == prev_seq + 1
            && e.prev_mac == prev_mac
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
        prev_seq = e.seq;
        prev_mac = e.mac;
    }
    Ok(Verification {
        entries: n,
        ok: true,
        first_bad_seq: None,
        head_mac: prev_mac,
    })
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
    fn wrong_key_fails() {
        let (c, k) = setup();
        append(&c, &k, "t1", "u", "a", "d", "").expect("append");
        let other = AuditKeys::single("k1", vec![9u8; 32]).expect("keys");
        assert!(!verify(&c, &other, "t1").expect("v").ok);
    }
}
