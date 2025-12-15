# VK Gateway - Reverse Proxy con Balanceo de Carga

Gateway de alto rendimiento construido con Axum y Rust que implementa un reverse proxy con balanceo de carga desacoplado para el servicio VK.

## Características

- **Reverse Proxy Completo**: Reenvía todas las peticiones HTTP a backends configurados
- **Balanceo de Carga Desacoplado**: Arquitectura modular que permite cambiar algoritmos fácilmente
- **Health Checks Automáticos**: Monitoreo periódico de la salud de los backends
- **Múltiples Algoritmos de Balanceo**:
  - Round Robin
  - Least Connections
  - Random
  - Weighted Round Robin (por provider)
- **Gestión de Backends Dinámica**: Backends configurados en PostgreSQL
- **Caché con Redis**: Soporte para caché distribuido
- **Logging Detallado**: Sistema de logging con tracing
- **CORS Habilitado**: Configuración CORS permisiva

## Arquitectura

```
┌─────────────┐
│   Cliente   │
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────────┐
│        VK Gateway (Axum)            │
│  ┌──────────────────────────────┐  │
│  │   Load Balancer (Trait)      │  │
│  │  - Round Robin               │  │
│  │  - Least Connections         │  │
│  │  - Random                    │  │
│  │  - Weighted Round Robin      │  │
│  └──────────────────────────────┘  │
│  ┌──────────────────────────────┐  │
│  │   Health Checker             │  │
│  │  (monitoreo periódico)       │  │
│  └──────────────────────────────┘  │
└─────────────┬───────────────────────┘
              │
       ┌──────┴──────┐
       ▼             ▼
┌─────────────┐ ┌─────────────┐
│  Backend 1  │ │  Backend 2  │
│  (Supabase) │ │  (GDrive)   │
└─────────────┘ └─────────────┘
       │             │
       ▼             ▼
┌──────────────────────────────┐
│     PostgreSQL (Supabase)    │
│   - Backends (config.local)  │
└──────────────────────────────┘
```

## Instalación

### Requisitos

- Rust 1.70+
- PostgreSQL (Supabase)
- Redis (Upstash)

### Configuración

1. Clona el repositorio:
```bash
git clone <repo-url>
cd vk-gateway
```

2. Configura las variables de entorno en `.env`:
```bash
# Database
DATABASE_URL=postgresql://user:password@host:5432/database
REDIS_URL=rediss://default:password@host:6379

# Gateway Configuration
SERVER_ID=your-gateway-uuid
PORT=3000

# Load Balancer Strategy (opcional)
# Opciones: round-robin, least-connections, random, weighted-round-robin
LOAD_BALANCER_STRATEGY=round-robin

# Health Check Interval (opcional, en segundos)
HEALTH_CHECK_INTERVAL=30

# VK Secret para health checks (opcional)
VK_SECRET=your-secret-key
```

3. Asegúrate de que la base de datos tenga backends configurados:
```sql
-- Ejemplo de inserción de backends
INSERT INTO config.local (server_id, provider, server_name, server_url)
VALUES
  ('backend-1-uuid', 'supabase', 'Backend Supabase 1', 'https://backend1.example.com'),
  ('backend-2-uuid', 'gdrive', 'Backend GDrive 1', 'https://backend2.example.com');
```

4. Compila y ejecuta:
```bash
cargo build --release
cargo run
```

## Uso

### Endpoints del Gateway

#### Health Check del Gateway
```bash
GET http://localhost:3000/health
```

Respuesta:
```
Gateway is healthy
```

#### Estadísticas del Gateway
```bash
GET http://localhost:3000/stats
```

Respuesta:
```json
{
  "load_balancer": "RoundRobin",
  "total_backends": 2,
  "healthy_backends": 2,
  "backends": [
    {
      "server_id": "backend-1-uuid",
      "server_name": "Backend Supabase 1",
      "server_url": "https://backend1.example.com",
      "provider": "supabase",
      "is_healthy": true,
      "consecutive_failures": 0
    }
  ]
}
```

#### Proxy a Backend Específico
```bash
# Accede a un backend específico por su ID
GET http://localhost:3000/backend/{server_id}/api/v1/users
```

#### Proxy con Balanceo de Carga
```bash
# Todas las demás rutas se balancean automáticamente
GET http://localhost:3000/api/v1/files
POST http://localhost:3000/api/v1/users
```

## Cambiar el Algoritmo de Balanceo

El sistema está diseñado para permitir cambios rápidos en el algoritmo de balanceo.

### Método 1: Variable de Entorno

Simplemente cambia la variable `LOAD_BALANCER_STRATEGY` en tu `.env`:

```bash
# Round Robin (por defecto)
LOAD_BALANCER_STRATEGY=round-robin

# Least Connections
LOAD_BALANCER_STRATEGY=least-connections

# Random
LOAD_BALANCER_STRATEGY=random

# Weighted Round Robin (prioriza según provider)
LOAD_BALANCER_STRATEGY=weighted-round-robin
```

### Método 2: Modificar el Código

En `src/main.rs` línea 72-75:

