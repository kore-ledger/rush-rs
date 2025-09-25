# Rush Actor System

[![Rust](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://www.rust-lang.org)
[![Version](https://img.shields.io/badge/version-0.6.5-green.svg)](#)

**Actor System** es el corazón de Rush-rs, implementando un sistema robusto de actores concurrentes basado en el modelo actor de Carl Hewitt, inspirado en Akka y Erlang/OTP.

## 🎭 Características Principales

### Sistema de Actores Completo
- **Actores concurrentes** con aislamiento total de estado
- **Jerarquía de actores** con supervisión automática
- **Ciclo de vida gestionado** (creación, ejecución, detención)
- **Comunicación tell/ask** asíncrona
- **Event bus integrado** para comunicación desacoplada

### Comunicación Robusta
- **Tell**: Envío asíncrono sin respuesta (fire-and-forget)
- **Ask**: Envío con respuesta garantizada
- **Event publishing/subscribing** para comunicación pub/sub
- **Rate limiting** para prevención de message flooding

### Gestión de Estado
- **Aislamiento**: Cada actor maneja su propio estado privado
- **Thread-safe**: Sin necesidad de mutexes en código de usuario
- **Memory safe**: 100% safe Rust, sin código unsafe

## 🏗️ Arquitectura

```
ActorSystem
├── SystemRunner          # Ejecutor del sistema
├── ActorRef<T>          # Referencia tipada a un actor
├── ActorContext<T>      # Contexto de ejecución del actor
├── EventBus            # Bus de eventos sistema
└── Hierarchy           # Jerarquía de actores padre-hijo
    ├── RootActors      # Actores raíz
    └── ChildActors     # Actores hijo con supervisión
```

## 🚀 Inicio Rápido

### Definir un Actor

```rust
use rush_actor::*;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;

// Mensaje que recibe el actor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CalculatorMessage {
    Add(i32),
    Subtract(i32),
    Multiply(i32),
    Divide(i32),
    GetResult,
}

// Respuesta del actor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CalculatorResponse {
    Result(i32),
    Error(String),
}

// Estado del actor
#[derive(Default)]
pub struct CalculatorActor {
    result: i32,
}

// Implementar el trait Actor
#[async_trait]
impl Actor for CalculatorActor {
    type Message = CalculatorMessage;
    type Response = CalculatorResponse;
}

// Implementar el manejador de mensajes
#[async_trait]
impl Handler<CalculatorActor> for CalculatorActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        match msg {
            CalculatorMessage::Add(value) => {
                self.result += value;
                Ok(CalculatorResponse::Result(self.result))
            }
            CalculatorMessage::Subtract(value) => {
                self.result -= value;
                Ok(CalculatorResponse::Result(self.result))
            }
            CalculatorMessage::Multiply(value) => {
                self.result *= value;
                Ok(CalculatorResponse::Result(self.result))
            }
            CalculatorMessage::Divide(value) => {
                if value == 0 {
                    Ok(CalculatorResponse::Error("Division by zero".to_string()))
                } else {
                    self.result /= value;
                    Ok(CalculatorResponse::Result(self.result))
                }
            }
            CalculatorMessage::GetResult => {
                Ok(CalculatorResponse::Result(self.result))
            }
        }
    }
}
```

### Usar el Sistema de Actores

```rust
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Crear sistema de actores
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());

    // 2. Ejecutar sistema en background
    tokio::spawn(async move {
        runner.run().await;
    });

    // 3. Crear actor calculadora
    let calculator = system
        .create_root_actor("calculator", CalculatorActor::default())
        .await?;

    // 4. Usar tell (sin respuesta)
    calculator.tell(CalculatorMessage::Add(10)).await?;
    calculator.tell(CalculatorMessage::Multiply(5)).await?;

    // 5. Usar ask (con respuesta)
    let response = calculator.ask(CalculatorMessage::GetResult).await?;
    match response {
        CalculatorResponse::Result(value) => {
            println!("Resultado: {}", value); // Output: 50
        }
        CalculatorResponse::Error(err) => {
            println!("Error: {}", err);
        }
    }

    // 6. Limpiar recursos
    calculator.ask_stop().await?;

    Ok(())
}
```

## 📡 Comunicación Entre Actores

### Tell - Envío Asíncrono

```rust
// Tell no bloquea y no devuelve respuesta
actor_ref.tell(MyMessage::DoSomething).await?;

// Útil para:
// - Notificaciones
// - Comandos sin respuesta requerida
// - Máximo rendimiento
```

### Ask - Envío con Respuesta

```rust
// Ask espera una respuesta del actor
let response = actor_ref.ask(MyMessage::Calculate(42)).await?;

// Útil para:
// - Consultas que requieren respuesta
// - Operaciones síncronas
// - Validación de resultados
```

### Event Bus - Pub/Sub

```rust
#[async_trait]
impl Handler<MyActor> for MyActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        // Publicar evento
        ctx.publish_event(MyEvent::SomethingHappened { data: 42 }).await?;

        Ok(MyResponse::Success)
    }
}

// En otro actor, suscribirse a eventos
let mut event_receiver = actor_ref.subscribe();

tokio::spawn(async move {
    while let Ok(event) = event_receiver.recv().await {
        println!("Recibido evento: {:?}", event);
    }
});
```

## 👨‍👩‍👧‍👦 Jerarquía de Actores

### Crear Actores Hijo

```rust
#[async_trait]
impl Handler<ParentActor> for ParentActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        match msg {
            ParentMessage::CreateChild(name) => {
                // Crear actor hijo
                let child = ctx.create_child(&name, ChildActor::default()).await?;

                // Almacenar referencia si es necesario
                self.children.insert(name, child);

                Ok(ParentResponse::ChildCreated)
            }
            ParentMessage::SendToChild(name, child_msg) => {
                if let Some(child) = self.children.get(&name) {
                    child.tell(child_msg).await?;
                    Ok(ParentResponse::MessageSent)
                } else {
                    Ok(ParentResponse::ChildNotFound)
                }
            }
        }
    }
}
```

### Acceder a Actores en la Jerarquía

```rust
// Obtener actor hijo por nombre
let child = ctx.get_child::<ChildActor>("child_name").await?;

// Obtener actor padre
if let Some(parent) = ctx.parent::<ParentActor>().await {
    parent.tell(MessageToParent::Update).await?;
}

// Obtener actor por path absoluto
let actor = system.get_actor::<SomeActor>(&ActorPath::from("/user/parent/child")).await?;
```

## 🛡️ Manejo de Errores y Supervisión

### Errores en Actores

```rust
#[async_trait]
impl Handler<MyActor> for MyActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        match msg {
            MyMessage::RiskyOperation => {
                // Manejo de errores robusto
                match self.risky_operation() {
                    Ok(result) => Ok(MyResponse::Success(result)),
                    Err(e) => {
                        // Log del error
                        tracing::error!("Operación falló: {}", e);

                        // Retornar error tipado
                        Err(Error::Functional(format!("Operation failed: {}", e)))
                    }
                }
            }
        }
    }
}
```

### Supervisión de Actores Hijo

```rust
// Los actores padre supervisan automáticamente a sus hijos
// Si un actor hijo falla, el padre puede:
// 1. Recibir notificación del fallo
// 2. Decidir si reiniciar el hijo
// 3. Escalar el error si es necesario
```

## ⚡ Optimizaciones de Rendimiento

### Rate Limiting

```rust
// El sistema incluye protección automática contra message flooding
// Configuración por defecto:
// - Máximo 1000 mensajes por segundo por actor
// - Buffer de mensajes limitado
// - Backpressure automático
```

### Batch Processing

```rust
// Para procesar múltiples mensajes eficientemente
#[async_trait]
impl Handler<BatchActor> for BatchActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        match msg {
            BatchMessage::AddItem(item) => {
                self.buffer.push(item);

                // Procesar en lotes de 100
                if self.buffer.len() >= 100 {
                    self.process_batch().await?;
                    self.buffer.clear();
                }

                Ok(BatchResponse::ItemAdded)
            }
        }
    }
}
```

## 🔍 Debugging y Monitoring

### Logging Estructurado

```rust
use tracing::{info, warn, error, debug};

#[async_trait]
impl Handler<MyActor> for MyActor {
    async fn handle_message(
        &mut self,
        sender: ActorPath,
        msg: Self::Message,
        ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        // Log entrada
        debug!("Actor {} recibió mensaje de {}: {:?}",
               ctx.actor_path(), sender, msg);

        let result = self.process_message(msg);

        match &result {
            Ok(response) => {
                info!("Procesamiento exitoso: {:?}", response);
            }
            Err(error) => {
                error!("Error procesando mensaje: {}", error);
            }
        }

        result
    }
}
```

### Métricas del Sistema

```rust
// Estadísticas disponibles del sistema
let stats = system.stats();
println!("Actores activos: {}", stats.active_actors);
println!("Mensajes procesados: {}", stats.messages_processed);
println!("Errores: {}", stats.errors);
```

## 🧪 Testing de Actores

### Tests Unitarios

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn test_calculator_actor() {
        // Configurar sistema de test
        let (system, mut runner) = ActorSystem::create(CancellationToken::new());

        tokio::spawn(async move {
            runner.run().await;
        });

        // Crear actor
        let calculator = system
            .create_root_actor("test_calc", CalculatorActor::default())
            .await
            .unwrap();

        // Probar operaciones
        calculator.tell(CalculatorMessage::Add(10)).await.unwrap();
        calculator.tell(CalculatorMessage::Multiply(5)).await.unwrap();

        let response = calculator.ask(CalculatorMessage::GetResult).await.unwrap();

        match response {
            CalculatorResponse::Result(value) => {
                assert_eq!(value, 50);
            }
            _ => panic!("Respuesta inesperada"),
        }

        // Limpiar
        calculator.ask_stop().await.unwrap();
    }
}
```

### Tests de Integración

```rust
#[tokio::test]
async fn test_actor_hierarchy() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());

    tokio::spawn(async move {
        runner.run().await;
    });

    // Crear jerarquía de actores
    let parent = system
        .create_root_actor("parent", ParentActor::default())
        .await
        .unwrap();

    parent.tell(ParentMessage::CreateChild("child1".to_string())).await.unwrap();
    parent.tell(ParentMessage::CreateChild("child2".to_string())).await.unwrap();

    // Verificar que los hijos fueron creados
    let child1 = system
        .get_actor::<ChildActor>(&ActorPath::from("/user/parent/child1"))
        .await
        .unwrap();

    let response = child1.ask(ChildMessage::GetStatus).await.unwrap();
    assert_eq!(response, ChildResponse::Active);
}
```

## 📊 Patrones Avanzados

### Actor Pool

```rust
pub struct WorkerPool {
    workers: Vec<ActorRef<WorkerActor>>,
    current_worker: usize,
}

impl WorkerPool {
    pub async fn new(
        system: &ActorSystem,
        pool_size: usize,
    ) -> Result<Self, Error> {
        let mut workers = Vec::new();

        for i in 0..pool_size {
            let worker = system
                .create_root_actor(&format!("worker_{}", i), WorkerActor::default())
                .await?;
            workers.push(worker);
        }

        Ok(WorkerPool {
            workers,
            current_worker: 0,
        })
    }

    pub async fn submit_work(&mut self, work: WorkItem) -> Result<WorkResult, Error> {
        // Round-robin distribution
        let worker = &self.workers[self.current_worker];
        self.current_worker = (self.current_worker + 1) % self.workers.len();

        worker.ask(WorkerMessage::Process(work)).await
    }
}
```

### Circuit Breaker Pattern

```rust
pub struct CircuitBreakerActor {
    state: CircuitState,
    failure_count: u32,
    failure_threshold: u32,
    timeout: Duration,
    last_failure_time: Option<Instant>,
}

#[derive(Debug)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[async_trait]
impl Handler<CircuitBreakerActor> for CircuitBreakerActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: Self::Message,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<Self::Response, Error> {
        match msg {
            CircuitMessage::Execute(operation) => {
                match self.state {
                    CircuitState::Open => {
                        if let Some(last_failure) = self.last_failure_time {
                            if last_failure.elapsed() > self.timeout {
                                self.state = CircuitState::HalfOpen;
                            } else {
                                return Ok(CircuitResponse::CircuitOpen);
                            }
                        }
                    }
                    _ => {}
                }

                match self.execute_operation(operation).await {
                    Ok(result) => {
                        if matches!(self.state, CircuitState::HalfOpen) {
                            self.state = CircuitState::Closed;
                            self.failure_count = 0;
                        }
                        Ok(CircuitResponse::Success(result))
                    }
                    Err(e) => {
                        self.failure_count += 1;
                        if self.failure_count >= self.failure_threshold {
                            self.state = CircuitState::Open;
                            self.last_failure_time = Some(Instant::now());
                        }
                        Ok(CircuitResponse::Failure(e))
                    }
                }
            }
        }
    }
}
```

## 🔧 Configuración Avanzada

### Configuración del Sistema

```rust
use std::time::Duration;

