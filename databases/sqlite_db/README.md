# Rush SQLite Database Backend

[![Rust](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.5-green.svg)](#)

**SQLite Database Backend** es la implementación de base de datos embebida optimizada para Rush-rs, proporcionando persistencia confiable, segura y de alto rendimiento usando SQLite como motor de almacenamiento.

## 🗄️ Características Principales

### Base de Datos Embebida Optimizada
- **SQLite 3** como motor de almacenamiento confiable
- **WAL Mode** (Write-Ahead Logging) para mejor concurrencia
- **Memory-mapped I/O** para acceso de datos ultrarrápido
- **Cache inteligente** de 40MB configurado automáticamente
- **Optimización automática** cada 10,000 consultas

### Seguridad Robusta
- **Prevención de SQL injection** con validación de identificadores
- **Sanitización automática** de todos los inputs
- **Constraints de integridad** habilitadas por defecto
- **Validación exhaustiva** de parámetros de entrada
- **Zero unsafe code** - 100% Rust seguro

### Rendimiento Optimizado
- **Performance monitoring** integrado con métricas detalladas
- **Connection pooling** implícito con Arc<Mutex<Connection>>
- **Batch operations** para procesamiento eficiente
- **Pragma optimizations** aplicadas automáticamente
- **Auto-optimization** basada en patrones de uso

## 🏗️ Arquitectura

```
SqliteManager
├── Connection Pool         # Gestión de conexiones thread-safe
├── Performance Monitor     # Métricas y auto-optimización
├── Security Layer         # Prevención de SQL injection
├── Collection Store       # Almacén de key-value ordenado
└── State Store           # Almacén de estado único
    ├── Table Creation    # Esquemas optimizados
    ├── Query Execution   # Consultas parametrizadas seguras
    └── Iterator Support  # Iteración eficiente de resultados
```

## 🚀 Inicio Rápido

### Configuración Básica

```rust
use sqlite_db::*;
use rush_store::database::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Crear manager de SQLite
    let manager = SqliteManager::new("./my_database")?;

    // La base de datos se crea automáticamente con:
    // - WAL mode habilitado
    // - Cache de 40MB
    // - Memory-mapped I/O optimizado
    // - Foreign keys habilitadas
    // - Optimizaciones de rendimiento aplicadas

    // 2. Crear collection para almacenar key-value pairs
    let mut collection = manager.create_collection(
        "events",           // Nombre de la tabla
        "user_123"         // Prefijo para los datos
    )?;

    // 3. Almacenar datos
    collection.put("event_1", b"{'type': 'login', 'timestamp': '2025-01-01T10:00:00Z'}")?;
    collection.put("event_2", b"{'type': 'purchase', 'amount': 99.99}")?;

    // 4. Recuperar datos
    let event_data = collection.get("event_1")?;
    println!("Event data: {}", String::from_utf8_lossy(&event_data));

    // 5. Iterar sobre todos los eventos
    for (key, value) in collection.iter(false) {
        println!("Key: {}, Value: {}", key, String::from_utf8_lossy(&value));
    }

    Ok(())
}
```

### State Storage (Almacén de Estado Único)

```rust
use sqlite_db::*;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct UserProfile {
    name: String,
    email: String,
    preferences: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = SqliteManager::new("./user_profiles")?;

    // Crear state store para un solo valor por entidad
    let mut state_store = manager.create_state(
        "user_profiles",    // Tabla
        "user_456"         // ID único del usuario
    )?;

    // Serializar y almacenar estado
    let profile = UserProfile {
        name: "Alice Johnson".to_string(),
        email: "alice@example.com".to_string(),
        preferences: vec!["dark_mode".to_string(), "notifications_enabled".to_string()],
    };

    let serialized = bincode::serialize(&profile)?;
    state_store.put(&serialized)?;

    // Recuperar y deserializar estado
    let stored_data = state_store.get()?;
    let recovered_profile: UserProfile = bincode::deserialize(&stored_data)?;

    println!("Perfil recuperado: {:?}", recovered_profile);

    Ok(())
}
```

## 📊 Monitoring de Rendimiento

### Métricas Integradas

```rust
let manager = SqliteManager::new("./monitored_db")?;

// Realizar operaciones
let mut collection = manager.create_collection("test_data", "prefix")?;
for i in 0..1000 {
    collection.put(&format!("key_{}", i), &format!("value_{}", i).into_bytes())?;
}

// Obtener estadísticas de rendimiento
let stats = manager.performance_stats();

println!("Performance Statistics:");
println!("  Total queries executed: {}", stats.query_count);
println!("  Last optimization time: {}", stats.last_optimize_time);

// Estadísticas en tiempo real
println!("  Queries per second: {}", stats.queries_per_second());
println!("  Time since last optimization: {} hours",
         stats.hours_since_last_optimization());
```

### Optimización Manual

```rust
let manager = SqliteManager::new("./database")?;

// La optimización se ejecuta automáticamente cada 10,000 queries
// Pero también puede ejecutarse manualmente

manager.optimize()?;
println!("Database optimized manually");

// La optimización incluye:
// - PRAGMA optimize
// - ANALYZE para estadísticas actualizadas
// - Recolección de estadísticas de rendimiento
```

### Health Checks

```rust
impl SqliteManager {
    pub fn health_check(&self) -> Result<HealthStatus, Error> {
        // Verificar conectividad
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("Connection check failed: {}", e))
        })?;

        // Test de conectividad básica
        conn.execute("SELECT 1", []).map_err(|e| {
            Error::Store(format!("Health check query failed: {}", e))
        })?;

        // Verificar métricas
        let stats = self.performance_stats();

        if stats.query_count > 0 {
            Ok(HealthStatus::Healthy)
        } else {
            Ok(HealthStatus::Unknown)
        }
    }
}
```

## 🔒 Seguridad y Validación

### Prevención de SQL Injection

```rust
// El sistema incluye validación automática de identificadores
use sqlite_db::validate_sql_identifier;

// ✅ Identificadores válidos
assert!(validate_sql_identifier("user_events").is_ok());
assert!(validate_sql_identifier("table123").is_ok());
assert!(validate_sql_identifier("_internal_table").is_ok());

// ❌ Identificadores peligrosos rechazados automáticamente
assert!(validate_sql_identifier("users; DROP TABLE--").is_err());
assert!(validate_sql_identifier("table'name").is_err());
assert!(validate_sql_identifier("").is_err()); // Vacío
assert!(validate_sql_identifier("very_long_identifier_that_exceeds_the_64_character_limit_and_should_be_rejected").is_err());
```

### Sanitización Automática

```rust
// Todos los identificadores se sanitizan automáticamente
let manager = SqliteManager::new("./secure_db")?;

// El nombre de tabla se valida y sanitiza
let collection = manager.create_collection(
    "user_data",  // Validado: solo caracteres alfanuméricos y _
    "user_123"    // Validado: formato de prefijo seguro
)?;

// Las consultas usan parámetros prepared para prevenir injection
collection.put("key", b"data")?; // Automáticamente parametrizado

// Resultado: SELECT value FROM "user_data" WHERE prefix = ?1 AND sn = ?2
// Los valores de los parámetros no pueden causar SQL injection
```

### Configuración de Seguridad

```rust
// El sistema aplica automáticamente configuración de seguridad:
//
// PRAGMA foreign_keys=ON;                    -- Integridad referencial
// PRAGMA ignore_check_constraints=OFF;       -- Validar constraints
// PRAGMA trusted_schema=OFF;                 -- Deshabilitar esquemas no confiables
// PRAGMA defensive=ON;                       -- Modo defensivo
```

## ⚡ Optimizaciones de Rendimiento

### Configuración Automática

```rust
// Al abrir una conexión, se aplican automáticamente estas optimizaciones:

// PRAGMA journal_mode=WAL;           -- Write-Ahead Logging
// PRAGMA synchronous=NORMAL;         -- Balance seguridad/rendimiento
// PRAGMA cache_size=10000;           -- Cache de 40MB
// PRAGMA temp_store=memory;          -- Tablas temporales en RAM
// PRAGMA mmap_size=268435456;        -- 256MB memory-mapped I/O
// PRAGMA optimize;                   -- Optimizaciones automáticas
```

### Batch Operations

```rust
let manager = SqliteManager::new("./batch_db")?;
let mut collection = manager.create_collection("bulk_data", "batch_prefix")?;

// Para insertar múltiples elementos eficientemente
let items: Vec<(String, Vec<u8>)> = (0..10000)
    .map(|i| (format!("key_{}", i), format!("value_{}", i).into_bytes()))
    .collect();

// Begin transaction para batch insert
{
    let conn = collection.conn.lock().unwrap();
    let tx = conn.unchecked_transaction()?;

    for (key, value) in &items {
        collection.put(key, value)?;
    }

    tx.commit()?;
}

println!("Inserted {} items in batch", items.len());
```

### Memory-Mapped I/O

```rust
// SQLite está configurado con memory-mapped I/O para máximo rendimiento:
// - 256MB de memory-mapping habilitado
// - Acceso directo a páginas de datos en memoria
// - Reducción de llamadas al sistema
// - Latencia mínima para lecturas frecuentes

let manager = SqliteManager::new("./mmap_optimized")?;
let collection = manager.create_collection("hot_data", "frequently_accessed")?;

// Las lecturas frecuentes se benefician de memory-mapping
for i in 0..1000 {
    let key = format!("hot_key_{}", i % 100); // Acceso repetitivo
    if let Ok(data) = collection.get(&key) {
        // Data accedida directamente desde memoria mapeada
        println!("Hot data: {} bytes", data.len());
    }
}
```

## 🔄 Iteradores y Consultas

### Iteración Básica

```rust
let manager = SqliteManager::new("./iteration_db")?;
let mut collection = manager.create_collection("items", "prefix")?;

// Insertar datos de prueba
for i in 1..=100 {
    collection.put(&format!("item_{:03}", i), &format!("data_{}", i).into_bytes())?;
}

// Iterar en orden ascendente
println!("Ascending order:");
for (key, value) in collection.iter(false) {
    println!("  {}: {}", key, String::from_utf8_lossy(&value));
}

// Iterar en orden descendente
println!("Descending order:");
for (key, value) in collection.iter(true) {
    println!("  {}: {}", key, String::from_utf8_lossy(&value));
}
```

### Iteración Eficiente con Batching

```rust
use sqlite_db::SqliteCollection;

impl SqliteCollection {
    pub fn iter_batched(&self, batch_size: usize, reverse: bool) -> BatchIterator {
        BatchIterator::new(self, batch_size, reverse)
    }
}

pub struct BatchIterator<'a> {
    collection: &'a SqliteCollection,
    batch_size: usize,
    current_offset: usize,
    current_batch: Vec<(String, Vec<u8>)>,
    reverse: bool,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = (String, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        // Implementación de iteración por lotes para mejor rendimiento
        // con grandes datasets
        if self.current_batch.is_empty() {
            self.load_next_batch();
        }

        self.current_batch.pop()
    }
}

// Uso
let batch_iter = collection.iter_batched(1000, false);
for (key, value) in batch_iter {
    // Procesa en lotes de 1000 para eficiencia
    process_item(&key, &value);
}
```

## 🧪 Testing y Desarrollo

### Tests Unitarios

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_collection_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let manager = SqliteManager::new(db_path).unwrap();
        let mut collection = manager.create_collection("test_table", "test_prefix").unwrap();

        // Test put/get
        collection.put("key1", b"value1").unwrap();
        let retrieved = collection.get("key1").unwrap();
        assert_eq!(retrieved, b"value1");

        // Test delete
        collection.del("key1").unwrap();
        assert!(collection.get("key1").is_err());
    }

    #[test]
    fn test_sql_injection_prevention() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        // Intentos de SQL injection deben fallar en la validación
        let malicious_names = vec![
            "users; DROP TABLE users; --",
            "test' OR '1'='1",
            "table\"; DELETE FROM users; --",
            "",
            "a".repeat(65), // Muy largo
        ];

        for malicious_name in malicious_names {
            assert!(manager.create_collection(&malicious_name, "prefix").is_err());
        }
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap());

        let handles: Vec<_> = (0..10).map(|i| {
            let manager_clone = Arc::clone(&manager);
            thread::spawn(move || {
                let mut collection = manager_clone
                    .create_collection("concurrent_test", &format!("thread_{}", i))
                    .unwrap();

                for j in 0..100 {
                    let key = format!("key_{}", j);
                    let value = format!("value_{}_{}", i, j);
                    collection.put(&key, value.as_bytes()).unwrap();
                }
            })
        }).collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Verificar que todos los datos se escribieron correctamente
        let stats = manager.performance_stats();
        assert!(stats.query_count > 0);
    }
}
```

### Benchmarks de Rendimiento

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    #[test]
    fn benchmark_insert_performance() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut collection = manager.create_collection("benchmark", "perf_test").unwrap();

        let num_items = 10000;
        let start = Instant::now();

        for i in 0..num_items {
            let key = format!("key_{:06}", i);
            let value = format!("value_{}_with_some_longer_data_to_test_performance", i);
            collection.put(&key, value.as_bytes()).unwrap();
        }

        let duration = start.elapsed();
        let items_per_sec = num_items as f64 / duration.as_secs_f64();

        println!("Inserted {} items in {:?}", num_items, duration);
        println!("Performance: {:.2} items/second", items_per_sec);

        // Verificar que el rendimiento es aceptable
        assert!(items_per_sec > 1000.0, "Performance below threshold: {} items/sec", items_per_sec);
    }

    #[test]
    fn benchmark_read_performance() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut collection = manager.create_collection("read_bench", "read_test").unwrap();

        // Preparar datos
        let num_items = 1000;
        for i in 0..num_items {
            collection.put(&format!("key_{:03}", i), &format!("value_{}", i).into_bytes()).unwrap();
        }

        // Benchmark de lectura
        let start = Instant::now();
        let mut total_bytes = 0;

        for _ in 0..10000 {
            let key = format!("key_{:03}", fastrand::usize(0..num_items));
            if let Ok(data) = collection.get(&key) {
                total_bytes += data.len();
            }
        }

        let duration = start.elapsed();
        let reads_per_sec = 10000.0 / duration.as_secs_f64();

        println!("Read {} random keys in {:?}", 10000, duration);
        println!("Performance: {:.2} reads/second", reads_per_sec);
        println!("Total data read: {} bytes", total_bytes);

        assert!(reads_per_sec > 10000.0, "Read performance below threshold");
    }
}
```

