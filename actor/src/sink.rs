// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Event Sink System
//!
//! The event sink system provides a comprehensive framework for consuming and processing events
//! published by actors within the system. It implements a robust subscriber pattern that enables
//! decoupled event handling, allowing for clean separation between event producers (actors) and
//! event consumers (sinks). This architecture is essential for implementing cross-cutting concerns
//! such as logging, metrics collection, persistence, audit trails, and integration with external
//! systems.
//!
//! # Architecture Overview
//!
//! The sink system is built on a publisher-subscriber pattern with the following key components:
//!
//! - **Events**: Serializable data structures that represent state changes or significant occurrences
//! - **Publishers**: Actors that emit events via the event bus using broadcast channels
//! - **Sinks**: Containers that connect event streams to subscriber implementations
//! - **Subscribers**: Components that process individual events according to their business logic
//!
//! ## Event Flow
//!
//! ```text
//! Actor (Publisher)
//!     │
//!     │ publish_event()
//!     ▼
//! Event Bus (Broadcast Channel)
//!     │
//!     │ multiple receivers
//!     ▼
//! Sink₁, Sink₂, Sink₃, ...
//!     │
//!     │ notify()
//!     ▼
//! Subscriber₁, Subscriber₂, Subscriber₃, ...
//! ```
//!
//! # Core Capabilities
//!
//! ## Event Processing Patterns
//!
//! - **Fire-and-Forget**: Events are delivered asynchronously without blocking publishers
//! - **Independent Processing**: Each subscriber processes events independently
//! - **Ordered Delivery**: Events are processed in the order they were published
//! - **Backpressure Handling**: Automatic handling of slow consumers through lagging detection
//!
//! ## Error Handling and Resilience
//!
//! - **Isolation**: Subscriber errors don't affect other subscribers or the publishing actor
//! - **Graceful Degradation**: Channel closure is handled cleanly without panics
//! - **Lag Recovery**: Automatic recovery from temporary processing delays
//! - **Fault Tolerance**: Individual sink failures don't impact system stability
//!
//! ## Performance Characteristics
//!
//! - **Low Latency**: Minimal overhead between event publication and subscriber notification
//! - **High Throughput**: Efficient broadcasting to multiple subscribers simultaneously
//! - **Memory Bounded**: Configurable channel capacity prevents unbounded memory growth
//! - **Non-Blocking**: Publishers never block waiting for subscribers to process events
//!
//! # Common Usage Patterns
//!
//! ## Event Logging and Audit
//!
//! ```ignore
//! use rush_actor::*;
//! use async_trait::async_trait;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! pub struct AuditEvent {
//!     pub user_id: String,
//!     pub action: String,
//!     pub timestamp: chrono::DateTime<chrono::Utc>,
//! }
//!
//! impl Event for AuditEvent {}
//!
//! pub struct AuditLogger {
//!     log_file: String,
//! }
//!
//! #[async_trait]
//! impl Subscriber<AuditEvent> for AuditLogger {
//!     async fn notify(&self, event: AuditEvent) {
//!         // Write audit event to secure log file
//!         eprintln!("AUDIT: {} performed {} at {}",
//!             event.user_id, event.action, event.timestamp);
//!     }
//! }
//! ```
//!
//! ## Metrics Collection
//!
//! ```ignore
//! use rush_actor::*;
//! use async_trait::async_trait;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! pub struct MetricEvent {
//!     pub metric_name: String,
//!     pub value: f64,
//!     pub labels: HashMap<String, String>,
//! }
//!
//! impl Event for MetricEvent {}
//!
//! pub struct PrometheusCollector {
//!     client: prometheus::Client,
//! }
//!
//! #[async_trait]
//! impl Subscriber<MetricEvent> for PrometheusCollector {
//!     async fn notify(&self, event: MetricEvent) {
//!         // Send metrics to Prometheus
//!         self.client.record_metric(
//!             &event.metric_name,
//!             event.value,
//!             event.labels
//!         ).await;
//!     }
//! }
//! ```
//!
//! ## Database Persistence
//!
//! ```ignore
//! use rush_actor::*;
//! use async_trait::async_trait;
//!
//! pub struct DatabasePersister {
//!     db_pool: sqlx::PgPool,
//! }
//!
//! #[async_trait]
//! impl<E: Event> Subscriber<E> for DatabasePersister {
//!     async fn notify(&self, event: E) {
//!         // Serialize and store event in database
//!         let json_data = serde_json::to_string(&event).unwrap();
//!         sqlx::query("INSERT INTO events (data) VALUES ($1)")
//!             .bind(json_data)
//!             .execute(&self.db_pool)
//!             .await
//!             .expect("Failed to persist event");
//!     }
//! }
//! ```
//!
//! # Integration with Actor System
//!
//! Sinks integrate seamlessly with the actor system through the `SystemRef::run_sink()` method,
//! which handles the complete lifecycle of event processing in background tasks:
//!
//! ```ignore
//! use rush_actor::*;
//!
//! async fn setup_event_processing(system: &SystemRef) {
//!     // Create actor and subscribe to its events
//!     let actor_ref = system.create_actor("publisher", PublishingActor::new()).await.unwrap();
//!     let event_receiver = actor_ref.subscribe::<MyEvent>();
//!
//!     // Create and run sink for logging
//!     let logging_sink = Sink::new(event_receiver, EventLogger::new());
//!     system.run_sink(logging_sink).await;
//!
//!     // Create and run sink for metrics
//!     let metrics_receiver = actor_ref.subscribe::<MyEvent>();
//!     let metrics_sink = Sink::new(metrics_receiver, MetricsCollector::new());
//!     system.run_sink(metrics_sink).await;
//! }
//! ```
//!
//! # Threading and Concurrency
//!
//! The sink system is designed for high-concurrency environments:
//!
//! - **Thread Safety**: All components implement `Send + Sync + 'static` for safe concurrent access
//! - **Async Processing**: Full async/await support for non-blocking I/O operations
//! - **Parallel Execution**: Multiple sinks can process the same events concurrently
//! - **Isolation**: Each sink runs in its own background task with independent error handling
//!
//! # Best Practices
//!
//! ## Subscriber Implementation
//!
//! 1. **Error Handling**: Always handle errors gracefully within subscribers
//! 2. **Performance**: Keep subscriber processing fast to avoid lagging
//! 3. **Resource Management**: Properly manage external resources (files, connections)
//! 4. **Idempotency**: Design subscribers to handle duplicate events safely
//!
//! ## Event Design
//!
//! 1. **Immutability**: Events should be immutable after creation
//! 2. **Serialization**: Ensure events serialize/deserialize correctly
//! 3. **Versioning**: Plan for event schema evolution over time
//! 4. **Size**: Keep events reasonably small to optimize memory usage
//!

