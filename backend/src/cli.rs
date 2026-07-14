//! CLI subcommands. The same binary can run as the HTTP server (default) or as an admin
//! tool over the local SQLite database.
//!
//! Output format: ANSI tables when stdout is a TTY, tab-separated otherwise — `crashbox
//! issues list | awk` works without ceremony.

use std::io::{self, IsTerminal, Write};

use clap::{Parser, Subcommand};
use ulid::Ulid;

use crate::config::Config;
use crate::db;
use crate::security::password;
use crate::sentry::dsn::{mask_public_key, Dsn};

#[derive(Parser, Debug)]
#[command(version, about = "Crashbox — tiny self-hosted Sentry-compatible tracker", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Start the HTTP server (default if no subcommand is given).
    Serve,
    /// Work with issues.
    Issues {
        #[command(subcommand)]
        action: IssuesAction,
    },
    /// Work with projects.
    Projects {
        #[command(subcommand)]
        action: ProjectsAction,
    },
    /// Work with users.
    Users {
        #[command(subcommand)]
        action: UsersAction,
    },
    /// Snapshot the SQLite database to a file (atomic via VACUUM INTO).
    Backup {
        /// Destination path. Will be created; refuses to overwrite existing files.
        path: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum IssuesAction {
    /// List issues, newest-seen first.
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "unresolved")]
        status: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Mark an issue as resolved.
    Resolve { id: i64 },
    /// Mark an issue as unresolved.
    Unresolve { id: i64 },
}

#[derive(Subcommand, Debug)]
pub enum ProjectsAction {
    /// List projects.
    List,
    /// Generate a new public_key (invalidates the current DSN).
    RotateKey { id: i64 },
}

#[derive(Subcommand, Debug)]
pub enum UsersAction {
    /// Reset a user's password. Prompts interactively for the new value, or with `--stdin`
    /// reads a single line from stdin (use in pipelines / docker exec -i).
    ResetPassword {
        email: String,
        /// Read the new password from stdin (single line, no echo control). Useful for
        /// non-TTY contexts like `docker exec -i` or CI scripts.
        #[arg(long)]
        stdin: bool,
    },
}

/// Returns whether the CLI handled the request and the process should exit.
pub async fn run_if_present(cli: Cli, cfg: &Config) -> anyhow::Result<bool> {
    let Some(cmd) = cli.cmd else {
        return Ok(false);
    };
    match cmd {
        Cmd::Serve => Ok(false),
        Cmd::Issues { action } => {
            let pool = db::connect(&cfg.database_url).await?;
            run_issues(action, &pool, cfg).await?;
            Ok(true)
        }
        Cmd::Projects { action } => {
            let pool = db::connect(&cfg.database_url).await?;
            run_projects(action, &pool, cfg).await?;
            Ok(true)
        }
        Cmd::Users { action } => {
            let pool = db::connect(&cfg.database_url).await?;
            run_users(action, &pool).await?;
            Ok(true)
        }
        Cmd::Backup { path } => {
            let pool = db::connect(&cfg.database_url).await?;
            run_backup(&path, &pool).await?;
            Ok(true)
        }
    }
}

async fn run_issues(
    action: IssuesAction,
    pool: &sqlx::SqlitePool,
    _cfg: &Config,
) -> anyhow::Result<()> {
    match action {
        IssuesAction::List {
            project,
            status,
            limit,
        } => {
            #[derive(sqlx::FromRow)]
            struct Row {
                id: i64,
                project_slug: String,
                title: String,
                status: String,
                event_count: i64,
                last_seen: String,
            }
            // Resolve --project=slug to project_id (Option) so users don't have to remember ids.
            let project_id: Option<i64> = if let Some(slug) = project.as_deref() {
                let id: Option<i64> = sqlx::query_scalar("SELECT id FROM projects WHERE slug = ?")
                    .bind(slug)
                    .fetch_optional(pool)
                    .await?;
                if id.is_none() {
                    anyhow::bail!("no project with slug={slug:?}");
                }
                id
            } else {
                None
            };
            let mut qb = sqlx::QueryBuilder::new(
                "SELECT i.id, p.slug AS project_slug, i.title, i.status, \
                        i.event_count, i.last_seen \
                 FROM issues i JOIN projects p ON p.id = i.project_id ",
            );
            qb.push("WHERE 1=1 ");
            if let Some(id) = project_id {
                qb.push(" AND i.project_id = ");
                qb.push_bind(id);
            }
            if status != "all" {
                qb.push(" AND i.status = ");
                qb.push_bind(status);
            }
            qb.push(" ORDER BY i.last_seen DESC LIMIT ");
            qb.push_bind(limit.clamp(1, 500));
            let rows: Vec<Row> = qb.build_query_as().fetch_all(pool).await?;

            let headers = ["id", "project", "status", "count", "last_seen", "title"];
            let body: Vec<Vec<String>> = rows
                .into_iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.project_slug,
                        r.status,
                        r.event_count.to_string(),
                        r.last_seen,
                        r.title,
                    ]
                })
                .collect();
            print_table(&headers, &body);
        }
        IssuesAction::Resolve { id } => {
            let n = crate::db::issues::set_status(pool, id, "resolved").await?;
            if n == 0 {
                anyhow::bail!("no issue with id={id}");
            }
            eprintln!("resolved #{id}");
        }
        IssuesAction::Unresolve { id } => {
            let n = crate::db::issues::set_status(pool, id, "unresolved").await?;
            if n == 0 {
                anyhow::bail!("no issue with id={id}");
            }
            eprintln!("unresolved #{id}");
        }
    }
    Ok(())
}

