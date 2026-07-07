pub mod assets;
pub mod auth;
pub mod dsn_auth;
pub mod error;
pub mod health;
pub mod heartbeats;
pub mod ingest;
pub mod issues;
pub mod livelog;
pub mod projects;
pub mod routes;
pub mod tokens;

/// Standard list-endpoint envelope: the requested page plus the total match count, so API
/// consumers (UI and agents alike) can paginate without probing for an empty page.
#[derive(Debug, serde::Serialize)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub total: i64,
}
