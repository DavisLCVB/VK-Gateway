use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;

/// Rate limiter configuration
#[derive(Clone, Copy)]
pub struct RateLimiterConfig {
    pub max_requests: u32,
    pub window_secs: u64,
    pub block_duration_secs: u64,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            max_requests: 10,        // Max 10 requests
            window_secs: 60,         // Per 60 seconds
            block_duration_secs: 300, // Block for 5 minutes
        }
    }
}

/// Check if a token is rate limited using Redis
pub async fn check_rate_limit(
    redis_client: &mut redis::aio::ConnectionManager,
    token: &str,
    config: &RateLimiterConfig,
) -> Result<bool, redis::RedisError> {
    let conn = redis_client;

    // Check if token is blocked
    let block_key = format!("rate_limit:blocked:{}", token);
    let is_blocked: bool = conn.exists(&block_key).await?;

    if is_blocked {
        tracing::warn!("Token {} is blocked", token);
        return Ok(false);
    }

    // Increment request count
    let count_key = format!("rate_limit:count:{}", token);
    let count: u32 = conn.incr(&count_key, 1).await?;

    // Set expiration on first request
    if count == 1 {
        let _: () = conn.expire(&count_key, config.window_secs as i64).await?;
    }

    // Check if limit exceeded
    if count > config.max_requests {
        tracing::warn!(
            "Token {} exceeded rate limit: {} requests in {} seconds",
            token,
            count,
            config.window_secs
        );

        // Block the token
        let _: () = conn.set_ex(
            &block_key,
            "blocked",
            config.block_duration_secs as u64,
        )
        .await?;

        // Delete the counter
        let _: () = conn.del(&count_key).await?;

        return Ok(false);
    }

    tracing::debug!("Token {} request count: {}/{}", token, count, config.max_requests);
    Ok(true)
}

/// Extract upload token from Authorization or X-Upload-Token headers
fn extract_upload_token(req: &Request) -> Option<String> {
    // Try Authorization: Bearer <token> first
    if let Some(auth_header) = req.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                return Some(token.to_string());
            }
        }
    }

    // Fallback to X-Upload-Token header
    if let Some(token_header) = req.headers().get("x-upload-token") {
        if let Ok(token) = token_header.to_str() {
            return Some(token.to_string());
        }
    }

    None
}

/// Middleware to rate limit requests based on upload token
/// Supports both Authorization: Bearer <token> and X-Upload-Token headers
pub async fn rate_limit_middleware(
    mut redis_client: redis::aio::ConnectionManager,
    config: RateLimiterConfig,
    req: Request,
    next: Next,
) -> Response {
    // Extract upload token from headers
    let token = match extract_upload_token(&req) {
        Some(t) => t,
        None => {
            // No token header, allow request to proceed
            return next.run(req).await;
        }
    };

    // Check rate limit
    match check_rate_limit(&mut redis_client, &token, &config).await {
        Ok(true) => {
            // Rate limit OK, proceed
            next.run(req).await
        }
        Ok(false) => {
            // Rate limit exceeded
            tracing::warn!("Rate limit exceeded for token: {}", token);
            (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded. Token is temporarily blocked.",
            )
                .into_response()
        }
        Err(e) => {
            // Redis error, log but allow request to proceed
            tracing::error!("Redis error in rate limiter: {}", e);
            next.run(req).await
        }
    }
}

/// Get rate limit info for a token
pub async fn get_rate_limit_info(
    redis_client: &mut redis::aio::ConnectionManager,
    token: &str,
) -> Result<RateLimitInfo, redis::RedisError> {
    let conn = redis_client;

    let block_key = format!("rate_limit:blocked:{}", token);
    let count_key = format!("rate_limit:count:{}", token);

    let is_blocked: bool = conn.exists(&block_key).await?;
    let request_count: Option<u32> = conn.get(&count_key).await?;
    let ttl: i64 = if is_blocked {
        conn.ttl(&block_key).await?
    } else if request_count.is_some() {
        conn.ttl(&count_key).await?
    } else {
        -1
    };

    Ok(RateLimitInfo {
        is_blocked,
        request_count: request_count.unwrap_or(0),
        ttl_seconds: if ttl > 0 { Some(ttl as u64) } else { None },
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RateLimitInfo {
    pub is_blocked: bool,
    pub request_count: u32,
    pub ttl_seconds: Option<u64>,
}

/// Clear rate limit for a token (admin function)
pub async fn clear_rate_limit(
    redis_client: &mut redis::aio::ConnectionManager,
    token: &str,
) -> Result<(), redis::RedisError> {
    let conn = redis_client;

    let block_key = format!("rate_limit:blocked:{}", token);
    let count_key = format!("rate_limit:count:{}", token);

    let _: () = conn.del(&[&block_key, &count_key]).await?;

    tracing::info!("Cleared rate limit for token: {}", token);
    Ok(())
}
