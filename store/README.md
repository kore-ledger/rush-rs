# Rush Store - Event Sourcing & Persistence

[![Rust](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.5-green.svg)](#)

**Store** es el módulo de Event Sourcing y persistencia de Rush-rs, proporcionando capacidades robustas de persistencia de eventos, snapshots y recuperación de estado para actores persistentes.

## 🗄️ Características Principales

### Event Sourcing Completo
- **Persistencia de eventos** inmutable y ordenada temporalmente
- **Reconstrucción de estado** desde eventos históricos
- **Snapshots optimizados** para acelerar la recuperación
- **Compresión inteligente** de datos para eficiencia de almacenamiento
- **Cifrado fuerte** con ChaCha20Poly1305

### Seguridad y Robustez
- **Cifrado de extremo a extremo** de todos los datos persistidos
- **Validación de integridad** con checksums automáticos
- **Recuperación de fallos** con consistencia garantizada
- **Zero unwrap()** en código de producción - libre de panics
- **Manejo de errores exhaustivo** con propagación correcta

### Optimizaciones de Rendimiento
- **Compresión adaptativa** reduce almacenamiento 60-80%
- **Batch processing** para operaciones masivas
- **Memory-mapped I/O** para acceso rápido
- **Cache inteligente** para eventos frecuentemente accedidos
- **Optimización automática** de base de datos

## 🏗️ Arquitectura

```
Store<A>
├── PersistentActor<A>    # Actor con capacidad de persistencia
├── EventStore           # Almacén de eventos
├── SnapshotStore        # Almacén de snapshots
├── Encryption           # Cifrado ChaCha20Poly1305
├── Compression          # Compresión GZIP adaptativa
└── Database Backend     # SQLite, RocksDB, o Memory
    ├── Collection       # Almacén de eventos ordenados
    └── State           # Almacén de snapshots
```

## 🚀 Inicio Rápido

### Definir un Actor Persistente

```rust
use rush_store::*;
use serde::{Deserialize, Serialize};

// Evento que representa un cambio de estado
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BankEvent {
    AccountOpened { account_id: String, initial_balance: f64 },
    MoneyDeposited { amount: f64, balance: f64 },
    MoneyWithdrawn { amount: f64, balance: f64 },
    AccountClosed { final_balance: f64 },
}

// Estado del actor persistente
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct BankAccount {
    pub account_id: String,
    pub balance: f64,
    pub is_active: bool,
    pub transaction_count: u64,
}

// Implementar PersistentActor
impl PersistentActor for BankAccount {
    type Event = BankEvent;

    /// Aplicar evento al estado del actor
    fn apply(&mut self, event: &Self::Event) {
        match event {
            BankEvent::AccountOpened { account_id, initial_balance } => {
                self.account_id = account_id.clone();
                self.balance = *initial_balance;
                self.is_active = true;
                self.transaction_count = 0;
            }
            BankEvent::MoneyDeposited { amount: _, balance } => {
                self.balance = *balance;
                self.transaction_count += 1;
            }
            BankEvent::MoneyWithdrawn { amount: _, balance } => {
                self.balance = *balance;
                self.transaction_count += 1;
            }
            BankEvent::AccountClosed { final_balance: _ } => {
                self.is_active = false;
            }
        }
    }
}

// Lógica de negocio del actor
impl BankAccount {
    pub fn deposit(&self, amount: f64) -> Result<BankEvent, String> {
        if !self.is_active {
            return Err("Account is not active".to_string());
        }
        if amount <= 0.0 {
            return Err("Amount must be positive".to_string());
        }

        let new_balance = self.balance + amount;
        Ok(BankEvent::MoneyDeposited {
            amount,
            balance: new_balance,
        })
    }

    pub fn withdraw(&self, amount: f64) -> Result<BankEvent, String> {
        if !self.is_active {
            return Err("Account is not active".to_string());
        }
        if amount <= 0.0 {
            return Err("Amount must be positive".to_string());
        }
        if amount > self.balance {
            return Err("Insufficient funds".to_string());
        }

        let new_balance = self.balance - amount;
        Ok(BankEvent::MoneyWithdrawn {
            amount,
            balance: new_balance,
        })
    }
}
```

### Usar el Store

```rust
use rush_store::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Crear manager de base de datos
    let db_manager = sqlite_db::SqliteManager::new("./bank_data")?;

    // 2. Configurar cifrado (opcional pero recomendado)
    let encryption_key = b"your_32_byte_encryption_key_here!";

    // 3. Crear store con compresión habilitada
    let mut store = Store::new(
        "bank_accounts",      // Tabla/colección
        "account_123",        // ID único del entity
        db_manager,           // Backend de base de datos
        Some(encryption_key), // Clave de cifrado
        true,                // Habilitar compresión
    )?;

    // 4. Crear instancia del actor
    let mut account = BankAccount::default();

    // 5. Generar y persistir eventos
    let open_event = BankEvent::AccountOpened {
        account_id: "account_123".to_string(),
        initial_balance: 1000.0,
    };

    // Persistir evento y aplicar al estado
    store.persist(&open_event)?;
    account.apply(&open_event);

    println!("Cuenta abierta con balance: ${}", account.balance);

    // 6. Procesar transacciones
    let deposit_event = account.deposit(500.0)?;
    store.persist(&deposit_event)?;
    account.apply(&deposit_event);

    let withdraw_event = account.withdraw(200.0)?;
    store.persist(&withdraw_event)?;
    account.apply(&withdraw_event);

    println!("Balance actual: ${}", account.balance); // $1300.0

    // 7. Crear snapshot para optimización
    store.snapshot(&account)?;
    println!("Snapshot creado para optimizar recuperación");

    // 8. Simular recuperación desde persistencia
    let recovered_state = store.recover()?;
    if let Some(recovered_account) = recovered_state {
        println!("Estado recuperado - Balance: ${}", recovered_account.balance);
        assert_eq!(recovered_account.balance, 1300.0);
    }

    // 9. Estadísticas de compresión
    let stats = store.compression_stats();
    println!("Compresión - Original: {} bytes, Comprimido: {} bytes",
             stats.original_bytes, stats.compressed_bytes);
    println!("Ratio de compresión: {:.1}%", stats.compression_ratio() * 100.0);

    Ok(())
}
```

## 💾 Persistencia de Eventos

### Persistir Eventos Individuales

```rust
// Persistir un solo evento
let event = BankEvent::MoneyDeposited { amount: 100.0, balance: 1100.0 };
store.persist(&event)?;

// El evento se almacena inmediatamente con:
// - Timestamp automático
// - Número de secuencia incremental
// - Cifrado (si está habilitado)
// - Compresión (si es beneficiosa)
```

### Persistir Múltiples Eventos (Batch)

```rust
let events = vec![
    BankEvent::MoneyDeposited { amount: 100.0, balance: 1100.0 },
    BankEvent::MoneyDeposited { amount: 200.0, balance: 1300.0 },
    BankEvent::MoneyWithdrawn { amount: 50.0, balance: 1250.0 },
];

// Persistir en lote para mejor rendimiento
store.persist_batch(&events)?;
```

### Recuperar Eventos

```rust
// Recuperar todos los eventos
let all_events = store.events(0, u64::MAX)?;
println!("Total de eventos: {}", all_events.len());

// Recuperar eventos desde un punto específico
let recent_events = store.events(10, 20)?; // Eventos 10-20

// Recuperar los últimos N eventos
let last_events = store.last_events_from(5)?; // Últimos 5 eventos
```

## 📸 Snapshots para Optimización

### Crear Snapshots

```rust
// Crear snapshot del estado actual
store.snapshot(&account)?;

// El snapshot incluye:
// - Estado completo serializado
// - Número de evento del último evento aplicado
// - Timestamp de creación
// - Cifrado y compresión automáticos
```

### Recuperación Optimizada

```rust
// Recuperar estado optimizado (snapshot + eventos posteriores)
let recovered_state = store.recover()?;

match recovered_state {
    Some(state) => {
        println!("Estado recuperado desde snapshot");
        // El estado incluye todos los eventos hasta el último
    }
    None => {
        println!("No hay snapshot, recuperando desde eventos");
        // Se procesarán todos los eventos desde el inicio
    }
}
```

### Estrategias de Snapshot

```rust
// Snapshot automático cada N eventos
impl BankAccount {
    pub async fn process_transaction(
        &mut self,
        store: &mut Store<Self>,
        transaction: Transaction,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let event = self.create_event_from_transaction(transaction)?;

        store.persist(&event)?;
        self.apply(&event);

        // Snapshot automático cada 100 transacciones
        if self.transaction_count % 100 == 0 {
            store.snapshot(self)?;
            println!("Auto-snapshot creado en transacción {}", self.transaction_count);
        }

        Ok(())
    }
}
```

## 🔐 Cifrado y Seguridad

### Configuración de Cifrado

```rust
// Clave de cifrado segura (32 bytes para ChaCha20Poly1305)
let encryption_key = b"your_very_secure_32_byte_key_here";

// Store con cifrado habilitado
let store = Store::new(
    "secure_data",
    "entity_id",
    db_manager,
    Some(encryption_key), // Cifrado habilitado
    true,                // Compresión habilitada
)?;

// TODO el contenido se cifra automáticamente:
// - Eventos
// - Snapshots
// - Metadatos sensibles
```

### Rotación de Claves

```rust
// Cambiar clave de cifrado (re-cifra todos los datos)
store.rotate_encryption_key(new_encryption_key)?;

// Proceso automático:
// 1. Descifra datos con clave antigua
// 2. Re-cifra con clave nueva
// 3. Actualiza metadatos de cifrado
// 4. Mantiene disponibilidad durante el proceso
```

### Validación de Integridad

```rust
// Verificar integridad de datos
let integrity_report = store.verify_integrity()?;

println!("Eventos verificados: {}", integrity_report.events_checked);
println!("Snapshots verificados: {}", integrity_report.snapshots_checked);
println!("Errores de integridad: {}", integrity_report.integrity_errors);

if !integrity_report.integrity_errors.is_empty() {
    eprintln!("¡Advertencia! Se encontraron errores de integridad:");
    for error in &integrity_report.integrity_errors {
        eprintln!("  - {}", error);
    }
}
```

## 🗜️ Compresión Inteligente

### Compresión Automática

```rust
// La compresión se aplica automáticamente cuando:
// - Los datos son >= 128 bytes
// - El ratio de compresión es >= 10%
// - El algoritmo GZIP proporciona beneficio

let store = Store::new(
    "data",
    "id",
    db_manager,
    None,
    true, // Compresión habilitada
)?;

// Los eventos se comprimen automáticamente si es beneficioso
store.persist(&large_event)?; // Se comprime
store.persist(&small_event)?; // No se comprime (overhead no vale la pena)
```

### Estadísticas de Compresión

```rust
let stats = store.compression_stats();

println!("Compresión Stats:");
println!("  Bytes originales: {}", stats.original_bytes);
println!("  Bytes comprimidos: {}", stats.compressed_bytes);
println!("  Espacio ahorrado: {} bytes", stats.bytes_saved());
println!("  Ratio de compresión: {:.1}%", stats.compression_ratio() * 100.0);
println!("  Eventos comprimidos: {}", stats.compressed_events);
println!("  Eventos sin comprimir: {}", stats.uncompressed_events);
```

### Configuración de Compresión

```rust
// Configurar umbrales de compresión
let mut store = Store::new(
    "data", "id", db_manager, None, true
)?;

// Personalizar configuración de compresión
store.set_compression_config(CompressionConfig {
    min_size_threshold: 256,        // Comprimir solo si >= 256 bytes
    min_ratio_threshold: 0.15,      // Solo si comprime >= 15%
    compression_level: CompressionLevel::Balanced, // Balance velocidad/tamaño
})?;
```

## 🔄 Recuperación y Replay

### Recuperación Completa

```rust
// Recuperar estado completo desde la persistencia
let recovered_state = store.recover()?;

match recovered_state {
    Some(state) => {
        println!("Estado recuperado exitosamente");
        println!("Balance: ${}", state.balance);
        println!("Transacciones: {}", state.transaction_count);
    }
    None => {
        println!("No hay datos persistidos, iniciando estado fresco");
    }
}
```

### Replay de Eventos

```rust
// Replay manual de eventos para debugging o migración
let events = store.events(0, u64::MAX)?;
let mut reconstructed_state = BankAccount::default();

println!("Reproduciendo {} eventos:", events.len());

for (index, event) in events.iter().enumerate() {
    println!("  Evento {}: {:?}", index + 1, event);
    reconstructed_state.apply(event);
    println!("    Balance después: ${}", reconstructed_state.balance);
}

println!("Estado final reconstruido: {:?}", reconstructed_state);
```

### Recuperación a Punto Específico

```rust
// Recuperar estado en un punto específico en el tiempo
let events_until_point = store.events(0, 50)?; // Primeros 50 eventos
let mut state_at_point = BankAccount::default();

for event in &events_until_point {
    state_at_point.apply(event);
}

println!("Estado en evento 50: ${}", state_at_point.balance);
```

## 🎯 Patrones Avanzados

### Saga Pattern con Event Sourcing

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SagaEvent {
    SagaStarted { saga_id: String, steps: Vec<String> },
    StepCompleted { saga_id: String, step: String },
    StepFailed { saga_id: String, step: String, error: String },
    SagaCompleted { saga_id: String },
    SagaFailed { saga_id: String, failed_step: String },
    CompensationStarted { saga_id: String },
    CompensationCompleted { saga_id: String },
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct SagaState {
    pub saga_id: String,
    pub status: SagaStatus,
    pub completed_steps: Vec<String>,
    pub failed_step: Option<String>,
    pub compensation_completed: bool,
}

impl PersistentActor for SagaState {
    type Event = SagaEvent;

    fn apply(&mut self, event: &Self::Event) {
        match event {
            SagaEvent::SagaStarted { saga_id, steps: _ } => {
                self.saga_id = saga_id.clone();
                self.status = SagaStatus::InProgress;
                self.completed_steps.clear();
            }
            SagaEvent::StepCompleted { saga_id: _, step } => {
                self.completed_steps.push(step.clone());
            }
            SagaEvent::StepFailed { saga_id: _, step, error: _ } => {
                self.failed_step = Some(step.clone());
                self.status = SagaStatus::Failed;
            }
            SagaEvent::SagaCompleted { saga_id: _ } => {
                self.status = SagaStatus::Completed;
            }
            SagaEvent::CompensationCompleted { saga_id: _ } => {
                self.compensation_completed = true;
                self.status = SagaStatus::Compensated;
            }
            _ => {}
        }
    }
}
```

### CQRS con Event Sourcing

```rust
// Command Model - Para escrituras
pub struct WriteModel {
    store: Store<BankAccount>,
}

impl WriteModel {
    pub async fn handle_deposit(&mut self, account_id: &str, amount: f64) -> Result<(), String> {
        // Cargar estado actual
        let mut account = self.store.recover()?.unwrap_or_default();

        // Validar comando
        if amount <= 0.0 {
            return Err("Amount must be positive".to_string());
        }

        // Generar evento
        let event = BankEvent::MoneyDeposited {
            amount,
            balance: account.balance + amount,
        };

        // Persistir evento
        self.store.persist(&event).map_err(|e| e.to_string())?;

        // Aplicar al estado
        account.apply(&event);

        Ok(())
    }
}

// Query Model - Para lecturas (puede usar proyecciones optimizadas)
pub struct ReadModel {
    store: Store<BankAccount>,
}

impl ReadModel {
    pub async fn get_balance(&self, account_id: &str) -> Result<f64, String> {
        let account = self.store.recover().map_err(|e| e.to_string())?;
        Ok(account.map(|a| a.balance).unwrap_or(0.0))
    }

    pub async fn get_transaction_history(&self, account_id: &str) -> Result<Vec<BankEvent>, String> {
        self.store.events(0, u64::MAX).map_err(|e| e.to_string())
    }
}
```

## 🔧 Configuración de Base de Datos

### SQLite (Recomendado para la mayoría de casos)

```rust
use sqlite_db::SqliteManager;

let db_manager = SqliteManager::new("./data")?;

// Configuración optimizada automáticamente:
// - WAL mode para mejor concurrencia
// - Cache de 40MB
// - Memory-mapped I/O
// - Optimización automática cada 10K queries
```

### RocksDB (Para alto rendimiento)

```rust
use rocksdb_db::RocksDbManager;

let db_manager = RocksDbManager::new("./rocksdb_data")?;

// Características:
// - Rendimiento extremo para escrituras
// - Compresión LZ4 integrada
// - Optimizado para SSDs
// - Ideal para workloads intensivos
```

### Memory (Para testing)

```rust
use rush_store::MemoryManager;

let db_manager = MemoryManager::default();

// Características:
// - Todo en memoria RAM
// - Ideal para testing
// - Sin persistencia entre reinicios
// - Rendimiento máximo
```

## 🧪 Testing

### Tests de Persistencia

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_event_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let mut store = Store::new(
            "test_accounts",
            "test_account",
            db_manager,
            None,
            false,
        ).unwrap();

        let mut account = BankAccount::default();

        // Persistir eventos
        let open_event = BankEvent::AccountOpened {
            account_id: "test_account".to_string(),
            initial_balance: 1000.0,
        };

        store.persist(&open_event).unwrap();
        account.apply(&open_event);

        let deposit_event = BankEvent::MoneyDeposited {
            amount: 500.0,
            balance: 1500.0,
        };

        store.persist(&deposit_event).unwrap();
        account.apply(&deposit_event);

        // Verificar persistencia
        let events = store.events(0, u64::MAX).unwrap();
        assert_eq!(events.len(), 2);

        // Verificar recuperación
        let recovered = store.recover().unwrap().unwrap();
        assert_eq!(recovered.balance, 1500.0);
        assert_eq!(recovered.transaction_count, 1);
    }

    #[tokio::test]
    async fn test_snapshot_optimization() {
        let temp_dir = TempDir::new().unwrap();
        let db_manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let mut store = Store::new(
            "test_accounts",
            "snapshot_test",
            db_manager,
            None,
            false,
        ).unwrap();

        let mut account = BankAccount::default();

        // Generar muchos eventos
        for i in 1..=100 {
            let event = BankEvent::MoneyDeposited {
                amount: 10.0,
                balance: i as f64 * 10.0,
            };
            store.persist(&event).unwrap();
            account.apply(&event);

            // Snapshot cada 25 eventos
            if i % 25 == 0 {
                store.snapshot(&account).unwrap();
            }
        }

        // Verificar recuperación optimizada
        let recovered = store.recover().unwrap().unwrap();
        assert_eq!(recovered.balance, 1000.0);
        assert_eq!(recovered.transaction_count, 100);

        // Verificar que se usa snapshot (recuperación rápida)
        let start = std::time::Instant::now();
        let _recovered_again = store.recover().unwrap().unwrap();
        let recovery_time = start.elapsed();

        // La recuperación desde snapshot debe ser muy rápida
        assert!(recovery_time.as_millis() < 10);
    }
}
```

### Property-based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_deposit_withdraw_invariants(
        initial_balance in 0f64..10000f64,
        deposits in prop::collection::vec(0f64..1000f64, 0..20),
        withdrawals in prop::collection::vec(0f64..500f64, 0..10)
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();
        let mut store = Store::new("test", "account", db_manager, None, false).unwrap();

        let mut account = BankAccount::default();

        // Evento inicial
        let open_event = BankEvent::AccountOpened {
            account_id: "account".to_string(),
            initial_balance,
        };
        store.persist(&open_event).unwrap();
        account.apply(&open_event);

        let mut expected_balance = initial_balance;

        // Aplicar depósitos
        for deposit in deposits {
            if let Ok(event) = account.deposit(deposit) {
                store.persist(&event).unwrap();
                account.apply(&event);
                expected_balance += deposit;
            }
        }

        // Aplicar retiros
        for withdrawal in withdrawals {
            if let Ok(event) = account.withdraw(withdrawal) {
                store.persist(&event).unwrap();
                account.apply(&event);
                expected_balance -= withdrawal;
            }
        }

        // Invariante: balance calculado debe coincidir con el persistido
        prop_assert_eq!(account.balance, expected_balance);

        // Invariante: recuperación debe dar el mismo estado
        let recovered = store.recover().unwrap().unwrap();
        prop_assert_eq!(recovered.balance, expected_balance);
    }
}
```

## 📊 Monitoring y Métricas

### Métricas del Store

```rust
let metrics = store.metrics();