use crate::Event;

use async_trait::async_trait;
use tokio::sync::broadcast::{Receiver as EventReceiver, error::RecvError};

use tracing::debug;

/// Event processing container that connects event streams to subscriber implementations.
///
/// A `Sink` serves as the bridge between the actor system's event broadcasting mechanism and
/// custom event processing logic. It continuously consumes events from a broadcast channel
/// and forwards them to a subscriber implementation for processing. This design enables
/// clean separation of concerns between event production (actors) and event consumption
/// (subscribers), facilitating modular and testable event handling architectures.
///
/// # Type Parameters
///
/// * `E` - The event type that this sink will process. Must implement the `Event` trait,
///   which requires serialization, cloning, and thread safety capabilities.
///
/// # Architecture
///
/// The sink operates on a simple but powerful architecture:
///
/// ```text
/// EventReceiver<E> ───► Sink<E> ───► Subscriber<E>
///        │                │               │
///        │                │               ▼
///    Broadcast         Event Loop     Custom Logic
///     Channel         (run method)   (notify method)
/// ```
///
/// ## Event Processing Loop
///
/// Once started via the `run()` method, the sink enters an infinite loop that:
/// 1. **Receives Events**: Waits for events from the broadcast channel
/// 2. **Forwards Events**: Calls the subscriber's `notify()` method with each event
/// 3. **Handles Errors**: Manages channel closure and lagging conditions gracefully
/// 4. **Logs Activity**: Provides debug information about event processing
///
/// ## Error Handling Strategy
///
/// The sink implements robust error handling for common failure scenarios:
///
/// - **Channel Closure**: Clean shutdown when the event publisher terminates
/// - **Lagging Events**: Automatic recovery when processing falls behind
/// - **Subscriber Errors**: Isolated error handling (errors in subscribers don't crash the sink)
///
/// # Thread Safety and Concurrency
///
/// Sinks are designed for concurrent execution environments:
///
/// - **Single-Threaded Processing**: Each sink processes events sequentially within its own task
/// - **Multiple Sinks**: Multiple sinks can subscribe to the same event stream independently
/// - **Thread-Safe Subscribers**: Subscriber implementations must be `Send + Sync + 'static`
/// - **Non-Blocking**: Event processing doesn't block the publishing actors
///
/// # Performance Characteristics
///
/// - **Memory Efficiency**: Uses boxed trait objects to avoid monomorphization overhead
/// - **Low Latency**: Minimal processing delay between event receipt and subscriber notification
/// - **Backpressure Handling**: Automatic handling of slow consumers through lagging recovery
/// - **Resource Bounded**: Memory usage is bounded by the broadcast channel capacity
///
/// # Lifecycle Management
///
/// Sinks follow a simple lifecycle:
///
/// 1. **Creation**: Instantiated with an event receiver and subscriber
/// 2. **Execution**: Started via `run()` method (typically in a background task)
/// 3. **Processing**: Continuously processes events until channel closure
/// 4. **Termination**: Automatically stops when the event stream ends
///
/// # Common Integration Patterns
///
/// ## Background Task Execution
///
/// ```ignore
/// use rush_actor::*;
/// use tokio;
///
/// async fn run_background_sink<E: Event>(
///     event_receiver: tokio::sync::broadcast::Receiver<E>,
///     subscriber: impl Subscriber<E>,
/// ) {
///     let mut sink = Sink::new(event_receiver, subscriber);
///     tokio::spawn(async move {
///         sink.run().await;
///     });
/// }
/// ```
///
/// ## System Integration
///
/// ```ignore
/// use rush_actor::*;
///
/// async fn setup_system_monitoring(system: &SystemRef) -> Result<(), Error> {
///     let actor_ref = system.create_actor("monitored", MonitoredActor::new()).await?;
///
///     // Create multiple sinks for different purposes
///     let log_receiver = actor_ref.subscribe::<AuditEvent>();
///     let log_sink = Sink::new(log_receiver, FileLogger::new("audit.log"));
///     system.run_sink(log_sink).await;
///
///     let metrics_receiver = actor_ref.subscribe::<MetricEvent>();
///     let metrics_sink = Sink::new(metrics_receiver, PrometheusExporter::new());
///     system.run_sink(metrics_sink).await;
///
///     Ok(())
/// }
/// ```
///
/// ## Testing and Development
///
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use std::sync::{Arc, Mutex};
///
/// // Test subscriber that collects events for verification
/// struct TestCollector<E> {
///     events: Arc<Mutex<Vec<E>>>,
/// }
///
/// #[async_trait]
/// impl<E: Event> Subscriber<E> for TestCollector<E> {
///     async fn notify(&self, event: E) {
///         self.events.lock().unwrap().push(event);
///     }
/// }
///
/// #[tokio::test]
/// async fn test_event_processing() {
///     let collector = TestCollector { events: Arc::new(Mutex::new(Vec::new())) };
///     let (tx, rx) = tokio::sync::broadcast::channel(100);
///
///     let mut sink = Sink::new(rx, collector.clone());
///
///     // Send test events
///     tx.send(TestEvent::new("test")).unwrap();
///
///     // Process events (in real usage, this would run in background)
///     // sink.run().await; // This would run forever, so we'd use timeouts in tests
/// }
/// ```
///
/// # Best Practices
///
/// ## Subscriber Design
/// - Keep subscriber processing fast to avoid lagging
/// - Handle errors gracefully within subscribers
/// - Use appropriate logging levels for debugging
/// - Consider idempotency for duplicate event handling
///
/// ## Resource Management
/// - Ensure subscribers properly clean up resources
/// - Use connection pooling for database subscribers
/// - Implement proper backoff strategies for external service integration
/// - Monitor memory usage in long-running sinks
///
/// ## Error Handling
/// - Log subscriber errors but don't panic
/// - Implement retry logic where appropriate
/// - Use circuit breakers for external service integration
/// - Consider dead letter queues for failed events
pub struct Sink<E: Event> {
    /// The subscriber that will process each received event.
    /// Boxed to allow for different subscriber implementations while maintaining type safety.
    subscriber: Box<dyn Subscriber<E>>,
    /// The broadcast channel receiver for incoming events from actor publishers.
    /// This receiver is cloned from the original channel created by the actor system.
    event_receiver: EventReceiver<E>,
}

