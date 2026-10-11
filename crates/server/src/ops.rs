//! Operator commands: online backup, restore with verification, and audit-chain verification from the command line.
//!
//! A backup is a directory holding `open365.db` (a transactionally consistent snapshot made with `VACUUM INTO` while
//! the server keeps running) and a copy of `blobs/`. Keys (audit key, bootstrap token, TLS key) are never in the data
//! directory and so never in a backup. The backup holds password hashes, TOTP secrets and document bytes: store it
//! with the same care as the live data.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::audit::{self, Verification};
use crate::config::AuditKeys;
use crate::util::{make_private_dir, sha256_hex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub versions: usize,
    pub blobs: usize,
    pub bytes: u64,
}

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{what}: {e}")
}

fn copy_tree(src: &Path, dst: &Path, blobs: &mut usize, bytes: &mut u64) -> Result<(), String> {
    make_private_dir(dst).map_err(err("create directory"))?;
    let Ok(rd) = std::fs::read_dir(src) else {
        return Ok(());
    };
    for entry in rd {
        let entry = entry.map_err(err("read directory"))?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".tmp-") {
            continue;
        }
        let (from, to) = (entry.path(), dst.join(&name));
        let ty = entry.file_type().map_err(err("stat"))?;
        if ty.is_dir() {
            copy_tree(&from, &to, blobs, bytes)?;
        } else if ty.is_file() {
            *bytes += std::fs::copy(&from, &to).map_err(err("copy blob"))?;
            // Opened for writing: Windows refuses to sync a read-only handle.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&to)
                .and_then(|f| f.sync_all())
                .map_err(err("sync blob"))?;
            *blobs += 1;
        }
    }
    Ok(())
}

/// Checks that every version row in the database at `db` has its blob under `blobs` with the right hash.
fn check_blobs(db: &Path, blobs: &Path, rehash: bool) -> Result<usize, String> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err("open database"))?;
    let mut s = conn
        .prepare("SELECT tenant_id, blob_id FROM versions")
        .map_err(err("query versions"))?;
    let rows = s
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err("query versions"))?;
    let mut n = 0;
    for r in rows {
        let (tenant, blob) = r.map_err(err("read version"))?;
        if blob.len() < 2 {
            return Err(format!("bad blob id in database: {blob}"));
        }
        let p = blobs.join(&tenant).join(&blob[..2]).join(&blob);
        if rehash {
            let data = std::fs::read(&p).map_err(|e| format!("missing blob {blob}: {e}"))?;
            if sha256_hex(&data) != blob {
                return Err(format!("blob {blob} does not match its hash"));
            }
        } else if !p.is_file() {
            return Err(format!("missing blob {blob}"));
        }
        n += 1;
    }
    Ok(n)
}