## 🔧 Configuración Avanzada

### Personalización de Conexión

```rust
use rusqlite::{Connection, OpenFlags};

impl SqliteManager {
    pub fn with_custom_config(
        path: &str,
        config: SqliteConfig,
    ) -> Result<Self, Error> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | if config.read_only { OpenFlags::SQLITE_OPEN_READ_ONLY } else { OpenFlags::empty() };

        let conn = Connection::open_with_flags(path, flags)?;

        // Aplicar configuración personalizada
        let pragma_statements = format!(
            "
            PRAGMA journal_mode={};
            PRAGMA synchronous={};
            PRAGMA cache_size={};
            PRAGMA temp_store={};
            PRAGMA mmap_size={};
            ",
            config.journal_mode,
            config.synchronous_mode,
            config.cache_size_pages,
            config.temp_store_mode,
            config.mmap_size_bytes,
        );

        conn.execute_batch(&pragma_statements)?;

        Ok(SqliteManager {
            conn: Arc::new(Mutex::new(conn)),
            query_count: Arc::new(AtomicUsize::new(0)),
            last_optimize_time: Arc::new(AtomicU64::new(0)),
        })
    }
}

#[derive(Debug)]
pub struct SqliteConfig {
    pub journal_mode: JournalMode,
    pub synchronous_mode: SynchronousMode,
    pub cache_size_pages: i32,
    pub temp_store_mode: TempStoreMode,
    pub mmap_size_bytes: i64,
    pub read_only: bool,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        SqliteConfig {
            journal_mode: JournalMode::WAL,
            synchronous_mode: SynchronousMode::Normal,
            cache_size_pages: 10000,    // 40MB
            temp_store_mode: TempStoreMode::Memory,
            mmap_size_bytes: 256 * 1024 * 1024, // 256MB
            read_only: false,
        }
    }
}
```

