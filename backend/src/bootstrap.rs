use sqlx::SqlitePool;
use ulid::Ulid;

use crate::config::{AdminBootstrap, Config, ProjectBootstrap};
use crate::db::{projects, users};
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
    Ok(())
}

async fn bootstrap_admin(pool: &SqlitePool, admin: &AdminBootstrap) -> anyhow::Result<()> {
    let Some(email) = admin.email.as_deref() else {
        if users::count(pool).await? == 0 {
            tracing::warn!(
                "bootstrap: no admin user exists and CRASHBOX_ADMIN_EMAIL is not set"
            );
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
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
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
