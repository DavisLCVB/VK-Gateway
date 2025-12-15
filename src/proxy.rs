use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use sqlx::PgPool;
use std::sync::Arc;

use crate::{
    db::Backend,
    health::HealthChecker,
    load_balancer::LoadBalancer,
};

#[derive(Clone)]
pub struct ProxyState {
    pub backends: Vec<Backend>,
    pub load_balancer: Arc<dyn LoadBalancer>,
    pub health_checker: Arc<HealthChecker>,
    pub client: Client<HttpConnector, Body>,
    pub db_pool: PgPool,
}

impl ProxyState {
    pub fn new(
        backends: Vec<Backend>,
        load_balancer: Arc<dyn LoadBalancer>,
        health_checker: Arc<HealthChecker>,
        db_pool: PgPool,
    ) -> Self {
        let client = Client::builder(TokioExecutor::new()).build_http();

        Self {
            backends,
            load_balancer,
            health_checker,
            client,
            db_pool,
        }
    }
}

/// Select a backend using the load balancer
async fn select_backend_via_load_balancer(state: &ProxyState) -> Result<Backend, StatusCode> {
    let healthy_backends = state.health_checker.get_healthy_backends(&state.backends).await;

    if healthy_backends.is_empty() {
        tracing::error!("No healthy backends available");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    match state.load_balancer.select_backend(&healthy_backends).await {
        Some(b) => Ok(b),
        None => {
            tracing::error!("Load balancer failed to select a backend");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// Extract file ID from common URL patterns
/// Supports patterns like:
/// - /api/v1/files/{id}
/// - /api/v1/files/download/{id}
/// - /files/{id}
/// - /download/{id}
fn extract_file_id_from_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    // Pattern: /api/v1/files/{id} or /api/v1/files/download/{id}
    if segments.len() >= 4 && segments[0] == "api" && segments[2] == "files" {
        if segments.len() >= 5 && segments[3] == "download" {
            return Some(segments[4].to_string());
        } else if segments.len() == 4 {
            return Some(segments[3].to_string());
        }
    }

    // Pattern: /files/{id} or /files/download/{id}
    if segments.len() >= 2 && segments[0] == "files" {
        if segments.len() >= 3 && segments[1] == "download" {
            return Some(segments[2].to_string());
        } else if segments.len() == 2 {
            return Some(segments[1].to_string());
        }
    }

    // Pattern: /download/{id}
    if segments.len() == 2 && segments[0] == "download" {
        return Some(segments[1].to_string());
    }

    None
}

/// Handler principal del proxy que reenvía todas las peticiones
pub async fn proxy_handler(
    State(state): State<ProxyState>,
    mut req: Request,
) -> Result<Response, StatusCode> {
    let path = req.uri().path();

    // Try to extract file ID from path and route to specific backend
    let backend = if let Some(file_id) = extract_file_id_from_path(path) {
        tracing::debug!("Detected file request for ID: {}", file_id);

        // Query database for the backend that owns this file
        match crate::db::get_file_backend(&state.db_pool, &file_id).await {
            Ok(Some(server_id)) => {
                tracing::info!("File {} is owned by backend {}", file_id, server_id);

                // Find the backend by server_id
                match state.backends.iter().find(|b| b.server_id == server_id) {
                    Some(backend) => {
                        // Check if backend is healthy
                        if !state.health_checker.is_backend_healthy(&server_id).await {
                            tracing::warn!("Backend {} for file {} is not healthy", server_id, file_id);
                            return Err(StatusCode::SERVICE_UNAVAILABLE);
                        }
                        backend.clone()
                    }
                    None => {
                        tracing::error!("Backend {} not found in configuration", server_id);
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                }
            }
            Ok(None) => {
                tracing::warn!("File {} not found in metadata, using load balancer", file_id);
                // Fall back to load balancing if file not found in metadata
                select_backend_via_load_balancer(&state).await?
            }
            Err(e) => {
                tracing::error!("Database error looking up file {}: {}", file_id, e);
                // Fall back to load balancing on database error
                select_backend_via_load_balancer(&state).await?
            }
        }
    } else {
        // Not a file request, use load balancer
        select_backend_via_load_balancer(&state).await?
    };

    tracing::info!(
        "Proxying {} {} to backend {} ({})",
        req.method(),
        req.uri(),
        backend.server_id,
        backend.server_url
    );

    // Construye la URL del backend
    let path_and_query = req.uri().path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let backend_url = format!("{}{}", backend.server_url.trim_end_matches('/'), path_and_query);

    // Parsea la nueva URI
    let uri = match backend_url.parse::<Uri>() {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!("Failed to parse backend URL {}: {}", backend_url, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Actualiza la URI de la petición
    *req.uri_mut() = uri.clone();

    // Actualiza el header Host
    if let Some(host) = uri.host() {
        let host_header = if let Some(port) = uri.port_u16() {
            format!("{}:{}", host, port)
        } else {
            host.to_string()
        };

        if let Ok(header_value) = HeaderValue::from_str(&host_header) {
            req.headers_mut().insert("host", header_value);
        }
    }

    // Reenvía la petición al backend
    let response = match state.client.request(req).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to proxy request to backend {}: {}", backend.server_id, e);
            state.load_balancer.release_backend(&backend).await;
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // Libera el backend en el load balancer
    state.load_balancer.release_backend(&backend).await;

    // Convierte la respuesta de hyper a axum
    let (parts, body) = response.into_parts();
    let body = Body::new(body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)).boxed());

    Ok(Response::from_parts(parts, body))
}

/// Handler para peticiones específicas a un backend por ID
pub async fn proxy_to_specific_backend(
    State(state): State<ProxyState>,
    Path(server_id): Path<String>,
    mut req: Request,
) -> Result<Response, StatusCode> {
    // Busca el backend específico
    let backend = match state.backends.iter().find(|b| b.server_id == server_id) {
        Some(b) => b,
        None => {
            tracing::warn!("Backend {} not found", server_id);
            return Err(StatusCode::NOT_FOUND);
        }
    };

    // Verifica si el backend está saludable
    if !state.health_checker.is_backend_healthy(&server_id).await {
        tracing::warn!("Backend {} is not healthy", server_id);
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    tracing::info!(
        "Proxying {} to specific backend {} ({})",
        req.uri(),
        backend.server_id,
        backend.server_url
    );

    // Construye la URL del backend
    let path = req.uri().path();

    // Remueve el prefijo /backend/{server_id} de la ruta
    let backend_path = path.strip_prefix(&format!("/backend/{}", server_id))
        .unwrap_or(path);

    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let backend_url = format!("{}{}{}", backend.server_url.trim_end_matches('/'), backend_path, query);

    // Parsea la nueva URI
    let uri = match backend_url.parse::<Uri>() {
        Ok(uri) => uri,
        Err(e) => {
            tracing::error!("Failed to parse backend URL {}: {}", backend_url, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Actualiza la URI de la petición
    *req.uri_mut() = uri.clone();

    // Actualiza el header Host
    if let Some(host) = uri.host() {
        let host_header = if let Some(port) = uri.port_u16() {
            format!("{}:{}", host, port)
        } else {
            host.to_string()
        };

        if let Ok(header_value) = HeaderValue::from_str(&host_header) {
            req.headers_mut().insert("host", header_value);
        }
    }

    // Reenvía la petición al backend
    let response = match state.client.request(req).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to proxy request to backend {}: {}", backend.server_id, e);
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // Convierte la respuesta de hyper a axum
    let (parts, body) = response.into_parts();
    let body = Body::new(body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)).boxed());

    Ok(Response::from_parts(parts, body))
}

/// Handler de health check del gateway mismo
pub async fn gateway_health() -> impl IntoResponse {
    (StatusCode::OK, "Gateway is healthy")
}

/// Handler para obtener estadísticas del gateway
pub async fn gateway_stats(State(state): State<ProxyState>) -> impl IntoResponse {
    let health_status = state.health_checker.get_all_health_status().await;

    let stats = serde_json::json!({
        "load_balancer": state.load_balancer.name(),
        "total_backends": state.backends.len(),
        "healthy_backends": health_status.values().filter(|s| s.is_healthy).count(),
        "backends": state.backends.iter().map(|b| {
            let status = health_status.get(&b.server_id);
            serde_json::json!({
                "server_id": b.server_id,
                "server_name": b.server_name,
                "server_url": b.server_url,
                "provider": b.provider,
                "is_healthy": status.map(|s| s.is_healthy).unwrap_or(true),
                "consecutive_failures": status.map(|s| s.consecutive_failures).unwrap_or(0),
            })
        }).collect::<Vec<_>>(),
    });

    (StatusCode::OK, axum::Json(stats))
}
