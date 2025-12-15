use super::LoadBalancer;
use crate::db::Backend;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Balanceador Round Robin - distribuye las peticiones de manera circular
pub struct RoundRobinBalancer {
    counter: AtomicUsize,
}

impl RoundRobinBalancer {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LoadBalancer for RoundRobinBalancer {
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }

        let index = self.counter.fetch_add(1, Ordering::Relaxed) % backends.len();
        Some(backends[index].clone())
    }

    async fn release_backend(&self, _backend: &Backend) {
        // Round robin no necesita liberar recursos
    }

    fn name(&self) -> &str {
        "RoundRobin"
    }
}

/// Balanceador Least Connections - selecciona el backend con menos conexiones activas
pub struct LeastConnectionsBalancer {
    connections: Arc<RwLock<HashMap<String, usize>>>,
}

impl LeastConnectionsBalancer {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl LoadBalancer for LeastConnectionsBalancer {
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }

        let connections = self.connections.read().await;

        // Encuentra el backend con menos conexiones
        let selected = backends
            .iter()
            .min_by_key(|backend| {
                connections.get(&backend.server_id).unwrap_or(&0)
            })
            .cloned();

        drop(connections);

        // Incrementa el contador de conexiones
        if let Some(ref backend) = selected {
            let mut connections = self.connections.write().await;
            *connections.entry(backend.server_id.clone()).or_insert(0) += 1;
        }

        selected
    }

    async fn release_backend(&self, backend: &Backend) {
        let mut connections = self.connections.write().await;
        if let Some(count) = connections.get_mut(&backend.server_id) {
            *count = count.saturating_sub(1);
        }
    }

    fn name(&self) -> &str {
        "LeastConnections"
    }
}

/// Balanceador Random - selecciona un backend aleatoriamente
pub struct RandomBalancer;

impl RandomBalancer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LoadBalancer for RandomBalancer {
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }

        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hash, Hasher};

        // Usa un hash aleatorio basado en el timestamp
        let s = RandomState::new();
        let mut hasher = s.build_hasher();
        std::time::SystemTime::now().hash(&mut hasher);
        let hash = hasher.finish();

        let index = (hash as usize) % backends.len();
        Some(backends[index].clone())
    }

    async fn release_backend(&self, _backend: &Backend) {
        // Random no necesita liberar recursos
    }

    fn name(&self) -> &str {
        "Random"
    }
}

/// Balanceador Weighted Round Robin - distribuye basado en pesos
/// Los pesos se basan en el provider (puedes ajustar según necesites)
pub struct WeightedRoundRobinBalancer {
    counter: AtomicUsize,
}

impl WeightedRoundRobinBalancer {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }

    fn get_weight(provider: &str) -> usize {
        match provider {
            "supabase" => 3, // Supabase recibe 3x más tráfico
            "gdrive" => 1,   // Google Drive recibe 1x
            _ => 1,
        }
    }
}

#[async_trait]
impl LoadBalancer for WeightedRoundRobinBalancer {
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend> {
        if backends.is_empty() {
            return None;
        }

        // Construye una lista ponderada de backends
        let mut weighted_backends = Vec::new();
        for backend in backends {
            let weight = Self::get_weight(&backend.provider);
            for _ in 0..weight {
                weighted_backends.push(backend.clone());
            }
        }

        if weighted_backends.is_empty() {
            return None;
        }

        let index = self.counter.fetch_add(1, Ordering::Relaxed) % weighted_backends.len();
        Some(weighted_backends[index].clone())
    }

    async fn release_backend(&self, _backend: &Backend) {
        // Weighted round robin no necesita liberar recursos
    }

    fn name(&self) -> &str {
        "WeightedRoundRobin"
    }
}
