mod cache;
mod config;
mod db;
mod health;
mod load_balancer;
mod proxy;

use anyhow::Result;
use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::Config,
    health::HealthChecker,
    load_balancer::create_load_balancer,
    proxy::{
        delete_expired_files, gateway_health, gateway_stats, proxy_handler,
        proxy_to_specific_backend, ProxyState,
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    // Inicializa el sistema de logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vk_gateway=debug,tower_http=debug,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting VK Gateway...");

    // Carga la configuración
    let config = Config::from_env()?;
    tracing::info!("Configuration loaded");

    // Conecta a PostgreSQL
    let db_pool = db::create_pool(&config.database_url).await?;
    tracing::info!("Connected to PostgreSQL");

    // Conecta a Redis
    let _redis_client = cache::create_redis_client(&config.redis_url).await?;
    tracing::info!("Connected to Redis");

    // Obtiene la lista de backends desde la base de datos
    let backends = db::get_all_backends(&db_pool).await?;
    tracing::info!("Loaded {} backends from database", backends.len());

    if backends.is_empty() {
        tracing::warn!(
            "No backends found in database. The gateway will not be able to proxy requests."
        );
    }

    for backend in &backends {
        tracing::info!(
            "  - {} ({}) - {} [{}]",
            backend.server_name,
            backend.server_id,
            backend.server_url,
            backend.provider
        );
    }

    // Crea el load balancer
    // Puedes cambiar la estrategia aquí: "round-robin", "least-connections", "random", "weighted-round-robin"
    let load_balancer_strategy =
        std::env::var("LOAD_BALANCER_STRATEGY").unwrap_or_else(|_| "round-robin".to_string());

    let load_balancer = create_load_balancer(&load_balancer_strategy);
    tracing::info!("Using load balancer: {}", load_balancer.name());

    // Crea el health checker
    let health_checker = Arc::new(HealthChecker::new(config.vk_secret.clone()));

    // Inicia los health checks periódicos (cada 30 segundos)
    let health_check_interval = std::env::var("HEALTH_CHECK_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    health_checker
        .clone()
        .start_health_checks(backends.clone(), health_check_interval)
        .await;
    tracing::info!(
        "Health checker started (interval: {}s)",
        health_check_interval
    );

    // Crea el estado del proxy
    let proxy_state = ProxyState::new(
        backends,
        load_balancer,
        health_checker,
        db_pool.clone(),
        config.vk_secret.clone(),
    );

    // Configura las rutas de Axum
    let app = Router::new()
        // Rutas del gateway
        .route("/api/v1/health", get(gateway_health))
        .route("/api/v1/stats", get(gateway_stats))
        .route(
            "/api/v1/files/delete-expired",
            axum::routing::delete(delete_expired_files),
        )
        // Ruta para acceder a un backend específico por ID
        .route(
            "/api/v1/backend/:server_id/*path",
            get(proxy_to_specific_backend)
                .post(proxy_to_specific_backend)
                .put(proxy_to_specific_backend)
                .patch(proxy_to_specific_backend)
                .delete(proxy_to_specific_backend),
        )
        // Ruta catch-all para proxy transparente
        .fallback(proxy_handler)
        .with_state(proxy_state)
        // Middlewares
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // Inicia el servidor
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("VK Gateway listening on {}", addr);
    tracing::info!("Gateway endpoints:");
    tracing::info!("  - GET  /health                        - Gateway health check");
    tracing::info!("  - GET  /stats                         - Gateway statistics");
    tracing::info!("  - POST /api/v1/files/delete-expired   - Delete expired files");
    tracing::info!("  - *    /backend/:id/*                 - Proxy to specific backend");
    tracing::info!("  - *    /*                             - Proxy to load-balanced backend");

    axum::serve(listener, app).await?;

    Ok(())
}
