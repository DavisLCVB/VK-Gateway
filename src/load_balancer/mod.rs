pub mod strategies;

use crate::db::Backend;
use async_trait::async_trait;
use std::sync::Arc;

/// Trait que define el comportamiento de un balanceador de carga.
/// Implementa este trait para crear nuevos algoritmos de balanceo.
#[async_trait]
pub trait LoadBalancer: Send + Sync {
    /// Selecciona el siguiente backend disponible basado en el algoritmo de balanceo.
    ///
    /// # Arguments
    /// * `backends` - Lista de backends disponibles y saludables
    ///
    /// # Returns
    /// El backend seleccionado o None si no hay backends disponibles
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend>;

    /// Notifica al balanceador que una petición ha sido completada.
    /// Útil para algoritmos que rastrean conexiones activas.
    async fn release_backend(&self, backend: &Backend);

    /// Retorna el nombre del algoritmo de balanceo
    fn name(&self) -> &str;
}

/// Factory para crear diferentes tipos de balanceadores
pub fn create_load_balancer(strategy: &str) -> Arc<dyn LoadBalancer> {
    match strategy.to_lowercase().as_str() {
        "round-robin" | "roundrobin" => Arc::new(strategies::RoundRobinBalancer::new()),
        "least-connections" | "leastconnections" => Arc::new(strategies::LeastConnectionsBalancer::new()),
        "random" => Arc::new(strategies::RandomBalancer::new()),
        "weighted-round-robin" | "weightedroundrobin" => Arc::new(strategies::WeightedRoundRobinBalancer::new()),
        _ => {
            tracing::warn!("Unknown load balancer strategy '{}', defaulting to round-robin", strategy);
            Arc::new(strategies::RoundRobinBalancer::new())
        }
    }
}
