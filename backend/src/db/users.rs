use chrono::Utc;
use sqlx::SqlitePool;

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub name: Option<String>,
    pub password_hash: String,
    pub is_admin: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn count(pool: &SqlitePool) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
}

pub async fn find_by_email(pool: &SqlitePool, email: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as::<_, User>(
        "SELECT id, email, name, password_hash, is_admin, created_at, updated_at \
         FROM users WHERE email = ?",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn insert_admin(
    pool: &SqlitePool,
    email: &str,
    name: Option<&str>,
    password_hash: &str,
) -> sqlx::Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO users (email, name, password_hash, is_admin, created_at, updated_at) \
         VALUES (?, ?, ?, 1, ?, ?)",
    )
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(row.last_insert_rowid())
}

pub async fn update_password(
    pool: &SqlitePool,
    user_id: i64,
    password_hash: &str,
) -> sqlx::Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(password_hash)
        .bind(&now)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
