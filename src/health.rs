use crate::db::Backend;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub is_healthy: bool,
    pub last_check: std::time::Instant,
    pub consecutive_failures: usize,
}

/// Servicio que monitorea la salud de los backends
pub struct HealthChecker {
    client: Client,
    health_status: Arc<RwLock<HashMap<String, HealthStatus>>>,
    vk_secret: Option<String>,
}

impl HealthChecker {
    pub fn new(vk_secret: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            health_status: Arc::new(RwLock::new(HashMap::new())),
            vk_secret,
        }
    }

    /// Inicia el chequeo periódico de salud de los backends
    pub async fn start_health_checks(
        self: Arc<Self>,
        backends: Vec<Backend>,
        interval_secs: u64,
    ) {
        let mut interval = interval(Duration::from_secs(interval_secs));

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                for backend in &backends {
                    let checker = self.clone();
                    let backend = backend.clone();

                    tokio::spawn(async move {
                        checker.check_backend(&backend).await;
                    });
                }
            }
        });
    }

    /// Verifica la salud de un backend específico
    async fn check_backend(&self, backend: &Backend) {
        let health_url = format!("{}/api/v1/health", backend.server_url.trim_end_matches('/'));

        let mut request = self.client.get(&health_url);

        // Agrega el header X-KV-SECRET si está configurado
        if let Some(ref secret) = self.vk_secret {
            request = request.header("X-KV-SECRET", secret);
        }

        let is_healthy = match request.send().await {
            Ok(response) => {
                if response.status().is_success() {
                    tracing::debug!("Backend {} is healthy", backend.server_id);
                    true
                } else {
                    tracing::warn!(
                        "Backend {} returned status {}",
                        backend.server_id,
                        response.status()
                    );
                    false
                }
            }
            Err(e) => {
                tracing::warn!("Backend {} health check failed: {}", backend.server_id, e);
                false
            }
        };

        // Actualiza el estado de salud
        let mut health_map = self.health_status.write().await;
        let status = health_map
            .entry(backend.server_id.clone())
            .or_insert(HealthStatus {
                is_healthy: true,
                last_check: std::time::Instant::now(),
                consecutive_failures: 0,
            });

        status.last_check = std::time::Instant::now();

        if is_healthy {
            status.is_healthy = true;
            status.consecutive_failures = 0;
        } else {
            status.consecutive_failures += 1;

            // Marca como no saludable después de 3 fallos consecutivos
            if status.consecutive_failures >= 3 {
                status.is_healthy = false;
                tracing::error!(
                    "Backend {} marked as unhealthy after {} consecutive failures",
                    backend.server_id,
                    status.consecutive_failures
                );
            }
        }
    }

    /// Retorna solo los backends saludables
    pub async fn get_healthy_backends(&self, backends: &[Backend]) -> Vec<Backend> {
        let health_map = self.health_status.read().await;

        backends
            .iter()
            .filter(|backend| {
                health_map
                    .get(&backend.server_id)
                    .map(|status| status.is_healthy)
                    .unwrap_or(true) // Si no se ha chequeado, asume que está saludable
            })
            .cloned()
            .collect()
    }

    /// Verifica si un backend específico está saludable
    pub async fn is_backend_healthy(&self, server_id: &str) -> bool {
        let health_map = self.health_status.read().await;
        health_map
            .get(server_id)
            .map(|status| status.is_healthy)
            .unwrap_or(true)
    }

    /// Retorna el estado de salud de todos los backends
    pub async fn get_all_health_status(&self) -> HashMap<String, HealthStatus> {
        self.health_status.read().await.clone()
    }
}
