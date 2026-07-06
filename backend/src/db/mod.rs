pub mod events;
pub mod heartbeats;
pub mod issues;
pub mod projects;
pub mod users;

use std::path::Path;

use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool};
// Sqlite import is kept — used in PoolConnection<Sqlite> below.

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let opts = parse_sqlite_options(database_url);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

/// Begin a SQLite write transaction with `BEGIN IMMEDIATE`.
///
/// Why this exists, not `pool.begin()`:
/// the default sqlx `pool.begin()` runs `BEGIN` (= `BEGIN DEFERRED`), which acquires SHARED at
/// start and tries to upgrade to RESERVED on first write. Two parallel transactions can both
/// hold SHARED, both try to upgrade, and one (or both) get `SQLITE_BUSY`. `busy_timeout` retries
/// but contending transactions can keep racing and fail under bursts.
///
/// `BEGIN IMMEDIATE` takes the RESERVED lock at `BEGIN` time, so concurrent write
/// transactions cleanly queue on `busy_timeout` instead of fighting on upgrade. Standard
/// SQLite-with-pool pattern.
///
/// Use this for any path that issues INSERTs/UPDATEs as part of a multi-statement transaction.
pub async fn begin_write(pool: &SqlitePool) -> sqlx::Result<WriteTx> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    Ok(WriteTx {
        conn: Some(conn),
        committed: false,
    })
}

/// RAII handle for a `BEGIN IMMEDIATE` transaction.
///
/// On drop without `commit()` the transaction is rolled back. This mirrors sqlx's own
/// `Transaction` semantics but uses raw `BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK`.
pub struct WriteTx {
    conn: Option<PoolConnection<Sqlite>>,
    committed: bool,
}

// The two `expect`s below assert an internal invariant (commit/acquire after the tx is finished is
// a programmer error, never runtime input) — not the malformed-input path the no-panic rule guards.
#[allow(clippy::expect_used)]
impl WriteTx {
    pub async fn commit(mut self) -> sqlx::Result<()> {
        let mut conn = self.conn.take().expect("commit on already-finished tx");
        sqlx::query("COMMIT").execute(&mut *conn).await?;
        self.committed = true;
        Ok(())
    }

    /// Acquire a mutable reference suitable for `sqlx::query(...).execute(&mut *tx.acquire())`.
    /// Returned reference is a connection, not a sqlx::Transaction, so callers should NOT call
    /// `.begin()` on it — they'd nest into a SAVEPOINT and confuse the lock model.
    pub fn acquire(&mut self) -> &mut sqlx::SqliteConnection {
        self.conn
            .as_deref_mut()
            .expect("acquire on already-finished tx")
    }
}

impl Drop for WriteTx {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Best-effort rollback on a connection that's about to be returned to the pool.
        // If this fails the connection is dropped (and reopened from scratch on next acquire),
        // so we don't leak a half-open transaction.
        if let Some(mut conn) = self.conn.take() {
            tokio::spawn(async move {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            });
        }
    }
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

fn parse_sqlite_options(url: &str) -> SqliteConnectOptions {
    let raw = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);

    if let Some(parent) = Path::new(raw).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }

    SqliteConnectOptions::new()
        .filename(raw)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(15))
        .foreign_keys(true)
}
