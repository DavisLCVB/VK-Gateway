use sqlx::{PgPool, postgres::PgPoolOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Backend {
    pub server_id: String,
    pub provider: String,
    pub server_name: String,
    pub server_url: String,
}

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}

pub async fn get_all_backends(pool: &PgPool) -> Result<Vec<Backend>, sqlx::Error> {
    sqlx::query_as::<_, Backend>(
        "SELECT server_id, provider, server_name, server_url FROM config.local"
    )
    .fetch_all(pool)
    .await
}

/// Get a specific backend by ID from the database
/// Available for direct backend lookups when needed
#[allow(dead_code)]
pub async fn get_backend_by_id(pool: &PgPool, server_id: &str) -> Result<Option<Backend>, sqlx::Error> {
    sqlx::query_as::<_, Backend>(
        "SELECT server_id, provider, server_name, server_url FROM config.local WHERE server_id = $1"
    )
    .bind(server_id)
    .fetch_optional(pool)
    .await
}

/// Get the server_id for a file from metadata table
/// Returns the backend server_id that owns this file
pub async fn get_file_backend(pool: &PgPool, file_id: &str) -> Result<Option<String>, sqlx::Error> {
    let result = sqlx::query_scalar::<_, String>(
        "SELECT server_id FROM application.metadata WHERE file_id = $1"
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;

    Ok(result)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ExpiredFile {
    pub file_id: String,
    pub server_id: String,
}

/// Get all files that have expired (delete_at <= NOW())
pub async fn get_expired_files(pool: &PgPool) -> Result<Vec<ExpiredFile>, sqlx::Error> {
    sqlx::query_as::<_, ExpiredFile>(
        "SELECT file_id, server_id FROM application.metadata
         WHERE delete_at IS NOT NULL AND delete_at <= NOW()"
    )
    .fetch_all(pool)
    .await
}

/// Delete a file from metadata table
pub async fn delete_file_metadata(pool: &PgPool, file_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM application.metadata WHERE file_id = $1")
        .bind(file_id)
        .execute(pool)
        .await?;

    Ok(())
}