println!("Store Metrics:");
println!("  Eventos persistidos: {}", metrics.events_persisted);
println!("  Snapshots creados: {}", metrics.snapshots_created);
println!("  Tiempo promedio de persistencia: {:?}", metrics.avg_persist_time);
println!("  Espacio total usado: {} bytes", metrics.total_storage_bytes);
println!("  Último evento: {:?}", metrics.last_event_timestamp);
```

### Health Checks

```rust
// Verificar salud del store
let health = store.health_check().await?;

match health.status {
    HealthStatus::Healthy => {
        println!("Store is healthy");
    }
    HealthStatus::Degraded => {
        println!("Store is degraded: {}", health.message);
    }
    HealthStatus::Unhealthy => {
        eprintln!("Store is unhealthy: {}", health.message);
        // Tomar acciones correctivas
    }
}
```

### Tracing y Logging

```rust
use tracing::{info, warn, error, debug, span, Level};

// El store incluye logging estructurado automático
let _span = span!(Level::INFO, "bank_transaction", account_id = "123").entered();

info!("Processing deposit", amount = 500.0);

match store.persist(&deposit_event) {
    Ok(_) => {
        info!("Deposit persisted successfully");
    }
    Err(e) => {
        error!("Failed to persist deposit: {}", e);
        // Error handling...
    }
}
```

## 🎯 Mejores Prácticas

### Diseño de Eventos

```rust
// ✅ Bueno: Eventos inmutables, específicos y completos
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderEvent {
    OrderCreated {
        order_id: String,
        customer_id: String,
        items: Vec<OrderItem>,
        total_amount: f64,
        created_at: DateTime<Utc>,
    },
    PaymentProcessed {
        order_id: String,
        payment_method: PaymentMethod,
        amount: f64,
        transaction_id: String,
        processed_at: DateTime<Utc>,
    },
    OrderShipped {
        order_id: String,
        shipping_address: Address,
        tracking_number: String,
        shipped_at: DateTime<Utc>,
    },
}

