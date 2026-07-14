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
//! Heartbeat monitors ride the same pipeline with their own payload shape (see
//! [`HeartbeatKind`]): `heartbeat_down` from the sweep job, `heartbeat_recovered` from the
//! ping endpoint.
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
    Spike,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatKind {
    HeartbeatDown,
    HeartbeatRecovered,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestKind {
    Digest,
}

/// One message through the pipeline. `untagged` keeps the wire format of issue notifications
/// byte-identical to what it was before heartbeats existed — each variant carries its own
/// `kind` field, and the generic webhook consumer discriminates on that, not on an outer tag.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Notification {
    Issue(IssueNotification),
    Heartbeat(HeartbeatNotification),
    Digest(DigestNotification),
}

#[derive(Debug, Clone, Serialize)]
pub struct IssueNotification {
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
    /// Set only when `kind == Spike`: events seen in the last hour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_hour: Option<i64>,
    /// Set only when `kind == Spike`: events-per-hour averaged over the prior 23h.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_per_hour: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatNotification {
    pub kind: HeartbeatKind,
    pub project_name: String,
    pub project_slug: String,
    pub monitor_id: i64,
    pub monitor_name: String,
    /// Set only for `HeartbeatDown`: seconds past the ping deadline (`last_ping + period + grace`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overdue_seconds: Option<i64>,
    /// Set only for `HeartbeatRecovered`: seconds the monitor spent down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downtime_seconds: Option<i64>,
    pub link: String,
}

/// Periodic per-project summary fired by the digest job. Only sent when the window saw
/// activity — an empty digest is noise.
#[derive(Debug, Clone, Serialize)]
pub struct DigestNotification {
    pub kind: DigestKind,
    pub project_name: String,
    pub project_slug: String,
    /// Actual covered window (anchor → now), which can exceed the configured cadence after
    /// downtime.
    pub window_hours: i64,
    pub new_issues: i64,
    pub events: i64,
    /// Busiest issues of the window, largest first (bounded).
    pub top_issues: Vec<DigestTopIssue>,
    pub link: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DigestTopIssue {
    pub issue_id: i64,
    pub title: String,
    pub events: i64,
}

impl Notification {
    /// Compact human-friendly subject line, used by Telegram/Discord text bodies.
    pub fn subject(&self) -> String {
        match self {
            Self::Issue(n) => n.subject(),
            Self::Heartbeat(n) => n.subject(),
            Self::Digest(n) => n.subject(),
        }
    }

    pub fn link(&self) -> &str {
        match self {
            Self::Issue(n) => &n.link,
            Self::Heartbeat(n) => &n.link,
            Self::Digest(n) => &n.link,
        }
    }
}

impl IssueNotification {
    pub fn subject(&self) -> String {
        let prefix = match self.kind {
            Kind::NewIssue => "🆕 new issue",
            Kind::Reopened => "🔁 reopened",
            Kind::Spike => "🔥 spike",
        };
        match (self.kind, self.current_hour, self.baseline_per_hour) {
            (Kind::Spike, Some(cur), Some(base)) => format!(
                "[{}] {prefix}: {} — {cur}/h (was ~{base:.1}/h)",
                self.project_slug, self.issue_title,
            ),
            _ => format!("[{}] {prefix}: {}", self.project_slug, self.issue_title),
        }
    }
}

impl HeartbeatNotification {
    pub fn subject(&self) -> String {
        match self.kind {
            HeartbeatKind::HeartbeatDown => {
                let tail = self
                    .overdue_seconds
                    .map(|s| format!(" — overdue by {}", fmt_duration(s)))
                    .unwrap_or_default();
                format!(
                    "[{}] 💀 heartbeat down: {}{tail}",
                    self.project_slug, self.monitor_name
                )
            }
            HeartbeatKind::HeartbeatRecovered => {
                let tail = self
                    .downtime_seconds
                    .map(|s| format!(" — was down {}", fmt_duration(s)))
                    .unwrap_or_default();
                format!(
                    "[{}] 💚 heartbeat recovered: {}{tail}",
                    self.project_slug, self.monitor_name
                )
            }
        }
    }
}

impl DigestNotification {
    pub fn subject(&self) -> String {
        format!(
            "[{}] 🗞 digest: {} new issue{}, {} event{} in {}h",
            self.project_slug,
            self.new_issues,
            if self.new_issues == 1 { "" } else { "s" },
            self.events,
            if self.events == 1 { "" } else { "s" },
            self.window_hours,
        )
    }
}

/// Human-compact duration for subject lines: `45s`, `12m`, `3h`, `2d`.
fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
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
            .map(|_| Arc::new(Mutex::new(TokenBucket::new(cfg.notify.max_per_minute))))
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