let config = ActorSystemConfig {
    default_mailbox_size: 1000,
    actor_spawn_timeout: Duration::from_secs(30),
    message_timeout: Duration::from_secs(5),
    enable_metrics: true,
    log_level: LogLevel::Info,
};

let (system, runner) = ActorSystem::with_config(
    CancellationToken::new(),
    config
);
```

### Hooks del Sistema

```rust
// Hook ejecutado cuando un actor se crea
system.on_actor_created(|actor_path, actor_type| {
    println!("Actor creado: {} ({})", actor_path, actor_type);
});

// Hook ejecutado cuando un actor falla
system.on_actor_failed(|actor_path, error| {
    eprintln!("Actor falló: {} - {}", actor_path, error);
});

// Hook ejecutado cuando un actor se detiene
system.on_actor_stopped(|actor_path| {
    println!("Actor detenido: {}", actor_path);
});
```

## 🎯 Mejores Prácticas

### Diseño de Mensajes

```rust
// ✅ Bueno: Mensajes inmutables y serializables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserMessage {
    CreateUser { name: String, email: String },
    UpdateEmail { user_id: u64, new_email: String },
    DeleteUser { user_id: u64 },
}

// ❌ Malo: Referencias, punteros, o estado mutable
pub enum BadMessage<'a> {
    ProcessData(&'a mut Vec<u8>), // No serializable
    Callback(Box<dyn Fn()>),      // No serializable
}
```

### Gestión de Estado

```rust
// ✅ Bueno: Estado privado del actor
pub struct UserActor {
    users: HashMap<u64, User>,
    next_id: u64,
}