impl<E: Event> Sink<E> {
    /// Creates a new sink that connects an event receiver to a subscriber implementation.
    ///
    /// This constructor establishes the fundamental connection between an event stream and
    /// the custom logic that will process each event. The resulting sink is ready for
    /// execution but requires calling `run()` to begin processing events.
    ///
    /// # Parameters
    ///
    /// * `event_receiver` - A broadcast channel receiver that will provide the stream of events.
    ///   This receiver is typically obtained by calling `subscribe()` on an actor reference,
    ///   which creates a new receiver clone from the actor's event broadcast channel. Multiple
    ///   receivers can be created from the same channel, allowing multiple sinks to process
    ///   the same event stream independently.
    ///
    /// * `subscriber` - The implementation that will process each event received. This can be
    ///   any type that implements the `Subscriber<E>` trait, including closures, structs with
    ///   custom logic, or pre-built subscriber implementations. The subscriber will be boxed
    ///   internally to allow for dynamic dispatch and type erasure.
    ///
    /// # Returns
    ///
    /// Returns a new `Sink<E>` instance ready for execution. The sink is in an initialized
    /// state but not yet processing events. Call `run()` to begin the event processing loop.
    ///
    /// # Type Parameters
    ///
    /// The event type `E` is inferred from the receiver and subscriber parameters, ensuring
    /// type safety throughout the event processing pipeline. Both the receiver and subscriber
    /// must handle the same event type.
    ///
    /// # Memory Management
    ///
    /// The subscriber is stored as a boxed trait object (`Box<dyn Subscriber<E>>`), which
    /// provides several benefits:
    /// - **Type Erasure**: Different subscriber implementations can be used interchangeably
    /// - **Dynamic Dispatch**: Method calls are resolved at runtime, enabling flexibility
    /// - **Memory Efficiency**: Avoids code duplication from monomorphization
    /// - **Interface Stability**: Changes to subscriber implementations don't affect sink code
    ///
    /// # Thread Safety
    ///
    /// Both parameters must satisfy the thread safety requirements imposed by the `Subscriber`
    /// trait bounds (`Send + Sync + 'static`). This ensures the sink can be safely moved
    /// between threads and executed in background tasks.
    ///
    /// # Examples
    ///
    /// ## Basic Usage with Struct Subscriber
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    /// use tokio::sync::broadcast;
    ///
    /// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    /// struct LogEvent {
    ///     message: String,
    ///     level: String,
    /// }
    ///
    /// impl Event for LogEvent {}
    ///
    /// struct FileLogger {
    ///     file_path: String,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<LogEvent> for FileLogger {
    ///     async fn notify(&self, event: LogEvent) {
    ///         println!("Logging to {}: {} - {}", self.file_path, event.level, event.message);
    ///     }
    /// }
    ///
    /// // Create sink with struct subscriber
    /// let (_, rx) = broadcast::channel::<LogEvent>(100);
    /// let logger = FileLogger { file_path: "/var/log/app.log".to_string() };
    /// let sink = Sink::new(rx, logger);
    /// ```
    ///
    /// ## Multiple Sinks for Same Event Stream
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio::sync::broadcast;
    ///
    /// async fn setup_multiple_subscribers(
    ///     actor_ref: &ActorRef<EventPublisher>
    /// ) -> Result<(), Error> {
    ///     // Create multiple receivers from the same event stream
    ///     let log_receiver = actor_ref.subscribe::<UserEvent>();
    ///     let metrics_receiver = actor_ref.subscribe::<UserEvent>();
    ///     let audit_receiver = actor_ref.subscribe::<UserEvent>();
    ///
    ///     // Create different sinks for different purposes
    ///     let log_sink = Sink::new(log_receiver, EventLogger::new());
    ///     let metrics_sink = Sink::new(metrics_receiver, MetricsCollector::new());
    ///     let audit_sink = Sink::new(audit_receiver, AuditTracker::new());
    ///
    ///     // Each sink processes the same events independently
    ///     // (run() calls would typically happen in background tasks)
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Integration with System
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// async fn create_system_sink(system: &SystemRef) -> Result<(), Error> {
    ///     // Create actor and get event receiver
    ///     let actor_ref = system.create_actor("publisher", EventActor::new()).await?;
    ///     let receiver = actor_ref.subscribe::<SystemEvent>();
    ///
    ///     // Create sink with custom subscriber
    ///     let sink = Sink::new(receiver, SystemMonitor::new());
    ///
    ///     // Start processing events in background
    ///     system.run_sink(sink).await;
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Testing with Mock Subscribers
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    /// use std::sync::{Arc, Mutex};
    /// use tokio::sync::broadcast;
    ///
    /// struct MockSubscriber {
    ///     received_events: Arc<Mutex<Vec<TestEvent>>>,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<TestEvent> for MockSubscriber {
    ///     async fn notify(&self, event: TestEvent) {
    ///         self.received_events.lock().unwrap().push(event);
    ///     }
    /// }
    ///
    /// #[tokio::test]
    /// async fn test_sink_creation() {
    ///     let (tx, rx) = broadcast::channel(100);
    ///     let mock = MockSubscriber {
    ///         received_events: Arc::new(Mutex::new(Vec::new()))
    ///     };
    ///     let sink = Sink::new(rx, mock);
    ///     // Sink is created and ready for testing
    /// }
    /// ```
    ///
    /// # Performance Considerations
    ///
    /// - **Construction Cost**: Minimal - only involves boxing the subscriber
    /// - **Memory Overhead**: One heap allocation for the boxed subscriber
    /// - **Runtime Cost**: No performance impact during event processing
    /// - **Type Safety**: Compile-time verification of event type compatibility
    ///
    /// # Error Handling
    ///
    /// This method cannot fail - it performs only in-memory operations with valid parameters.
    /// Error handling occurs later during the `run()` phase when actual event processing
    /// begins and network/I/O operations may fail.
    pub fn new(
        event_receiver: EventReceiver<E>,
        subscriber: impl Subscriber<E>,
    ) -> Self {
        Sink {
            subscriber: Box::new(subscriber),
            event_receiver,
        }
    }