    pub fn build_heartbeat_link(&self, project_id: i64) -> String {
        let base = self.public_url.trim_end_matches('/');
        format!("{base}/projects/{project_id}/heartbeats")
    }

    pub fn build_project_link(&self, project_id: i64) -> String {
        let base = self.public_url.trim_end_matches('/');
        format!("{base}/projects/{project_id}/issues")
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
                        subject = %msg.subject(),
                        "notify: rate-limited, dropping"
                    );
                    continue;
                }
                match n.send(&msg).await {
                    Ok(()) => tracing::debug!(
                        notifier = n.name(),
                        subject = %msg.subject(),
                        "notify: delivered"
                    ),
                    Err(e) => tracing::warn!(
                        notifier = n.name(),
                        subject = %msg.subject(),
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

    fn issue_notification() -> IssueNotification {
        IssueNotification {
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
            current_hour: None,
            baseline_per_hour: None,
        }
    }

    #[test]
    fn subject_format() {
        let s = Notification::Issue(issue_notification()).subject();
        assert!(s.contains("demo"));
        assert!(s.contains("new issue"));
        assert!(s.contains("TypeError: x"));
    }

    #[test]
    fn spike_subject_includes_rate() {
        let n = IssueNotification {
            kind: Kind::Spike,
            event_count: 30,
            current_hour: Some(30),
            baseline_per_hour: Some(0.22),
            ..issue_notification()
        };
        let s = Notification::Issue(n).subject();
        assert!(s.contains("🔥 spike"), "got {s}");
        assert!(s.contains("30/h"));
        assert!(s.contains("0.2/h"));
    }

    /// The generic-webhook wire format for issue notifications predates heartbeats. The
    /// `untagged` enum must keep it byte-identical — this test pins the exact shape.
    #[test]
    fn issue_wire_format_is_unchanged_by_the_enum() {
        let v =
            serde_json::to_value(Notification::Issue(issue_notification())).expect("serializable");
        assert_eq!(
            v,
            serde_json::json!({
                "kind": "new_issue",
                "project_name": "Demo",
                "project_slug": "demo",
                "issue_id": 7,
                "issue_title": "TypeError: x",
                "event_count": 1,
                "level": "error",
                "environment": null,
                "release": null,
                "link": "http://localhost/issues/7",
            })
        );
    }

    #[test]
    fn heartbeat_subjects_and_wire_format() {
        let down = HeartbeatNotification {
            kind: HeartbeatKind::HeartbeatDown,
            project_name: "Demo".into(),
            project_slug: "demo".into(),
            monitor_id: 3,
            monitor_name: "nightly backup".into(),
            overdue_seconds: Some(300),
            downtime_seconds: None,
            link: "http://localhost/projects/1/heartbeats".into(),
        };
        let s = Notification::Heartbeat(down.clone()).subject();
        assert!(s.contains("💀 heartbeat down"), "got {s}");
        assert!(s.contains("nightly backup"));
        assert!(s.contains("overdue by 5m"));

        let v = serde_json::to_value(Notification::Heartbeat(down)).expect("serializable");
        assert_eq!(v["kind"], "heartbeat_down");
        assert_eq!(v["overdue_seconds"], 300);
        assert!(v.get("downtime_seconds").is_none(), "None fields skipped");
        assert!(v.get("issue_id").is_none(), "no issue fields on heartbeat");

        let recovered = HeartbeatNotification {
            kind: HeartbeatKind::HeartbeatRecovered,
            project_name: "Demo".into(),
            project_slug: "demo".into(),
            monitor_id: 3,
            monitor_name: "nightly backup".into(),
            overdue_seconds: None,
            downtime_seconds: Some(7200),
            link: "x".into(),
        };
        let s = Notification::Heartbeat(recovered).subject();
        assert!(s.contains("💚 heartbeat recovered"), "got {s}");
        assert!(s.contains("was down 2h"));
    }

    #[test]
    fn fmt_duration_units() {
        assert_eq!(fmt_duration(45), "45s");
        assert_eq!(fmt_duration(300), "5m");
        assert_eq!(fmt_duration(7200), "2h");
        assert_eq!(fmt_duration(200_000), "2d");
        assert_eq!(fmt_duration(-5), "0s");
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