// ❌ Malo: Estado compartido entre actores
pub struct BadActor {
    shared_state: Arc<Mutex<HashMap<u64, User>>>, // Anti-pattern
}
```

### Manejo de Errores

```rust
// ✅ Bueno: Errores específicos y recuperables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserError {
    UserNotFound(u64),
    InvalidEmail(String),
    DuplicateUser(String),
}

// Convertir a Error del sistema
impl From<UserError> for Error {
    fn from(err: UserError) -> Self {
        Error::Functional(format!("User error: {:?}", err))
    }
}
```

## 📚 API Reference

### Core Traits

- **`Actor`** - Trait principal que define un actor
- **`Handler<A>`** - Trait para manejar mensajes de tipo específico
- **`Message`** - Trait que deben implementar todos los mensajes
- **`Response`** - Trait que deben implementar todas las respuestas

### Core Types

- **`ActorSystem`** - Sistema de actores principal
- **`ActorRef<A>`** - Referencia tipada a un actor
- **`ActorContext<A>`** - Contexto de ejecución del actor
- **`ActorPath`** - Path jerárquico de un actor
- **`Error`** - Tipo de error del sistema

### Funciones de Comunicación

- **`tell(msg)`** - Envío asíncrono sin respuesta
- **`ask(msg)`** - Envío con respuesta esperada
- **`publish_event(event)`** - Publicar evento al bus
- **`subscribe()`** - Suscribirse a eventos del actor

---

Este sistema de actores proporciona una base sólida para construir aplicaciones concurrentes robustas y escalables en Rust. 🎭🦀