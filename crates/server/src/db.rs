//! SQLite access (WAL) and forward-only migrations tracked in `PRAGMA user_version`.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::error::ApiResult;

const MIGRATIONS: &[&str] = &[include_str!("../migrations/001_init.sql")];

#[derive(Debug)]
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> ApiResult<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn migrate(conn: &mut Connection) -> ApiResult<()> {
    let current =
        usize::try_from(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?)
            .unwrap_or(0);
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(
            None,
            "user_version",
            i64::try_from(i + 1).unwrap_or(i64::MAX),
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_are_idempotent() {
        let mut c = Connection::open_in_memory().expect("open");
        migrate(&mut c).expect("first");
        migrate(&mut c).expect("second");
        let v: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(usize::try_from(v).expect("v"), MIGRATIONS.len());
    }
}