// ❌ Malo: Eventos genéricos o con referencias mutables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BadEvent {
    StateChanged(HashMap<String, Value>), // Muy genérico
    ProcessData(Vec<u8>),                 // No descriptivo
}
```

### Gestión de Migración de Eventos

```rust
// Versionado de eventos para evolución del schema
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedBankEvent {
    #[serde(rename = "v1")]
    V1(BankEventV1),
    #[serde(rename = "v2")]
    V2(BankEventV2),
}

// Migración automática de eventos antiguos
impl VersionedBankEvent {
    pub fn migrate_to_latest(self) -> BankEventV2 {
        match self {
            VersionedBankEvent::V1(event_v1) => {
                // Migrar V1 -> V2
                BankEventV2::from_v1(event_v1)
            }
            VersionedBankEvent::V2(event_v2) => event_v2,
        }
    }
}
```

### Patrones de Recuperación

```rust
// Recuperación robusta con fallbacks
impl BankAccountService {
    pub async fn load_account(&self, account_id: &str) -> Result<BankAccount, ServiceError> {
        // 1. Intentar recuperación desde snapshot + eventos
        match self.store.recover() {
            Ok(Some(account)) => {
                info!("Account loaded from snapshot", account_id);
                return Ok(account);
            }
            Ok(None) => {
                info!("No snapshot found, loading from events", account_id);
            }
            Err(e) => {
                warn!("Snapshot recovery failed: {}, falling back to events", e);
            }
        }

        // 2. Fallback: reconstruir desde todos los eventos
        let events = self.store.events(0, u64::MAX)?;
        let mut account = BankAccount::default();

        for event in events {
            account.apply(&event);
        }

        // 3. Crear snapshot para futuras cargas
        if account.transaction_count > 0 {
            if let Err(e) = self.store.snapshot(&account) {
                warn!("Failed to create snapshot after recovery: {}", e);
            }
        }

        Ok(account)
    }
}
```

---

Este módulo de Event Sourcing proporciona una base sólida para construir aplicaciones con persistencia robusta, auditoría completa y recuperación de fallos garantizada. 🗄️📊🔒