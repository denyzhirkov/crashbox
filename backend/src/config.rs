use std::env;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(&'static str),
    #[error("invalid value for {var}: {source}")]
    Invalid {
        var: &'static str,
        #[source]
        source: anyhow::Error,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub public_url: String,
    pub database_url: String,
    pub data_dir: String,
    pub log_level: String,
    pub secret_key: String,

    pub admin: AdminBootstrap,
    pub project: ProjectBootstrap,
    pub ingest: IngestLimits,
    pub retention: Retention,
    pub spike: SpikeConfig,
    pub ui: UiConfig,
    pub security: SecurityConfig,
    pub notify: NotifyConfig,
    pub livelog: LiveLogConfig,
}

#[derive(Debug, Clone)]
pub struct AdminBootstrap {
    pub email: Option<String>,
    pub password: Option<String>,
    pub name: Option<String>,
    pub force_reset: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectBootstrap {
    pub name: Option<String>,
    pub platform: Option<String>,
    pub environment: Option<String>,
    pub public_key: Option<String>,
    pub secret_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IngestLimits {
    pub max_envelope_bytes: usize,
    pub max_event_bytes: usize,
    pub max_events_per_minute_per_project: u32,
    pub accept_unknown_item_types: bool,
    pub store_raw_unsupported_items: bool,
    pub enable_legacy_store_endpoint: bool,
}

#[derive(Debug, Clone)]
pub struct Retention {
    pub retention_days: u32,
    pub max_events_per_issue: u32,
    pub cleanup_interval_seconds: u64,
    /// Auto-resolve issues that haven't seen an event for this many days. `0` disables.
    pub auto_resolve_days: u32,
}

#[derive(Debug, Clone)]
pub struct SpikeConfig {
    /// How often to run a spike check. Default 5 min. `0` disables the job.
    pub check_interval_seconds: u64,
    /// Minimum events in the last hour to consider an issue spiking. Default 10 — below this,
    /// noise dominates and we'd alert on bumps that don't matter.
    pub min_events_per_hour: u32,
    /// Required ratio of (current hour rate) / (prior-23h baseline). Default 5.0.
    pub ratio_threshold: f64,
    /// Per-issue cooldown in seconds after a spike alert. Default 3600 (1h).
    pub cooldown_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct UiConfig {
    pub enabled: bool,
    pub app_name: String,
    pub theme: String,
}

#[derive(Debug, Clone)]
pub struct SecurityConfig {
    pub cookie_secure: bool,
    pub cors_allowed_origins: String,
    pub trust_proxy_headers: bool,
    pub allow_public_signup: bool,
}

#[derive(Debug, Clone)]
pub struct NotifyConfig {
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    pub discord_webhook_url: Option<String>,
    pub generic_webhook_url: Option<String>,
    /// Per-notifier cap. Defaults to 30/min so a sudden burst of new issues doesn't drown the
    /// channel; rate-limited messages are dropped (logged) rather than queued.
    pub max_per_minute: u32,
}

/// Live Logs — ephemeral, RAM-only real-time log streaming, separate from durable events.
/// Nothing here is persisted: a per-project ring buffer holds recent records for scrollback and
/// evaporates on restart.
#[derive(Debug, Clone)]
pub struct LiveLogConfig {
    pub enabled: bool,
    /// Ring buffer size per project (scrollback served to a freshly-connected stream).
    pub buffer_per_project: usize,
    /// Max accepted body size for the `/logs` ingest endpoint, checked before allocation.
    pub max_batch_bytes: usize,
    /// Per-record message cap; longer messages are truncated on a char boundary.
    pub message_max_bytes: usize,
    pub max_per_minute_per_project: u32,
    /// Cap on concurrent SSE subscribers per project — guards against leaked streams.
    pub max_subscribers_per_project: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            host: env_or("CRASHBOX_HOST", "0.0.0.0"),
            port: parse_env("CRASHBOX_PORT", "8080")?,
            public_url: env_or("CRASHBOX_PUBLIC_URL", "http://localhost:8080"),
            database_url: env_or("CRASHBOX_DATABASE_URL", "sqlite://./data/crashbox.db"),
            data_dir: env_or("CRASHBOX_DATA_DIR", "./data"),
            log_level: env_or("CRASHBOX_LOG_LEVEL", "info"),
            secret_key: env_or("CRASHBOX_SECRET_KEY", "change-me-generate-random"),

            admin: AdminBootstrap {
                email: env_opt("CRASHBOX_ADMIN_EMAIL"),
                password: env_opt("CRASHBOX_ADMIN_PASSWORD"),
                name: env_opt("CRASHBOX_ADMIN_NAME"),
                force_reset: parse_env("CRASHBOX_FORCE_ADMIN_RESET", "false")?,
            },
            project: ProjectBootstrap {
                name: env_opt("CRASHBOX_PROJECT_NAME"),
                platform: env_opt("CRASHBOX_PROJECT_PLATFORM"),
                environment: env_opt("CRASHBOX_PROJECT_ENVIRONMENT"),
                public_key: env_opt("CRASHBOX_PROJECT_PUBLIC_KEY"),
                secret_key: env_opt("CRASHBOX_PROJECT_SECRET_KEY"),
            },
            ingest: IngestLimits {
                max_envelope_bytes: parse_env("CRASHBOX_MAX_ENVELOPE_BYTES", "1048576")?,
                max_event_bytes: parse_env("CRASHBOX_MAX_EVENT_BYTES", "524288")?,
                max_events_per_minute_per_project: parse_env(
                    "CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT",
                    "600",
                )?,
                accept_unknown_item_types: parse_env(
                    "CRASHBOX_ACCEPT_UNKNOWN_ITEM_TYPES",
                    "false",
                )?,
                store_raw_unsupported_items: parse_env(
                    "CRASHBOX_STORE_RAW_UNSUPPORTED_ITEMS",
                    "false",
                )?,
                enable_legacy_store_endpoint: parse_env(
                    "CRASHBOX_ENABLE_LEGACY_STORE_ENDPOINT",
                    "false",
                )?,
            },
            retention: Retention {
                retention_days: parse_env("CRASHBOX_RETENTION_DAYS", "30")?,
                max_events_per_issue: parse_env("CRASHBOX_MAX_EVENTS_PER_ISSUE", "100")?,
                cleanup_interval_seconds: parse_env("CRASHBOX_CLEANUP_INTERVAL_SECONDS", "3600")?,
                auto_resolve_days: parse_env("CRASHBOX_AUTO_RESOLVE_DAYS", "14")?,
            },
            spike: SpikeConfig {
                check_interval_seconds: parse_env("CRASHBOX_SPIKE_CHECK_INTERVAL_SECONDS", "300")?,
                min_events_per_hour: parse_env("CRASHBOX_SPIKE_MIN_EVENTS_PER_HOUR", "10")?,
                ratio_threshold: parse_env("CRASHBOX_SPIKE_RATIO_THRESHOLD", "5.0")?,
                cooldown_seconds: parse_env("CRASHBOX_SPIKE_COOLDOWN_SECONDS", "3600")?,
            },
            ui: UiConfig {
                enabled: parse_env("CRASHBOX_UI_ENABLED", "true")?,
                app_name: env_or("CRASHBOX_UI_APP_NAME", "Crashbox"),
                theme: env_or("CRASHBOX_UI_THEME", "system"),
            },
            security: SecurityConfig {
                cookie_secure: parse_env("CRASHBOX_COOKIE_SECURE", "false")?,
                cors_allowed_origins: env_or("CRASHBOX_CORS_ALLOWED_ORIGINS", "*"),
                trust_proxy_headers: parse_env("CRASHBOX_TRUST_PROXY_HEADERS", "false")?,
                allow_public_signup: parse_env("CRASHBOX_ALLOW_PUBLIC_SIGNUP", "false")?,
            },
            notify: NotifyConfig {
                telegram_bot_token: env_opt("CRASHBOX_TELEGRAM_BOT_TOKEN"),
                telegram_chat_id: env_opt("CRASHBOX_TELEGRAM_CHAT_ID"),
                discord_webhook_url: env_opt("CRASHBOX_DISCORD_WEBHOOK_URL"),
                generic_webhook_url: env_opt("CRASHBOX_GENERIC_WEBHOOK_URL"),
                max_per_minute: parse_env("CRASHBOX_NOTIFY_MAX_PER_MINUTE", "30")?,
            },
            livelog: LiveLogConfig {
                enabled: parse_env("CRASHBOX_LIVE_LOGS_ENABLED", "true")?,
                buffer_per_project: parse_env("CRASHBOX_LIVE_LOG_BUFFER_PER_PROJECT", "1000")?,
                max_batch_bytes: parse_env("CRASHBOX_MAX_LOG_BATCH_BYTES", "262144")?,
                message_max_bytes: parse_env("CRASHBOX_LIVE_LOG_MESSAGE_MAX_BYTES", "16384")?,
                max_per_minute_per_project: parse_env(
                    "CRASHBOX_MAX_LOGS_PER_MINUTE_PER_PROJECT",
                    "6000",
                )?,
                max_subscribers_per_project: parse_env(
                    "CRASHBOX_MAX_LOG_SUBSCRIBERS_PER_PROJECT",
                    "50",
                )?,
            },
        })
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn env_opt(key: &'static str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn env_or(key: &'static str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn parse_env<T>(key: &'static str, default: &str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let raw = env_or(key, default);
    raw.parse::<T>().map_err(|e| ConfigError::Invalid {
        var: key,
        source: anyhow::Error::new(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env mutations must be serialized across tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev: Vec<_> = vars.iter().map(|(k, _)| (*k, env::var(*k).ok())).collect();
        for (k, v) in vars {
            match v {
                Some(val) => env::set_var(k, val),
                None => env::remove_var(k),
            }
        }
        f();
        for (k, v) in prev {
            match v {
                Some(val) => env::set_var(k, val),
                None => env::remove_var(k),
            }
        }
    }

    #[test]
    fn defaults_load() {
        with_env(
            &[
                ("CRASHBOX_HOST", None),
                ("CRASHBOX_PORT", None),
                ("CRASHBOX_MAX_ENVELOPE_BYTES", None),
            ],
            || {
                let cfg = Config::from_env().expect("defaults must load");
                assert_eq!(cfg.host, "0.0.0.0");
                assert_eq!(cfg.port, 8080);
                assert_eq!(cfg.ingest.max_envelope_bytes, 1_048_576);
            },
        );
    }

    #[test]
    fn invalid_int_fails_loud() {
        with_env(&[("CRASHBOX_PORT", Some("not-a-number"))], || {
            let err = Config::from_env().expect_err("must reject non-int port");
            let msg = err.to_string();
            assert!(msg.contains("CRASHBOX_PORT"), "msg: {msg}");
        });
    }
}
