use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use std::time::Duration;

pub async fn create_redis_client(redis_url: &str) -> Result<ConnectionManager, redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    ConnectionManager::new(client).await
}

/// Cache functions for future use - currently unused but kept for planned features
#[allow(dead_code)]
pub async fn cache_set(
    conn: &mut ConnectionManager,
    key: &str,
    value: &str,
    ttl: Duration,
) -> Result<(), redis::RedisError> {
    conn.set_ex(key, value, ttl.as_secs() as u64).await
}

#[allow(dead_code)]
pub async fn cache_get(
    conn: &mut ConnectionManager,
    key: &str,
) -> Result<Option<String>, redis::RedisError> {
    conn.get(key).await
}

#[allow(dead_code)]
pub async fn cache_delete(
    conn: &mut ConnectionManager,
    key: &str,
) -> Result<(), redis::RedisError> {
    conn.del(key).await
}