    /// Runs the event processing loop, continuously consuming and forwarding events to the subscriber.
    ///
    /// This is the core execution method of the sink that establishes the event processing pipeline.
    /// Once called, it enters an infinite loop that receives events from the broadcast channel and
    /// forwards them to the subscriber for processing. The method handles all aspects of event
    /// lifecycle management, including error recovery, graceful shutdown, and performance monitoring.
    ///
    /// # Behavior
    ///
    /// The method implements a robust event processing loop with the following behavior:
    ///
    /// ## Normal Operation
    /// 1. **Event Reception**: Waits asynchronously for events from the broadcast channel
    /// 2. **Debug Logging**: Records each received event for debugging and monitoring
    /// 3. **Subscriber Notification**: Forwards the event to the subscriber's `notify()` method
    /// 4. **Loop Continuation**: Returns to waiting for the next event
    ///
    /// ## Error Handling
    /// The loop handles two main error conditions from the broadcast channel:
    ///
    /// ### Channel Closure (`RecvError::Closed`)
    /// - **Trigger**: All senders have been dropped, indicating no more events will be published
    /// - **Action**: Clean loop termination - the method returns gracefully
    /// - **Use Case**: Normal shutdown when publishing actors stop or the system shuts down
    ///
    /// ### Channel Lagging (`RecvError::Lagged`)
    /// - **Trigger**: The subscriber is processing events slower than they're being published
    /// - **Action**: Automatic recovery - skips missed events and continues processing
    /// - **Rationale**: Prevents memory exhaustion and maintains system stability
    /// - **Trade-off**: Some events may be lost, but the system remains operational
    ///
    /// # Execution Context
    ///
    /// This method is typically executed in one of these contexts:
    ///
    /// ## Background Task Execution
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio;
    ///
    /// async fn spawn_sink_task<E: Event>(
    ///     mut sink: Sink<E>
    /// ) {
    ///     tokio::spawn(async move {
    ///         sink.run().await; // Runs until channel closes
    ///         println!("Sink terminated gracefully");
    ///     });
    /// }
    /// ```
    ///
    /// ## System Integration
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// async fn setup_system_sink(system: &SystemRef) -> Result<(), Error> {
    ///     let actor_ref = system.create_actor("publisher", MyActor::new()).await?;
    ///     let receiver = actor_ref.subscribe::<MyEvent>();
    ///     let sink = Sink::new(receiver, MySubscriber::new());
    ///
    ///     // This internally calls run() in a background task
    ///     system.run_sink(sink).await;
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// ## Throughput
    /// - **Event Rate**: Can process thousands of events per second depending on subscriber logic
    /// - **Batching**: Processes events one at a time (no automatic batching)
    /// - **Backpressure**: Automatic handling through lagging detection and recovery
    ///
    /// ## Latency
    /// - **Processing Delay**: Minimal overhead - events are forwarded immediately upon receipt
    /// - **Subscriber Impact**: Total latency depends on subscriber processing time
    /// - **Async Benefits**: Non-blocking I/O operations don't block event reception
    ///
    /// ## Memory Usage
    /// - **Channel Buffer**: Memory usage bounded by broadcast channel capacity
    /// - **Event Storage**: Events are not stored - processed and released immediately
    /// - **Lag Recovery**: Prevents unbounded memory growth during temporary slowdowns
    ///
    /// # Error Isolation
    ///
    /// The sink provides strong error isolation guarantees:
    ///
    /// ## Subscriber Error Handling
    /// - **Isolation**: Errors in subscriber logic don't crash the sink
    /// - **Continuation**: Processing continues with the next event after subscriber errors
    /// - **Responsibility**: Subscribers are responsible for their own error handling
    ///
    /// ## System Stability
    /// - **Independence**: Sink failures don't affect publishing actors or other sinks
    /// - **Graceful Degradation**: Lagging sinks are handled without system-wide impact
    /// - **Resource Protection**: Channel closure prevents resource leaks
    ///
    /// # Monitoring and Debugging
    ///
    /// The method provides comprehensive debugging information:
    ///
    /// ## Debug Logging
    /// Each event generates a debug log entry in the format:
    /// ```text
    /// "Received event: MyEvent { data: \"example\" }. Notify to the subscriber."
    /// ```
    ///
    /// ## Performance Monitoring
    /// - **Event Count**: Track total events processed by monitoring debug logs
    /// - **Lag Detection**: Monitor for `RecvError::Lagged` occurrences
    /// - **Processing Time**: Measure time spent in subscriber notifications
    ///
    /// # Termination Conditions
    ///
    /// The method terminates under these conditions:
    ///
    /// 1. **Normal Shutdown**: All event publishers drop their senders
    /// 2. **System Shutdown**: The actor system terminates, closing all channels
    /// 3. **Publisher Failure**: Publishing actors crash or are stopped
    /// 4. **Manual Termination**: The background task running this method is cancelled
    ///
    /// # Thread Safety
    ///
    /// This method requires `&mut self`, ensuring exclusive access during execution.
    /// This design prevents race conditions while allowing the sink to be moved between
    /// threads before execution begins.
    ///
    /// # Examples
    ///
    /// ## Basic Usage
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio::sync::broadcast;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let (tx, rx) = broadcast::channel::<String>(100);
    ///     let mut sink = Sink::new(rx, |event: String| async move {
    ///         println!("Processing: {}", event);
    ///     });
    ///
    ///     // Send some test events
    ///     tx.send("Hello".to_string())?;
    ///     tx.send("World".to_string())?;
    ///
    ///     // Drop sender to close channel
    ///     drop(tx);
    ///
    ///     // This will process both events and then terminate
    ///     sink.run().await;
    ///     println!("Sink finished processing");
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Error Handling and Monitoring
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    /// use std::sync::atomic::{AtomicUsize, Ordering};
    ///
    /// struct MonitoredSubscriber {
    ///     event_count: AtomicUsize,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<String> for MonitoredSubscriber {
    ///     async fn notify(&self, event: String) {
    ///         let count = self.event_count.fetch_add(1, Ordering::Relaxed);
    ///         println!("Event #{}: {}", count + 1, event);
    ///
    ///         // Simulate processing time
    ///         tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    ///     }
    /// }
    ///
    /// async fn run_monitored_sink() {
    ///     let (tx, rx) = tokio::sync::broadcast::channel(1000);
    ///     let subscriber = MonitoredSubscriber {
    ///         event_count: AtomicUsize::new(0),
    ///     };
    ///     let mut sink = Sink::new(rx, subscriber);
    ///
    ///     // Start sink in background
    ///     tokio::spawn(async move {
    ///         sink.run().await;
    ///         println!("Sink processing completed");
    ///     });
    ///
    ///     // Publisher loop
    ///     for i in 0..100 {
    ///         if tx.send(format!("Message {}", i)).is_err() {
    ///             break; // All receivers dropped
    ///         }
    ///         tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// ## Subscriber Performance
    /// - Keep subscriber processing fast to avoid lagging
    /// - Use async I/O operations to prevent blocking
    /// - Implement proper error handling within subscribers
    /// - Consider batching for high-throughput scenarios
    ///
    /// ## Resource Management
    /// - Monitor for excessive lagging events
    /// - Implement circuit breakers for external service calls
    /// - Use connection pooling for database operations
    /// - Set appropriate channel buffer sizes
    ///
    /// ## Error Handling
    /// - Log subscriber errors but don't panic
    /// - Implement retry logic where appropriate
    /// - Use health checks for critical subscribers
    /// - Consider dead letter queues for failed events
    pub async fn run(&mut self) {
        loop {
            match self.event_receiver.recv().await {
                Ok(event) => {
                    debug!(
                        "Received event: {:?}. Notify to the subscriber.",
                        event
                    );
                    self.subscriber.notify(event).await;
                }
                Err(error) => {
                    match error {
                        RecvError::Closed => break,
                        RecvError::Lagged(_) => {
                            // If the receiver is lagging, we should try to catch up
                            // by processing the events that are still in the channel.
                            continue;
                        }
                    }
                }
            }
        }
    }
}

