//! GET /api/admin/backup — download an atomic snapshot of the SQLite database.
//!
//! GET is deliberate: read-scope API tokens can pull backups (semantically a read, even
//! though a temp file is written). The snapshot is written next to the live DB (same
//! volume), streamed to the client, and deleted when the response body is dropped —
//! success, error, or client disconnect alike.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use chrono::Utc;
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;
use ulid::Ulid;

use crate::app_state::AppState;
use crate::db;
use crate::http::error::{AppError, AppResult};
use crate::security::sessions::AuthUser;

// One backup at a time: VACUUM INTO transiently doubles disk usage; concurrent snapshots
// would multiply that. Process-wide by nature — the process is the whole deployment.
static BACKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Deletes the temp snapshot on drop. `remove_file` is a blocking syscall, but unlinking one
/// file is far below the runtime's blocking threshold.
struct TempFileGuard(std::path::PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            tracing::warn!(path = %self.0.display(), error = %e, "failed to remove backup temp file");
        }
    }
}

pub async fn download(State(state): State<AppState>, _user: AuthUser) -> AppResult<Response> {
    let Ok(_vacuum_guard) = BACKUP_LOCK.try_lock() else {
        return Err(AppError::Conflict("a backup is already in progress".into()));
    };

    let db_path = db::database_file_path(&state.config.database_url);
    let dir = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::env::temp_dir(),
    };
    let temp_path = dir.join(format!(
        ".crashbox-backup-{}.db",
        Ulid::new().to_string().to_lowercase()
    ));

    db::vacuum_into(&state.db, &temp_path).await?;
    let cleanup = TempFileGuard(temp_path.clone());

    let file = tokio::fs::File::open(&temp_path)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let len = file.metadata().await.map(|m| m.len()).ok();
    tracing::info!(size_bytes = len, "streaming database backup");

    // The guard rides inside the stream closure so the temp file outlives the download and
    // no longer needs the lock — the expensive VACUUM phase is over.
    let stream = ReaderStream::new(file).map(move |chunk| {
        let _keep_until_body_drops = &cleanup;
        chunk
    });

    let filename = format!("crashbox-{}.db", Utc::now().format("%Y%m%d-%H%M%S"));
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        );
    if let Some(len) = len {
        builder = builder.header(header::CONTENT_LENGTH, len);
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| AppError::Internal(e.into()))
}