```rust
let load_balancer_strategy = std::env::var("LOAD_BALANCER_STRATEGY")
    .unwrap_or_else(|_| "round-robin".to_string());

let load_balancer = create_load_balancer(&load_balancer_strategy);
```

Cambia el valor por defecto o fuerza una estrategia específica:

```rust
let load_balancer = create_load_balancer("least-connections");
```

### Método 3: Crear un Algoritmo Personalizado

1. Crea una nueva estructura en `src/load_balancer/strategies.rs`:

```rust
pub struct CustomBalancer {
    // Tu estado interno
}

impl CustomBalancer {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl LoadBalancer for CustomBalancer {
    async fn select_backend(&self, backends: &[Backend]) -> Option<Backend> {
        // Tu lógica de selección
        todo!()
    }

    async fn release_backend(&self, backend: &Backend) {
        // Liberar recursos si es necesario
    }

    fn name(&self) -> &str {
        "Custom"
    }
}
```

2. Registra tu algoritmo en `src/load_balancer/mod.rs`:

```rust
pub fn create_load_balancer(strategy: &str) -> Arc<dyn LoadBalancer> {
    match strategy.to_lowercase().as_str() {
        "custom" => Arc::new(strategies::CustomBalancer::new()),
        // ... otros casos
    }
}
```

## Algoritmos de Balanceo Disponibles

### Round Robin
- **Descripción**: Distribuye las peticiones de manera circular entre todos los backends
- **Uso recomendado**: Cuando todos los backends tienen capacidad similar
- **Pros**: Simple, distribución uniforme
- **Contras**: No considera la carga actual de los backends

### Least Connections
- **Descripción**: Envía la petición al backend con menos conexiones activas
- **Uso recomendado**: Cuando las peticiones tienen duración variable
- **Pros**: Mejor distribución de carga real
- **Contras**: Overhead de tracking de conexiones

### Random
- **Descripción**: Selecciona un backend aleatoriamente
- **Uso recomendado**: Testing o distribución simple sin estado
- **Pros**: Sin estado, muy simple
- **Contras**: Distribución no garantizada

### Weighted Round Robin
- **Descripción**: Round robin con pesos basados en el provider
- **Pesos**: Supabase=3x, GDrive=1x
- **Uso recomendado**: Cuando algunos backends pueden manejar más carga
- **Pros**: Distribución proporcional a la capacidad
- **Contras**: Requiere configurar pesos manualmente

## Health Checks

El gateway realiza health checks periódicos a todos los backends:

- **Endpoint**: `/api/v1/health` en cada backend
- **Intervalo**: Configurable con `HEALTH_CHECK_INTERVAL` (default: 30s)
- **Timeout**: 5 segundos
- **Umbral**: 3 fallos consecutivos marcan el backend como no saludable
- **Header**: `X-KV-SECRET` si está configurado

Los backends no saludables son excluidos automáticamente del balanceo hasta que vuelvan a estar operativos.

## Logging

El gateway usa `tracing` para logging detallado. Puedes configurar el nivel de logs:

```bash
# En desarrollo
RUST_LOG=vk_gateway=debug,tower_http=debug,axum=debug cargo run

# En producción
RUST_LOG=vk_gateway=info,tower_http=info cargo run
```

## Estructura del Proyecto

```
vk-gateway/
├── src/
│   ├── main.rs              # Punto de entrada y configuración del servidor
│   ├── config.rs            # Configuración desde variables de entorno
│   ├── db.rs                # Conexión a PostgreSQL y queries
│   ├── cache.rs             # Cliente de Redis
│   ├── health.rs            # Health checker para backends
│   ├── proxy.rs             # Handlers del proxy
│   └── load_balancer/
│       ├── mod.rs           # Trait LoadBalancer y factory
│       └── strategies.rs    # Implementaciones de algoritmos
├── Cargo.toml               # Dependencias
├── .env                     # Variables de entorno
├── schema.sql               # Schema de la base de datos
└── README.md                # Esta documentación
```

## Desarrollo

### Testing Local

1. Inicia backends de prueba en diferentes puertos
2. Inserta sus URLs en la tabla `config.local`
3. Ejecuta el gateway:
```bash
cargo run
```

4. Prueba las peticiones:
```bash
# Health check
curl http://localhost:3000/health

# Stats
curl http://localhost:3000/stats

# Proxy
curl http://localhost:3000/api/v1/health
```

### Compilación Optimizada

```bash
cargo build --release
./target/release/vk-gateway
```

## Rendimiento

- **Async/Await**: Todo el código es asíncrono para máximo throughput
- **Connection Pooling**: PostgreSQL y Redis usan pools de conexiones
- **HTTP/2**: Soporte completo para HTTP/2
- **Zero-Copy**: Proxy de body sin copias innecesarias

## Próximas Mejoras

- [ ] Rate limiting por IP
- [ ] Sticky sessions para uploads grandes
- [ ] Métricas con Prometheus
- [ ] Circuit breaker pattern
- [ ] Retry automático con backoff
- [ ] Hot reload de configuración
- [ ] WebSocket proxying
- [ ] TLS/HTTPS termination

## Licencia

MIT

## Soporte

Para issues o preguntas, abre un issue en el repositorio.