/// Defines the interface for components that process events in the actor system's sink architecture.
///
/// The `Subscriber` trait is the core abstraction for event processing logic in the sink system.
/// It provides a clean, asynchronous interface for handling events published by actors, enabling
/// the implementation of diverse event processing patterns such as logging, metrics collection,
/// persistence, external system integration, and real-time analytics. This trait serves as the
/// foundation for building robust, scalable event-driven applications with the actor framework.
///
/// # Core Philosophy
///
/// The subscriber pattern implemented by this trait follows several key design principles:
///
/// ## Asynchronous Processing
/// All event processing is asynchronous, enabling non-blocking I/O operations and better
/// resource utilization. This allows subscribers to perform complex operations like database
/// writes, network calls, or file I/O without blocking the event processing pipeline.
///
/// ## Error Isolation
/// Subscriber implementations are responsible for their own error handling. Errors within
/// subscriber logic do not propagate to the sink or affect other subscribers, ensuring
/// system stability and fault tolerance.
///
/// ## Thread Safety
/// Subscribers must be safe to share across threads (`Send + Sync + 'static`), enabling
/// concurrent execution and flexible deployment patterns within the actor system.
///
/// ## Event Ownership
/// Events are passed by value to subscribers, providing full ownership and eliminating
/// concerns about shared mutable state or lifetime management.
///
/// # Type Parameters
///
/// * `E` - The event type this subscriber can process. Must implement the `Event` trait,
///   which requires serialization, cloning, debugging, and thread safety capabilities.
///   This constraint ensures events can be safely transmitted across thread boundaries
///   and persisted or logged as needed.
///
/// # Thread Safety Requirements
///
/// The trait requires implementors to satisfy strict thread safety bounds:
///
/// - **`Send`**: Subscribers can be moved between threads, enabling execution in background tasks
/// - **`Sync`**: Subscribers can be accessed concurrently from multiple threads safely
/// - **`'static`**: Subscribers have no lifetime dependencies, allowing flexible storage and execution
///
/// These requirements enable subscribers to be used in various execution contexts:
/// - Background task execution in thread pools
/// - Sharing between multiple sinks processing the same events
/// - Storage in dynamic collections and trait objects
/// - Integration with async runtimes and schedulers
///
/// # Event Processing Patterns
///
/// The subscriber interface supports a wide range of event processing patterns:
///
/// ## Transformation and Enrichment
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// struct EventEnricher {
///     metadata_service: MetadataService,
/// }
///
/// #[async_trait]
/// impl Subscriber<UserEvent> for EventEnricher {
///     async fn notify(&self, mut event: UserEvent) {
///         // Enrich event with additional metadata
///         event.timestamp = chrono::Utc::now();
///         event.source_ip = self.metadata_service.get_client_ip().await;
///
///         // Forward enriched event to downstream processors
///         self.forward_event(event).await;
///     }
/// }
/// ```
///
/// ## Aggregation and Analytics
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
///
/// struct EventAggregator {
///     counters: Arc<Mutex<HashMap<String, u64>>>,
///     window_size: Duration,
/// }
///
/// #[async_trait]
/// impl Subscriber<MetricEvent> for EventAggregator {
///     async fn notify(&self, event: MetricEvent) {
///         let mut counters = self.counters.lock().await;
///         *counters.entry(event.metric_name.clone()).or_insert(0) += 1;
///
///         // Emit aggregated metrics periodically
///         if self.should_emit_metrics().await {
///             self.emit_aggregated_metrics(&counters).await;
///         }
///     }
/// }
/// ```
///
/// ## External System Integration
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// struct WebhookNotifier {
///     http_client: reqwest::Client,
///     webhook_url: String,
/// }
///
/// #[async_trait]
/// impl Subscriber<AlertEvent> for WebhookNotifier {
///     async fn notify(&self, event: AlertEvent) {
///         // Send event to external webhook
///         match self.http_client
///             .post(&self.webhook_url)
///             .json(&event)
///             .send()
///             .await
///         {
///             Ok(response) if response.status().is_success() => {
///                 tracing::info!("Alert sent successfully: {:?}", event.id);
///             }
///             Ok(response) => {
///                 tracing::warn!("Webhook failed with status: {}", response.status());
///             }
///             Err(e) => {
///                 tracing::error!("Failed to send webhook: {}", e);
///             }
///         }
///     }
/// }
/// ```
///
/// # Implementation Patterns
///
/// ## Simple Function-Based Subscribers
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// // Implement subscriber for closure types
/// struct ClosureSubscriber<F, E>
/// where
///     F: Fn(E) -> BoxFuture<'static, ()> + Send + Sync + 'static,
///     E: Event,
/// {
///     func: F,
///     _phantom: std::marker::PhantomData<E>,
/// }
///
/// #[async_trait]
/// impl<F, E> Subscriber<E> for ClosureSubscriber<F, E>
/// where
///     F: Fn(E) -> BoxFuture<'static, ()> + Send + Sync + 'static,
///     E: Event,
/// {
///     async fn notify(&self, event: E) {
///         (self.func)(event).await;
///     }
/// }
/// ```
///
/// ## Stateful Subscribers with Resource Management
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// struct DatabaseSubscriber {
///     pool: sqlx::PgPool,
///     table_name: String,
/// }
///
/// impl DatabaseSubscriber {
///     pub async fn new(database_url: &str, table_name: String) -> Result<Self, sqlx::Error> {
///         let pool = sqlx::PgPool::connect(database_url).await?;
///         Ok(Self { pool, table_name })
///     }
/// }
///
/// #[async_trait]
/// impl<E: Event> Subscriber<E> for DatabaseSubscriber {
///     async fn notify(&self, event: E) {
///         let json_data = match serde_json::to_value(&event) {
///             Ok(data) => data,
///             Err(e) => {
///                 tracing::error!("Failed to serialize event: {}", e);
///                 return;
///             }
///         };
///
///         let query = format!("INSERT INTO {} (data) VALUES ($1)", self.table_name);
///         if let Err(e) = sqlx::query(&query)
///             .bind(json_data)
///             .execute(&self.pool)
///             .await
///         {
///             tracing::error!("Failed to persist event to database: {}", e);
///         }
///     }
/// }
/// ```
///
/// # Error Handling Best Practices
///
/// ## Graceful Error Recovery
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use std::sync::atomic::{AtomicU64, Ordering};
///
/// struct ResilientSubscriber {
///     error_count: AtomicU64,
///     max_errors: u64,
/// }
///
/// #[async_trait]
/// impl Subscriber<ProcessingEvent> for ResilientSubscriber {
///     async fn notify(&self, event: ProcessingEvent) {
///         match self.process_event(event).await {
///             Ok(()) => {
///                 // Reset error count on success
///                 self.error_count.store(0, Ordering::Relaxed);
///             }
///             Err(e) => {
///                 let errors = self.error_count.fetch_add(1, Ordering::Relaxed);
///                 tracing::warn!("Processing error #{}: {}", errors + 1, e);
///
///                 if errors + 1 >= self.max_errors {
///                     tracing::error!("Maximum error threshold reached, subscriber degraded");
///                     // Implement degraded mode or circuit breaker logic
///                 }
///             }
///         }
///     }
/// }
/// ```
///
/// ## Circuit Breaker Pattern
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// #[derive(Debug, Clone)]
/// enum CircuitState {
///     Closed,
///     Open { until: std::time::Instant },
///     HalfOpen,
/// }
///
/// struct CircuitBreakerSubscriber {
///     inner: Box<dyn Subscriber<ServiceEvent>>,
///     state: Arc<RwLock<CircuitState>>,
///     failure_threshold: u32,
///     recovery_timeout: std::time::Duration,
/// }
///
/// #[async_trait]
/// impl Subscriber<ServiceEvent> for CircuitBreakerSubscriber {
///     async fn notify(&self, event: ServiceEvent) {
///         let state = self.state.read().await.clone();
///
///         match state {
///             CircuitState::Open { until } if std::time::Instant::now() < until => {
///                 // Circuit is open, skip processing
///                 tracing::debug!("Circuit breaker open, skipping event");
///                 return;
///             }
///             CircuitState::Open { .. } => {
///                 // Transition to half-open for testing
///                 *self.state.write().await = CircuitState::HalfOpen;
///             }
///             _ => {}
///         }
///
///         // Attempt to process the event
///         match self.try_process_event(event).await {
///             Ok(()) => {
///                 if matches!(state, CircuitState::HalfOpen) {
///                     *self.state.write().await = CircuitState::Closed;
///                 }
///             }
///             Err(e) => {
///                 tracing::error!("Event processing failed: {}", e);
///                 *self.state.write().await = CircuitState::Open {
///                     until: std::time::Instant::now() + self.recovery_timeout,
///                 };
///             }
///         }
///     }
/// }
/// ```
///
/// # Performance Optimization
///
/// ## Batching for High-Throughput
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use tokio::sync::Mutex;
///
/// struct BatchingSubscriber {
///     batch: Arc<Mutex<Vec<LogEvent>>>,
///     batch_size: usize,
///     flush_interval: std::time::Duration,
/// }
///
/// #[async_trait]
/// impl Subscriber<LogEvent> for BatchingSubscriber {
///     async fn notify(&self, event: LogEvent) {
///         let mut batch = self.batch.lock().await;
///         batch.push(event);
///
///         if batch.len() >= self.batch_size {
///             let events = std::mem::take(&mut *batch);
///             drop(batch); // Release lock before async operation
///
///             self.flush_batch(events).await;
///         }
///     }
/// }
/// ```
///
/// # Testing Subscribers
///
/// ## Mock Subscribers for Testing
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use std::sync::{Arc, Mutex};
///
/// #[derive(Clone)]
/// struct TestSubscriber<E: Event> {
///     received: Arc<Mutex<Vec<E>>>,
///     delay: std::time::Duration,
/// }
///
/// impl<E: Event> TestSubscriber<E> {
///     pub fn new() -> Self {
///         Self {
///             received: Arc::new(Mutex::new(Vec::new())),
///             delay: std::time::Duration::from_millis(0),
///         }
///     }
///
///     pub fn with_delay(delay: std::time::Duration) -> Self {
///         Self {
///             received: Arc::new(Mutex::new(Vec::new())),
///             delay,
///         }
///     }
///
///     pub fn get_received(&self) -> Vec<E> {
///         self.received.lock().unwrap().clone()
///     }
/// }
///
/// #[async_trait]
/// impl<E: Event> Subscriber<E> for TestSubscriber<E> {
///     async fn notify(&self, event: E) {
///         if self.delay > std::time::Duration::from_millis(0) {
///             tokio::time::sleep(self.delay).await;
///         }
///         self.received.lock().unwrap().push(event);
///     }
/// }
/// ```
///
/// # Integration with Actor System
///
/// Subscribers integrate seamlessly with the actor system's lifecycle management:
///
/// ```ignore
/// use rush_actor::*;
///
/// async fn setup_comprehensive_monitoring(system: &SystemRef) -> Result<(), Error> {
///     let actor_ref = system.create_actor("monitored-service", ServiceActor::new()).await?;
///
///     // Set up multiple subscribers for different event types
///     let audit_receiver = actor_ref.subscribe::<AuditEvent>();
///     let audit_sink = Sink::new(audit_receiver, AuditLogger::new("audit.log"));
///     system.run_sink(audit_sink).await;
///
///     let metric_receiver = actor_ref.subscribe::<MetricEvent>();
///     let metric_sink = Sink::new(metric_receiver, PrometheusExporter::new());
///     system.run_sink(metric_sink).await;
///
///     let alert_receiver = actor_ref.subscribe::<AlertEvent>();
///     let alert_sink = Sink::new(alert_receiver, SlackNotifier::new());
///     system.run_sink(alert_sink).await;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait Subscriber<E: Event>: Send + Sync + 'static {
    /// Processes a single event received from the actor system's event stream.
    ///
    /// This is the core method of the `Subscriber` trait, called by the sink whenever a new event
    /// is received from the broadcast channel. Implementations of this method contain the custom
    /// logic for processing events, whether that involves logging, persistence, metrics collection,
    /// external system integration, or any other event-driven functionality.
    ///
    /// # Parameters
    ///
    /// * `event` - The event to be processed, passed by value to provide full ownership to the
    ///   subscriber. This eliminates lifetime concerns and enables flexible processing patterns
    ///   including event transformation, forwarding, or long-term storage.
    ///
    /// # Event Processing Contract
    ///
    /// The method operates under a specific contract that defines the expectations and guarantees
    /// of the event processing system:
    ///
    /// ## Ownership and Lifecycle
    /// - **Event Ownership**: The subscriber receives full ownership of the event
    /// - **Processing Responsibility**: The subscriber is solely responsible for event processing
    /// - **No Return Value**: Events are processed for side effects, not for return values
    /// - **Async Processing**: All processing occurs asynchronously, enabling non-blocking I/O
    ///
    /// ## Error Handling Responsibility
    /// - **Internal Error Management**: Subscribers must handle their own errors gracefully
    /// - **No Error Propagation**: Errors should not propagate to the sink or other subscribers
    /// - **Resilience**: Processing failures should not crash the subscriber or system
    /// - **Logging**: Errors should be logged appropriately for debugging and monitoring
    ///
    /// ## Performance Expectations
    /// - **Non-Blocking**: Should not block the event processing pipeline for extended periods
    /// - **Efficient Processing**: Should process events as quickly as reasonably possible
    /// - **Resource Management**: Should manage external resources (connections, files) properly
    /// - **Memory Usage**: Should not accumulate unbounded memory usage over time
    ///
    /// # Implementation Patterns
    ///
    /// ## Simple Processing
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    ///
    /// struct SimpleLogger;
    ///
    /// #[async_trait]
    /// impl Subscriber<UserAction> for SimpleLogger {
    ///     async fn notify(&self, event: UserAction) {
    ///         println!("[{}] User {} performed: {}",
    ///             chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
    ///             event.user_id,
    ///             event.action
    ///         );
    ///     }
    /// }
    /// ```
    ///
    /// ## Database Persistence with Error Handling
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    ///
    /// struct EventPersister {
    ///     db_pool: sqlx::PgPool,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<OrderEvent> for EventPersister {
    ///     async fn notify(&self, event: OrderEvent) {
    ///         match sqlx::query!(
    ///             "INSERT INTO order_events (order_id, event_type, data) VALUES ($1, $2, $3)",
    ///             event.order_id,
    ///             event.event_type.as_str(),
    ///             serde_json::to_string(&event).unwrap_or_default()
    ///         )
    ///         .execute(&self.db_pool)
    ///         .await
    ///         {
    ///             Ok(_) => {
    ///                 tracing::debug!("Successfully persisted event for order {}", event.order_id);
    ///             }
    ///             Err(e) => {
    ///                 tracing::error!(
    ///                     "Failed to persist event for order {}: {}",
    ///                     event.order_id,
    ///                     e
    ///                 );
    ///                 // Could implement retry logic, dead letter queue, etc.
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## External Service Integration
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    ///
    /// struct WebhookNotifier {
    ///     client: reqwest::Client,
    ///     webhook_url: String,
    ///     timeout: std::time::Duration,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<PaymentEvent> for WebhookNotifier {
    ///     async fn notify(&self, event: PaymentEvent) {
    ///         let payload = match serde_json::to_value(&event) {
    ///             Ok(json) => json,
    ///             Err(e) => {
    ///                 tracing::error!("Failed to serialize payment event: {}", e);
    ///                 return;
    ///             }
    ///         };
    ///
    ///         match tokio::time::timeout(
    ///             self.timeout,
    ///             self.client
    ///                 .post(&self.webhook_url)
    ///                 .json(&payload)
    ///                 .send()
    ///         ).await
    ///         {
    ///             Ok(Ok(response)) if response.status().is_success() => {
    ///                 tracing::info!("Webhook delivered successfully for payment {}", event.payment_id);
    ///             }
    ///             Ok(Ok(response)) => {
    ///                 tracing::warn!(
    ///                     "Webhook failed with status {} for payment {}",
    ///                     response.status(),
    ///                     event.payment_id
    ///                 );
    ///             }
    ///             Ok(Err(e)) => {
    ///                 tracing::error!("Webhook request failed for payment {}: {}", event.payment_id, e);
    ///             }
    ///             Err(_) => {
    ///                 tracing::error!("Webhook timeout for payment {}", event.payment_id);
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Retry with Exponential Backoff
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    /// use std::time::Duration;
    ///
    /// struct RetryingSubscriber {
    ///     max_retries: usize,
    ///     base_delay: Duration,
    /// }
    ///
    /// #[async_trait]
    /// impl Subscriber<CriticalEvent> for RetryingSubscriber {
    ///     async fn notify(&self, event: CriticalEvent) {
    ///         let mut attempt = 0;
    ///         let mut delay = self.base_delay;
    ///
    ///         loop {
    ///             match self.try_process_event(&event).await {
    ///                 Ok(()) => {
    ///                     if attempt > 0 {
    ///                         tracing::info!(
    ///                             "Successfully processed event after {} retries",
    ///                             attempt
    ///                         );
    ///                     }
    ///                     return;
    ///                 }
    ///                 Err(e) => {
    ///                     attempt += 1;
    ///                     if attempt > self.max_retries {
    ///                         tracing::error!(
    ///                             "Failed to process event after {} attempts: {}",
    ///                             self.max_retries,
    ///                             e
    ///                         );
    ///                         return;
    ///                     }
    ///
    ///                     tracing::warn!(
    ///                         "Processing attempt {} failed, retrying in {:?}: {}",
    ///                         attempt,
    ///                         delay,
    ///                         e
    ///                     );
    ///                     tokio::time::sleep(delay).await;
    ///                     delay *= 2; // Exponential backoff
    ///                 }
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// 1. **Error Handling**: Always handle errors gracefully within the method
    /// 2. **Performance**: Keep processing fast to avoid pipeline backups
    /// 3. **Resource Management**: Properly manage external resources and connections
    /// 4. **Logging**: Use appropriate log levels for monitoring and debugging
    /// 5. **Testing**: Design subscribers to be easily testable with clear interfaces
    /// 6. **Idempotency**: Consider making event processing idempotent when possible
    /// 7. **Monitoring**: Include metrics and health checks for production deployments
    /// 8. **Timeouts**: Use timeouts for external service calls to prevent hanging
    async fn notify(&self, event: E);
}