async fn run_projects(
    action: ProjectsAction,
    pool: &sqlx::SqlitePool,
    cfg: &Config,
) -> anyhow::Result<()> {
    match action {
        ProjectsAction::List => {
            #[derive(sqlx::FromRow)]
            struct Row {
                id: i64,
                name: String,
                slug: String,
                platform: Option<String>,
                public_key: String,
                event_count: i64,
            }
            let rows: Vec<Row> = sqlx::query_as(
                "SELECT p.id, p.name, p.slug, p.platform, p.public_key, \
                        COALESCE(SUM(i.event_count), 0) AS event_count \
                 FROM projects p \
                 LEFT JOIN issues i ON i.project_id = p.id \
                 GROUP BY p.id ORDER BY p.id ASC",
            )
            .fetch_all(pool)
            .await?;
            let headers = ["id", "slug", "name", "platform", "public_key", "events"];
            let body: Vec<Vec<String>> = rows
                .into_iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.slug,
                        r.name,
                        r.platform.unwrap_or_default(),
                        mask_public_key(&r.public_key),
                        r.event_count.to_string(),
                    ]
                })
                .collect();
            print_table(&headers, &body);
        }
        ProjectsAction::RotateKey { id } => {
            let new_key = Ulid::new().to_string().to_lowercase();
            let n = crate::db::projects::rotate_key(pool, id, &new_key).await?;
            if n == 0 {
                anyhow::bail!("no project with id={id}");
            }
            let dsn = Dsn::build(&cfg.public_url, &new_key, id);
            eprintln!("rotated. new dsn:");
            println!("{dsn}");
        }
    }
    Ok(())
}

async fn run_users(action: UsersAction, pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    match action {
        UsersAction::ResetPassword { email, stdin } => {
            let user = crate::db::users::find_by_email(pool, &email).await?;
            let Some(user) = user else {
                anyhow::bail!("no user with email={email:?}");
            };

            let new_pw = if stdin {
                use std::io::BufRead;
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                line.trim_end_matches(['\n', '\r']).to_string()
            } else {
                let pw = rpassword::prompt_password("new password: ")?;
                let confirm = rpassword::prompt_password("confirm password: ")?;
                if pw != confirm {
                    anyhow::bail!("passwords do not match");
                }
                pw
            };
            if new_pw.is_empty() {
                anyhow::bail!("password must not be empty");
            }
            let hash = password::hash_password(&new_pw)?;
            crate::db::users::update_password(pool, user.id, &hash).await?;
            eprintln!("password updated for {email}");
        }
    }
    Ok(())
}

async fn run_backup(path: &str, pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    db::vacuum_into(pool, std::path::Path::new(path)).await?;
    eprintln!("backup written to {path}");
    Ok(())
}

// ─── output helpers ────────────────────────────────────────────────────────

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if stdout.is_terminal() {
        print_pretty(&mut out, headers, rows).ok();
    } else {
        print_tsv(&mut out, headers, rows).ok();
    }
}

fn print_tsv<W: Write>(out: &mut W, headers: &[&str], rows: &[Vec<String>]) -> io::Result<()> {
    writeln!(out, "{}", headers.join("\t"))?;
    for row in rows {
        writeln!(out, "{}", row.join("\t"))?;
    }
    Ok(())
}

fn print_pretty<W: Write>(out: &mut W, headers: &[&str], rows: &[Vec<String>]) -> io::Result<()> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len().min(80);
            }
        }
    }
    let sep = |w: &mut W| -> io::Result<()> {
        for (i, width) in widths.iter().enumerate() {
            write!(w, "{}", "─".repeat(*width))?;
            if i < widths.len() - 1 {
                write!(w, "─┼─")?;
            }
        }
        writeln!(w)
    };

    // Header (bold via ANSI when TTY)
    for (i, h) in headers.iter().enumerate() {
        write!(out, "\x1b[1m{h:width$}\x1b[0m", width = widths[i])?;
        if i < headers.len() - 1 {
            write!(out, " │ ")?;
        }
    }
    writeln!(out)?;
    sep(out)?;
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let truncated = if cell.len() > 80 {
                format!("{}…", &cell[..79])
            } else {
                cell.clone()
            };
            write!(out, "{:width$}", truncated, width = widths[i])?;
            if i < row.len() - 1 {
                write!(out, " │ ")?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}