### Monitoreo Personalizado

```rust
pub trait PerformanceMonitor {
    fn on_query_executed(&self, query_type: QueryType, duration: Duration);
    fn on_optimization_completed(&self, duration: Duration);
    fn on_error_occurred(&self, error: &Error);
}

pub struct MetricsCollector {
    metrics: Arc<Mutex<DatabaseMetrics>>,
}

impl PerformanceMonitor for MetricsCollector {
    fn on_query_executed(&self, query_type: QueryType, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.record_query(query_type, duration);
    }

    fn on_optimization_completed(&self, duration: Duration) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.record_optimization(duration);
    }

    fn on_error_occurred(&self, error: &Error) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.record_error(error);
    }
}

// Integrar monitor personalizado
let monitor = Arc::new(MetricsCollector::new());
let manager = SqliteManager::with_monitor("./monitored_db", monitor)?;
```

## 📚 API Reference

### Core Types

- **`SqliteManager`** - Manager principal de base de datos SQLite
- **`SqliteCollection`** - Almacén de key-value pairs ordenados
- **`PerformanceStats`** - Métricas de rendimiento de la base de datos
- **`SqliteIterResult<'a>`** - Type alias para resultados de iteradores

### Traits Implementados

- **`DbManager`** - Interface principal de gestión de base de datos
- **`Collection`** - Interface para almacenes de key-value
- **`State`** - Interface para almacenes de estado único

### Funciones Principales

- **`new(path)`** - Crear nuevo manager de SQLite
- **`create_collection(name, prefix)`** - Crear almacén de colección
- **`create_state(name, prefix)`** - Crear almacén de estado
- **`performance_stats()`** - Obtener métricas de rendimiento
- **`optimize()`** - Ejecutar optimización manual

---

Este backend SQLite proporciona una base de datos embebida robusta, segura y optimizada para aplicaciones Rush-rs que requieren persistencia confiable con excelente rendimiento. 🗄️⚡🔒