/// Online backup of `data_dir` into the new directory `dest`.
pub fn backup(data_dir: &Path, dest: &Path) -> Result<Report, String> {
    if dest.exists()
        && std::fs::read_dir(dest)
            .map_err(err("read destination"))?
            .next()
            .is_some()
    {
        return Err(format!("{} is not empty", dest.display()));
    }
    make_private_dir(dest).map_err(err("create destination"))?;
    let live = data_dir.join("open365.db");
    let snapshot = dest.join("open365.db");
    {
        let conn = Connection::open_with_flags(&live, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(err("open database"))?;
        conn.busy_timeout(std::time::Duration::from_secs(30))
            .map_err(err("busy timeout"))?;
        conn.execute("VACUUM INTO ?1", params![snapshot.to_string_lossy()])
            .map_err(err("snapshot"))?;
    }
    // Blobs are immutable and content-addressed, so copying them after the snapshot is safe: every blob the
    // snapshot references already exists. The check below catches a purge that raced the copy.
    let (mut blobs, mut bytes) = (0, 0);
    copy_tree(
        &data_dir.join("blobs"),
        &dest.join("blobs"),
        &mut blobs,
        &mut bytes,
    )?;
    let versions = check_blobs(&snapshot, &dest.join("blobs"), false)?;
    Ok(Report {
        versions,
        blobs,
        bytes,
    })
}

/// Restores `backup_dir` into an empty `data_dir`, then verifies the result: SQLite integrity, every blob's hash,
/// and (with `keys`) every audit chain. A failed restore leaves no database behind.
pub fn restore(
    backup_dir: &Path,
    data_dir: &Path,
    keys: Option<&AuditKeys>,
) -> Result<Report, String> {
    let src_db = backup_dir.join("open365.db");
    if !src_db.is_file() {
        return Err(format!("{} has no open365.db", backup_dir.display()));
    }
    let dst_db = data_dir.join("open365.db");
    if dst_db.exists() {
        return Err(format!(
            "{} already holds a database; restore into an empty directory",
            data_dir.display()
        ));
    }
    make_private_dir(data_dir).map_err(err("create data directory"))?;
    let result = (|| {
        std::fs::copy(&src_db, &dst_db).map_err(err("copy database"))?;
        let (mut blobs, mut bytes) = (0, 0);
        copy_tree(
            &backup_dir.join("blobs"),
            &data_dir.join("blobs"),
            &mut blobs,
            &mut bytes,
        )?;
        {
            let conn = Connection::open_with_flags(&dst_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(err("open restored database"))?;
            let verdict: String = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(err("integrity check"))?;
            if verdict != "ok" {
                return Err(format!("integrity check failed: {verdict}"));
            }
        }
        let versions = check_blobs(&dst_db, &data_dir.join("blobs"), true)?;
        if let Some(keys) = keys {
            for (chain, v) in audit_verify_all(data_dir, keys)? {
                if !v.ok {
                    return Err(format!(
                        "audit chain {chain} fails at {:?}",
                        v.first_bad_seq
                    ));
                }
            }
        }
        Ok(Report {
            versions,
            blobs,
            bytes,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&dst_db);
    }
    result
}

/// Verifies every audit chain (the system chain `-` and each tenant's) on a read-only connection.
pub fn audit_verify_all(
    data_dir: &Path,
    keys: &AuditKeys,
) -> Result<Vec<(String, Verification)>, String> {
    let conn = Connection::open_with_flags(
        data_dir.join("open365.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(err("open database"))?;
    let chains = audit::chains(&conn).map_err(|_| "cannot list audit chains".to_string())?;
    let mut out = vec![];
    for chain in chains {
        let v = audit::verify(&conn, keys, &chain)
            .map_err(|_| format!("cannot verify chain {chain}"))?;
        out.push((chain, v));
    }
    Ok(out)
}

/// What [`gc_blobs`] found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Blob files no version row refers to (or stale `.tmp-` files), older than the grace period.
    pub orphans: usize,
    pub bytes: u64,
    /// Files removed (zero in a dry run).
    pub removed: usize,
    /// Orphans spared because they are younger than the grace period.
    pub too_new: usize,
}

/// Removes blob files that no version refers to: leftovers of failed uploads, purged documents and interrupted
/// writes. A blob is written before the database row that refers to it, and a re-upload of identical bytes reuses
/// the file, so only files not modified for `min_age_secs` are touched, and each is re-checked against the
/// database immediately before it is deleted. Safe to run while the server is running. `dry_run` only counts.
pub fn gc_blobs(data_dir: &Path, min_age_secs: u64, dry_run: bool) -> Result<GcReport, String> {
    let conn = Connection::open_with_flags(
        data_dir.join("open365.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(err("open database"))?;
    let root = data_dir.join("blobs");
    let mut report = GcReport::default();
    let Ok(tenants) = std::fs::read_dir(&root) else {
        return Ok(report);
    };
    let grace = std::time::Duration::from_secs(min_age_secs);
    let old_enough = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= grace)
    };
    let referenced = |tenant: &str, blob: &str| -> Result<bool, String> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM versions WHERE tenant_id = ?1 AND blob_id = ?2)",
            params![tenant, blob],
            |r| r.get(0),
        )
        .map_err(err("query versions"))
    };
    for tenant in tenants {
        let tenant = tenant.map_err(err("read blobs"))?;
        let tname = tenant.file_name().to_string_lossy().into_owned();
        if !tenant.file_type().map_err(err("stat"))?.is_dir() {
            continue;
        }
        for shard in std::fs::read_dir(tenant.path()).map_err(err("read tenant"))? {
            let shard = shard.map_err(err("read shard"))?;
            if !shard.file_type().map_err(err("stat"))?.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(shard.path()).map_err(err("read shard"))? {
                let f = f.map_err(err("read blob"))?;
                let fname = f.file_name().to_string_lossy().into_owned();
                if !f.file_type().map_err(err("stat"))?.is_file() {
                    continue;
                }
                let is_tmp = fname.starts_with(".tmp-");
                if !is_tmp && referenced(&tname, &fname)? {
                    continue;
                }
                let path = f.path();
                if !old_enough(&path) {
                    report.too_new += 1;
                    continue;
                }
                report.orphans += 1;
                report.bytes += f.metadata().map_or(0, |m| m.len());
                // Re-check right before deleting: an upload may have adopted the blob in the meantime.
                if dry_run || (!is_tmp && referenced(&tname, &fname)?) || !old_enough(&path) {
                    continue;
                }
                std::fs::remove_file(&path).map_err(err("remove blob"))?;
                report.removed += 1;
            }
        }
    }
    Ok(report)
}

/// Seals plaintext TOTP secrets and moves values sealed under an older key to the newest key
/// (`open365-server rotate-secrets`). Run it after adding a key to the key list; once it reports nothing left under
/// the old key, that key can be removed from the list.
/// How many records each key id still protects, so an operator knows when an old key can be retired.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct KeyUse {
    /// `(key id, audit rows signed with it)`.
    pub audit: Vec<(String, i64)>,
    /// `(key id or "plaintext", TOTP secrets sealed with it)`.
    pub secrets: Vec<(String, i64)>,
}

pub fn key_status(data_dir: &Path) -> Result<KeyUse, String> {
    let conn = Connection::open_with_flags(
        data_dir.join("open365.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(err("open database"))?;
    let audit: Vec<(String, i64)> = conn
        .prepare("SELECT key_id, COUNT(*) FROM audit GROUP BY key_id ORDER BY key_id")
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()
        })
        .map_err(err("read audit"))?;
    let stored: Vec<String> = conn
        .prepare("SELECT totp_secret FROM users WHERE totp_secret IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>())
        .map_err(err("read secrets"))?;
    let mut secrets: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for v in stored {
        let id = v
            .strip_prefix("v1:")
            .and_then(|r| r.split_once(':'))
            .map_or("plaintext", |(id, _)| id);
        *secrets.entry(id.to_string()).or_insert(0) += 1;
    }
    Ok(KeyUse {
        audit,
        secrets: secrets.into_iter().collect(),
    })
}

pub fn rotate_secrets(
    data_dir: &Path,
    keys: &crate::secrets::SecretKeys,
) -> Result<crate::secrets::Resealed, String> {
    let mut conn = Connection::open(data_dir.join("open365.db")).map_err(err("open database"))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))
        .map_err(err("open database"))?;
    crate::secrets::reseal_all(&mut conn, keys).map_err(err("re-seal secrets"))
}
