use sqlx::SqlitePool;
use ulid::Ulid;

use crate::config::{AdminBootstrap, Config, HeartbeatMonitorSpec, ProjectBootstrap};
use crate::db::{heartbeats, projects, users};
use crate::security::password;
use crate::sentry::dsn::{mask_public_key, Dsn};

pub async fn run(pool: &SqlitePool, cfg: &Config) -> anyhow::Result<()> {
    bootstrap_admin(pool, &cfg.admin).await?;
    let project_for_dsn = bootstrap_project(pool, &cfg.project).await?;

    if let Some((project_id, public_key)) = project_for_dsn {
        let full = Dsn::build(&cfg.public_url, &public_key, project_id);
        tracing::info!(
            project_id,
            public_key_masked = %mask_public_key(&public_key),
            dsn = %full,
            "bootstrap: project DSN (shown once at startup)"
        );
    }
    bootstrap_heartbeat_monitors(pool, &cfg.heartbeat.monitors).await?;
    Ok(())
}

/// Apply `CRASHBOX_HEARTBEAT_MONITORS` idempotently. `name` is the identity within the
/// default project (lowest id — env provisioning targets single-project deployments).
/// For declared monitors the env is the source of truth: `ping_key` and `period_seconds`
/// always converge to the declared values; `grace_seconds` / `description` only when
/// declared. Monitors not listed in the env are never touched or deleted, and status /
/// transition history is never altered.
async fn bootstrap_heartbeat_monitors(
    pool: &SqlitePool,
    specs: &[HeartbeatMonitorSpec],
) -> anyhow::Result<()> {
    if specs.is_empty() {
        return Ok(());
    }
    let project_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM projects ORDER BY id ASC LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let Some(project_id) = project_id else {
        tracing::warn!(
            "bootstrap: CRASHBOX_HEARTBEAT_MONITORS is set but no project exists; skipping"
        );
        return Ok(());
    };

    for spec in specs {
        // A ping key already attached to a *different* monitor is a config error: keys are
        // both identity and the sole authentication, so silently stealing one would break
        // whatever currently pings it.
        if let Some(owner) = heartbeats::find_by_ping_key(pool, &spec.ping_key).await? {
            if owner.project_id != project_id || owner.name != spec.name {
                anyhow::bail!(
                    "bootstrap: ping_key declared for monitor {:?} is already used by monitor {:?}",
                    spec.name,
                    owner.name
                );
            }
        }

        let grace = spec
            .grace_seconds
            .unwrap_or(heartbeats::GRACE_DEFAULT_SECONDS);
        match heartbeats::find_by_name(pool, project_id, &spec.name).await? {
            None => {
                let id = heartbeats::insert(
                    pool,
                    project_id,
                    &spec.name,
                    spec.description.as_deref(),
                    &spec.ping_key,
                    spec.period_seconds,
                    grace,
                )
                .await?;
                tracing::info!(
                    monitor_id = id,
                    name = %spec.name,
                    "bootstrap: heartbeat monitor created from env"
                );
            }
            Some(existing) => {
                if existing.ping_key != spec.ping_key {
                    heartbeats::set_ping_key(pool, existing.id, &spec.ping_key).await?;
                    tracing::info!(
                        monitor_id = existing.id,
                        name = %spec.name,
                        "bootstrap: heartbeat monitor ping_key updated from env"
                    );
                }
                let period_drifted = existing.period_seconds != spec.period_seconds;
                let grace_drifted = spec.grace_seconds.is_some() && existing.grace_seconds != grace;
                let description_drifted = spec.description.is_some()
                    && existing.description.as_deref() != spec.description.as_deref();
                if period_drifted || grace_drifted || description_drifted {
                    heartbeats::update(
                        pool,
                        existing.id,
                        None,
                        spec.description
                            .as_deref()
                            .map(Some)
                            .filter(|_| description_drifted),
                        Some(spec.period_seconds).filter(|_| period_drifted),
                        Some(grace).filter(|_| grace_drifted),
                    )
                    .await?;
                    tracing::info!(
                        monitor_id = existing.id,
                        name = %spec.name,
                        "bootstrap: heartbeat monitor settings converged to env"
                    );
                }
            }
        }
    }
    Ok(())
}

async fn bootstrap_admin(pool: &SqlitePool, admin: &AdminBootstrap) -> anyhow::Result<()> {
    let Some(email) = admin.email.as_deref() else {
        if users::count(pool).await? == 0 {
            tracing::warn!("bootstrap: no admin user exists and CRASHBOX_ADMIN_EMAIL is not set");
        }
        return Ok(());
    };
    let Some(password_plain) = admin.password.as_deref() else {
        tracing::warn!("bootstrap: CRASHBOX_ADMIN_EMAIL set but CRASHBOX_ADMIN_PASSWORD missing");
        return Ok(());
    };

    let existing = users::find_by_email(pool, email).await?;
    match existing {
        None => {
            let hash = password::hash_password(password_plain)?;
            let id = users::insert_admin(pool, email, admin.name.as_deref(), &hash).await?;
            tracing::info!(user_id = id, email = %email, "bootstrap: admin user created");
        }
        Some(user) if admin.force_reset => {
            let hash = password::hash_password(password_plain)?;
            users::update_password(pool, user.id, &hash).await?;
            tracing::warn!(
                user_id = user.id,
                email = %email,
                "bootstrap: admin password RESET via CRASHBOX_FORCE_ADMIN_RESET"
            );
        }
        Some(_) => {
            tracing::debug!(email = %email, "bootstrap: admin user already exists, skip");
        }
    }
    Ok(())
}

/// Returns Some((project_id, public_key)) if a project was newly created and a DSN should be
/// logged for it; None otherwise.
async fn bootstrap_project(
    pool: &SqlitePool,
    cfg: &ProjectBootstrap,
) -> anyhow::Result<Option<(i64, String)>> {
    if projects::count(pool).await? > 0 {
        tracing::debug!("bootstrap: project already exists, skip");
        return Ok(None);
    }
    let Some(name) = cfg.name.as_deref() else {
        tracing::warn!(
            "bootstrap: no project exists and CRASHBOX_PROJECT_NAME is not set; \
             create one via the UI or set the env var"
        );
        return Ok(None);
    };

    let public_key = cfg
        .public_key
        .clone()
        .unwrap_or_else(|| Ulid::new().to_string().to_lowercase());
    let secret_key_hash = match cfg.secret_key.as_deref() {
        Some(secret) => Some(password::hash_password(secret)?),
        None => None,
    };
    let slug = slugify(name);

    let id = projects::insert(
        pool,
        name,
        &slug,
        cfg.platform.as_deref(),
        cfg.environment.as_deref(),
        &public_key,
        secret_key_hash.as_deref(),
    )
    .await?;
    tracing::info!(project_id = id, name = %name, slug = %slug, "bootstrap: default project created");
    Ok(Some((id, public_key)))
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(slugify("My App"), "my-app");
        assert_eq!(slugify("  Hello / World!  "), "hello-world");
        assert_eq!(slugify("!!!"), "project");
    }
}
