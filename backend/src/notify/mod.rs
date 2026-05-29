//! Notification delivery.
//!
//! Triggers (see [`Kind`]):
//! - `NewIssue`: first event of a previously unseen `fingerprint` (the issue row was just
//!   inserted in this transaction).
//! - `Reopened`: an event landed on an issue whose status was `resolved`; we flipped it back to
//!   `unresolved` and emit a notification so the team knows it's not actually fixed.
//!
//! Triggers **never** fire on subsequent events of an already-unresolved issue — that path is
//! what `spike` detection (A2) is for. Documented in `docs/configuration.md`.
//!
//! Delivery is fire-and-forget via `tokio::spawn` from the ingest path: a slow Telegram API
//! must not block a Sentry SDK's request. Failures are logged at `WARN` and counted (future
//! `/metrics`).

pub mod discord;
pub mod telegram;
pub mod webhook;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::config::Config;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NewIssue,
    Reopened,
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub kind: Kind,
    pub project_name: String,
    pub project_slug: String,
    pub issue_id: i64,
    pub issue_title: String,
    pub event_count: i64,
    pub level: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub link: String,
}

impl Notification {
    /// Compact human-friendly subject line, used by Telegram/Discord text bodies.
    pub fn subject(&self) -> String {
        let prefix = match self.kind {
            Kind::NewIssue => "🆕 new issue",
            Kind::Reopened => "🔁 reopened",
        };
        format!("[{}] {prefix}: {}", self.project_slug, self.issue_title)
    }
}

#[async_trait]
pub trait Notifier: Send + Sync {
    fn name(&self) -> &'static str;
    async fn send(&self, msg: &Notification) -> Result<(), NotifyError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("http transport: {0}")]
    Http(#[from] reqwest::Error),
    #[error("upstream returned {status}: {body}")]
    BadStatus { status: u16, body: String },
}

/// Owns a list of configured notifiers and a per-notifier token bucket.
pub struct NotifyHub {
    pub notifiers: Vec<Arc<dyn Notifier>>,
    pub limiters: Vec<Arc<Mutex<TokenBucket>>>,
    pub public_url: String,
}

impl NotifyHub {
    /// Build from the validated `Config`. Empty if no notifier env vars are set; in that case
    /// `fire` is a no-op.
    pub fn from_config(cfg: &Config) -> Self {
        let mut notifiers: Vec<Arc<dyn Notifier>> = Vec::new();
        if let (Some(token), Some(chat_id)) = (
            cfg.notify.telegram_bot_token.as_ref(),
            cfg.notify.telegram_chat_id.as_ref(),
        ) {
            notifiers.push(Arc::new(telegram::TelegramNotifier::new(
                token.clone(),
                chat_id.clone(),
            )));
        }
        if let Some(url) = cfg.notify.discord_webhook_url.as_ref() {
            notifiers.push(Arc::new(discord::DiscordNotifier::new(url.clone())));
        }
        if let Some(url) = cfg.notify.generic_webhook_url.as_ref() {
            notifiers.push(Arc::new(webhook::GenericWebhook::new(url.clone())));
        }

        let limiters = (0..notifiers.len())
            .map(|_| {
                Arc::new(Mutex::new(TokenBucket::new(
                    cfg.notify.max_per_minute.into(),
                )))
            })
            .collect();

        Self {
            notifiers,
            limiters,
            public_url: cfg.public_url.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.notifiers.is_empty()
    }

    pub fn build_link(&self, issue_id: i64) -> String {
        let base = self.public_url.trim_end_matches('/');
        format!("{base}/issues/{issue_id}")
    }

    /// Fire-and-forget. Spawns tasks for each notifier; returns immediately. Drops the
    /// notification for any notifier whose rate bucket is empty (logged at `INFO`).
    pub fn fire(self: &Arc<Self>, msg: Notification) {
        if self.notifiers.is_empty() {
            return;
        }
        let hub = self.clone();
        tokio::spawn(async move {
            for (n, limiter) in hub.notifiers.iter().zip(hub.limiters.iter()) {
                let allowed = {
                    let mut bucket = limiter.lock().await;
                    bucket.try_take()
                };
                if !allowed {
                    tracing::info!(
                        notifier = n.name(),
                        issue_id = msg.issue_id,
                        "notify: rate-limited, dropping"
                    );
                    continue;
                }
                match n.send(&msg).await {
                    Ok(()) => tracing::debug!(
                        notifier = n.name(),
                        issue_id = msg.issue_id,
                        kind = ?msg.kind,
                        "notify: delivered"
                    ),
                    Err(e) => tracing::warn!(
                        notifier = n.name(),
                        issue_id = msg.issue_id,
                        error = %e,
                        "notify: delivery failed"
                    ),
                }
            }
        });
    }
}

/// Per-notifier token bucket. Capacity = `max_per_minute`; refills at `max_per_minute / 60`
/// tokens per second. Same shape as `ingest::rate_limit::RateLimiter` but a separate
/// implementation because keying differs (here: per-notifier, not per-project).
#[derive(Debug)]
pub struct TokenBucket {
    capacity: f64,
    refill_per_sec: f64,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(max_per_minute: u32) -> Self {
        let cap = f64::from(max_per_minute.max(1));
        Self {
            capacity: cap,
            refill_per_sec: cap / 60.0,
            tokens: cap,
            last_refill: Instant::now(),
        }
    }

    pub fn try_take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_format() {
        let n = Notification {
            kind: Kind::NewIssue,
            project_name: "Demo".into(),
            project_slug: "demo".into(),
            issue_id: 7,
            issue_title: "TypeError: x".into(),
            event_count: 1,
            level: Some("error".into()),
            environment: None,
            release: None,
            link: "http://localhost/issues/7".into(),
        };
        let s = n.subject();
        assert!(s.contains("demo"));
        assert!(s.contains("new issue"));
        assert!(s.contains("TypeError: x"));
    }

    #[test]
    fn token_bucket_respects_capacity() {
        let mut b = TokenBucket::new(3);
        assert!(b.try_take());
        assert!(b.try_take());
        assert!(b.try_take());
        assert!(!b.try_take());
    }
}
