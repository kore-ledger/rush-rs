// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Rush Actor System
//!
//! A high-performance, type-safe actor model implementation for building distributed, concurrent,
//! and fault-tolerant systems in Rust. This library provides a comprehensive actor runtime with
//! sophisticated supervision, event handling, and messaging capabilities designed for production
//! workloads.
//!
//! ## Overview
//!
//! The actor model is a mathematical model of concurrent computation that treats "actors" as the
//! fundamental units of computation. Originally formulated by Carl Hewitt in 1973, the model
//! provides an elegant solution to the challenges of concurrent programming by eliminating
//! shared mutable state and relying exclusively on message passing for communication.
//!
//! In response to a message, an actor can:
//! - Make local decisions based on its current state
//! - Update its private internal state
//! - Create child actors with supervision relationships
//! - Send messages to other actors (including itself)
//! - Publish events to the system event bus
//! - Determine its behavior for the next message
//!
//! ## Core Architecture
//!
//! This actor system is built on several foundational components that work together to provide
//! a robust concurrent runtime:
//!
//! ### Actor Hierarchy and Supervision
//!
//! All actors exist within a supervised tree structure where parent actors manage the lifecycle
//! of their children. This hierarchical organization provides:
//!
//! - **Fault Isolation**: Failures in child actors don't propagate to parents
//! - **Resource Management**: Automatic cleanup when actors terminate
//! - **Supervision Strategies**: Configurable failure handling and recovery policies
//! - **System Organization**: Clear ownership and responsibility boundaries
//!
//! ### Message Processing
//!
//! Actors communicate exclusively through typed message passing, ensuring:
//!
//! - **Type Safety**: Compile-time guarantees about message handling
//! - **Isolation**: No shared mutable state between actors
//! - **Asynchronous Processing**: Non-blocking message delivery and handling
//! - **Backpressure**: Built-in flow control mechanisms
//!
//! ### Event System
//!
//! A publish-subscribe event system enables:
//!
//! - **Loose Coupling**: Actors can publish events without knowing consumers
//! - **Cross-cutting Concerns**: Logging, metrics, auditing, and monitoring
//! - **Integration Points**: External system integration through event sinks
//! - **System Observability**: Real-time insight into system behavior
//!
//! ## Getting Started
//!
//! ### Basic Actor Implementation
//!
//! ```ignore
//! use actor::{Actor, ActorContext, ActorRef, Message, Handler, Response};
//! use async_trait::async_trait;
//! use serde::{Deserialize, Serialize};
//!
//! // Define your actor's state
//! struct Counter {
//!     value: u64,
//! }
//!
//! // Define messages your actor can handle
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct Increment(u64);
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct GetValue;
//!
//! // Implement the Actor trait
//! #[async_trait]
//! impl Actor for Counter {
//!     async fn pre_start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         println!("Counter actor starting");
//!         Ok(())
//!     }
//!
//!     async fn post_stop(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         println!("Counter actor stopping");
//!         Ok(())
//!     }
//! }
//!
//! // Implement message handlers
//! #[async_trait]
//! impl Handler<Increment> for Counter {
//!     async fn handle(
//!         &mut self,
//!         msg: Increment,
//!         _ctx: &mut ActorContext<Self>,
//!     ) -> Response<Increment> {
//!         self.value += msg.0;
//!         Response::empty()
//!     }
//! }
//!
//! #[async_trait]
//! impl Handler<GetValue> for Counter {
//!     async fn handle(
//!         &mut self,
//!         _msg: GetValue,
//!         _ctx: &mut ActorContext<Self>,
//!     ) -> Response<GetValue> {
//!         Response::reply(self.value)
//!     }
//! }
//! ```
//!
//! ### Creating and Running the System
//!
//! ```ignore
//! use actor::{ActorSystem, ActorRef};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create the actor system
//!     let (system, mut runner) = ActorSystem::create();
//!
//!     // Start the system runner in a separate task
//!     let system_task = tokio::spawn(async move {
//!         runner.run().await;
//!     });
//!
//!     // Create a counter actor
//!     let counter_ref: ActorRef<Counter> = system
//!         .create_root_actor("counter", Counter { value: 0 })
//!         .await?;
//!
//!     // Send messages to the actor
//!     counter_ref.tell(Increment(5)).await?;
//!     counter_ref.tell(Increment(3)).await?;
//!
//!     // Query the current value
//!     let current_value: u64 = counter_ref.ask(GetValue).await?;
//!     println!("Current value: {}", current_value); // Prints: Current value: 8
//!
//!     // Shutdown the system
//!     system.shutdown().await;
//!     system_task.await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Advanced Features
//!
//! ### Child Actor Management
//!
//! Actors can create and manage child actors, forming supervision hierarchies:
//!
//! ```ignore
//! use actor::{ActorContext, supervision::SupervisionStrategy};
//!
//! #[async_trait]
//! impl Handler<CreateChild> for ParentActor {
//!     async fn handle(
//!         &mut self,
//!         msg: CreateChild,
//!         ctx: &mut ActorContext<Self>,
//!     ) -> Response<CreateChild> {
//!         // Create a child actor with supervision
//!         let child_ref = ctx.create_child(
//!             &msg.name,
//!             ChildActor::new(),
//!             SupervisionStrategy::default(),
//!         ).await?;
//!
//!         self.children.push(child_ref);
//!         Response::empty()
//!     }
//! }
//! ```
//!
//! ### Event Publishing and Subscription
//!
//! Actors can publish events and external components can subscribe to them:
//!
//! ```ignore
//! use actor::{Event, Sink, Subscriber};
//!
//! // Define an event
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct CounterChanged {
//!     old_value: u64,
//!     new_value: u64,
//! }
//!
//! // Publish events from actors
//! impl Handler<Increment> for Counter {
//!     async fn handle(
//!         &mut self,
//!         msg: Increment,
//!         ctx: &mut ActorContext<Self>,
//!     ) -> Response<Increment> {
//!         let old_value = self.value;
//!         self.value += msg.0;
//!
//!         // Publish an event
//!         ctx.publish_event(CounterChanged {
//!             old_value,
//!             new_value: self.value,
//!         }).await?;
//!
//!         Response::empty()
//!     }
//! }
//!
//! // Create event subscribers
//! struct CounterLogger;
//!
//! #[async_trait]
//! impl Subscriber<CounterChanged> for CounterLogger {
//!     async fn notify(&mut self, event: CounterChanged) -> Result<(), actor::Error> {
//!         println!("Counter changed: {} -> {}", event.old_value, event.new_value);
//!         Ok(())
//!     }
//! }
//!
//! // Connect subscribers to the event stream
//! let sink = Sink::new(system.get_event_receiver());
//! sink.subscribe(CounterLogger).await?;
//! ```
//!
//! ### Fault Tolerance and Supervision
//!
//! Configure sophisticated supervision strategies for robust error handling:
//!
//! ```ignore
//! use actor::supervision::{
//!     SupervisionStrategy, Strategy, FixedIntervalStrategy, CustomIntervalStrategy
//! };
//! use std::time::Duration;
//!
//! // Immediate restart on failure
//! let restart_strategy = SupervisionStrategy::Retry(
//!     Strategy::NoInterval(3) // Maximum 3 immediate retries
//! );
//!
//! // Fixed interval retries
//! let fixed_strategy = SupervisionStrategy::Retry(
//!     Strategy::FixedInterval(
//!         FixedIntervalStrategy::new(5, Duration::from_secs(2))
//!     )
//! );
//!
//! // Custom backoff strategy (exponential backoff)
//! let backoff_delays = vec![
//!     Duration::from_millis(100),
//!     Duration::from_millis(200),
//!     Duration::from_millis(400),
//!     Duration::from_millis(800),
//! ];
//! let custom_strategy = SupervisionStrategy::Retry(
//!     Strategy::CustomInterval(
//!         CustomIntervalStrategy::new(backoff_delays)
//!     )
//! );
//! ```
//!
//! ## Integration Patterns
//!
//! ### External System Integration
//!
//! Integrate with external systems using event sinks and message bridges:
//!
//! ```ignore
//! // Database integration through events
//! struct DatabaseSink {
//!     connection: DatabaseConnection,
//! }
//!
//! #[async_trait]
//! impl Subscriber<UserCreated> for DatabaseSink {
//!     async fn notify(&mut self, event: UserCreated) -> Result<(), actor::Error> {
//!         self.connection.store_user(event.user).await
//!             .map_err(|e| actor::Error::Custom(e.to_string()))?;
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ### HTTP Service Integration
//!
//! Build web services with actor-based request handling:
//!
//! ```ignore
//! // HTTP handler that delegates to actors
//! async fn handle_request(
//!     request: HttpRequest,
//!     actor_ref: ActorRef<RequestHandler>,
//! ) -> HttpResponse {
//!     match actor_ref.ask(ProcessRequest(request)).await {
//!         Ok(response) => HttpResponse::Ok().json(response),
//!         Err(e) => HttpResponse::InternalServerError().json(format!("Error: {}", e)),
//!     }
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Message Processing Optimization
//!
//! - **Batching**: Group multiple small messages into larger batches when possible
//! - **Message Size**: Keep messages small to reduce serialization overhead
//! - **Handler Efficiency**: Minimize async operations in message handlers
//! - **State Design**: Structure actor state for efficient access patterns
//!
//! ### Memory Management
//!
//! - **Event Buffers**: Configure appropriate buffer sizes for event channels
//! - **Actor Lifecycle**: Implement proper cleanup in `post_stop` handlers
//! - **Child Limits**: Monitor and limit child actor creation to prevent resource exhaustion
//! - **Message Queues**: Monitor mailbox sizes to detect processing bottlenecks
//!
//! ### Scalability Patterns
//!
//! - **Worker Pools**: Create multiple worker actors for CPU-intensive tasks
//! - **Load Balancing**: Distribute work across multiple identical actors
//! - **Hierarchical Organization**: Structure actors in trees to isolate concerns
//! - **Resource Pooling**: Share expensive resources like database connections
//!
//! ## Best Practices
//!
//! ### Actor Design
//!
//! 1. **Single Responsibility**: Each actor should have one clear purpose
//! 2. **State Encapsulation**: Keep all state private and immutable from outside
//! 3. **Message-Driven**: Design interactions around clear message protocols
//! 4. **Fail Fast**: Let actors crash and restart rather than handling all errors
//!
//! ### Error Handling
//!
//! 1. **Supervision**: Use supervision strategies appropriate to your failure scenarios
//! 2. **Event Publishing**: Publish error events for monitoring and alerting
//! 3. **Resource Cleanup**: Always implement proper cleanup in `post_stop`
//! 4. **Graceful Degradation**: Design fallback behaviors for critical paths
//!
//! ### Testing
//!
//! 1. **Unit Testing**: Test message handlers in isolation
//! 2. **Integration Testing**: Test actor interactions and supervision
//! 3. **Load Testing**: Validate performance under realistic workloads
//! 4. **Failure Testing**: Test recovery scenarios and error conditions
//!
//! ## Common Pitfalls
//!
//! ### Anti-Patterns to Avoid
//!
//! - **Blocking Operations**: Never use blocking I/O in message handlers
//! - **Shared State**: Avoid sharing mutable state between actors
//! - **Message Loops**: Be careful about actors sending messages to themselves
//! - **Resource Leaks**: Always clean up resources in actor destructors
//! - **Deep Nesting**: Avoid overly deep actor hierarchies that complicate debugging
//!
//! ### Performance Gotchas
//!
//! - **Large Messages**: Avoid sending large objects in messages
//! - **Hot Actors**: Monitor for actors that become processing bottlenecks
//! - **Event Storms**: Prevent cascading event publications
//! - **Memory Pressure**: Watch for actors that accumulate unbounded state
//!
//! ## API Organization
//!
//! This crate provides a comprehensive set of types organized by functionality:
//!
//! - **Core Actor Types**: [`Actor`], [`ActorContext`], [`ActorRef`], [`Handler`], [`Message`], [`Response`]
//! - **System Management**: [`ActorSystem`], [`SystemRef`], [`SystemRunner`], [`SystemEvent`]
//! - **Fault Tolerance**: [`SupervisionStrategy`], [`Strategy`], retry strategies, [`ChildAction`]
//! - **Event Handling**: [`Event`], [`Sink`], [`Subscriber`] for publish-subscribe patterns
//! - **Retry Mechanisms**: [`RetryActor`], [`RetryMessage`] for robust error recovery
//! - **Actor Addressing**: [`ActorPath`] for hierarchical actor identification
//! - **Error Handling**: [`Error`] with comprehensive error information
//!
//! All types are designed to work together seamlessly, providing a cohesive
//! actor runtime with strong type safety and comprehensive fault tolerance.
//!

// Private modules containing the implementation
mod actor;
mod error;
mod handler;
mod path;
mod retries;
mod runner;
mod sink;
mod supervision;
mod system;

//
// Core Actor System Types
//

/// The fundamental actor trait defining actor behavior and lifecycle hooks.
///
/// This trait must be implemented by all actors in the system. It provides the essential
/// lifecycle methods that actors can override to customize their startup and shutdown
/// behavior. The trait uses async methods to support non-blocking operations during
/// actor initialization and cleanup.
///
/// See [`ActorContext`] and [`ActorRef`] for detailed usage examples.
pub use actor::Actor;

/// Execution context providing system services and actor lifecycle management.
///
/// The `ActorContext` is provided to all actors during message handling, enabling them
/// to interact with the actor system, create child actors, publish events, and manage
/// their lifecycle. This is the primary interface for actor system operations.
///
/// See the [`Actor`] trait for comprehensive examples and lifecycle hooks.
pub use actor::ActorContext;

/// A reference to an actor that enables message sending and actor interaction.
///
/// `ActorRef` provides a type-safe interface for sending messages to actors without
/// exposing their internal state. It supports both fire-and-forget (`tell`) and
/// request-response (`ask`) messaging patterns with full type safety.
///
/// See the [`Handler`] trait for message handling patterns.
pub use actor::ActorRef;

/// Enumeration of possible actions to take on child actors during supervision.
///
/// Used by the supervision system to determine how to handle child actor failures
/// and lifecycle events. This enables sophisticated fault tolerance strategies
/// through configurable child management policies.
///
/// See [`SupervisionStrategy`] for configuration examples.
pub use actor::ChildAction;

/// Trait for system events that can be published to event subscribers.
///
/// Events represent significant occurrences in the system that external components
/// may want to observe. All events must be serializable to support distributed
/// event processing and persistence.
///
/// See [`Sink`] and [`Subscriber`] for event handling patterns.
pub use actor::Event;

/// Trait for handling specific message types within actors.
///
/// This trait defines how actors respond to specific message types. Each actor
/// can implement multiple `Handler` traits for different message types, enabling
/// type-safe message processing with compile-time verification.
///
/// See the [`Actor`] trait for message handler implementation examples.
pub use actor::Handler;

/// Trait implemented by all messages that can be sent to actors.
///
/// Messages must be serializable and cloneable to support actor communication
/// across different execution contexts. The trait provides the foundation for
/// type-safe message passing in the actor system.
///
/// See the [`Handler`] trait for message design guidelines and examples.
pub use actor::Message;

/// Response wrapper for actor message handlers.
///
/// Provides a structured way to return responses from message handlers, supporting
/// both empty responses and responses with return values. The response system
/// enables request-response patterns while maintaining type safety.
///
/// See the [`Handler`] trait for response handling examples.
pub use actor::Response;

//
// Error Handling
//

/// Comprehensive error type for all actor system operations.
///
/// Provides detailed error information for various failure scenarios including
/// actor startup failures, message sending errors, system errors, and custom
/// application errors. The error type supports structured error handling with
/// appropriate error context.
///
/// This enum provides comprehensive error information for debugging and monitoring.
pub use error::Error;

//
// Actor Addressing
//

/// Hierarchical path identifying actors within the system tree.
///
/// Actor paths provide a unique addressing scheme for actors, following a
/// hierarchical structure that reflects the supervision tree. Paths enable
/// actor discovery, logging, and debugging by providing clear identity
/// for each actor instance.
///
/// Actor paths are automatically managed by the system during actor creation.
pub use path::ActorPath;

//
// Event System
//

/// Event sink container that connects event streams to subscriber implementations.
///
/// Sinks provide the bridge between the actor system's event publishing mechanism
/// and external event processing components. They manage event subscription,
/// delivery, and error handling for event-driven system integration.
///
/// See [`Subscriber`] for event processing implementation patterns.
pub use sink::Sink;

/// Trait for components that process events published by actors.
///
/// Subscribers receive notifications about system events and can implement
/// custom logic for event processing, such as logging, persistence, monitoring,
/// or integration with external systems.
///
/// See [`Sink`] for connecting subscribers to event streams.
pub use sink::Subscriber;

//
// Retry and Recovery Mechanisms
//

/// Specialized actor that implements retry logic for failed operations.
///
/// Provides a reusable component for implementing retry patterns with
/// configurable strategies. Useful for operations that may fail transiently
/// and benefit from automatic retry with backoff policies.
///
/// Provides reusable retry functionality with configurable strategies.
pub use retries::RetryActor;

/// Message wrapper that carries retry metadata and configuration.
///
/// Encapsulates original messages with retry-specific information such as
/// attempt counts, delay strategies, and failure conditions. Enables
/// sophisticated retry logic without modifying original message types.
///
/// Used with [`RetryActor`] for sophisticated retry scenarios.
pub use retries::RetryMessage;

//
// Supervision and Fault Tolerance
//

/// Strategy for custom retry intervals with user-defined delay sequences.
///
/// Provides maximum flexibility for retry timing by allowing custom delay
/// patterns such as exponential backoff, Fibonacci sequences, or any
/// application-specific timing strategy.
///
/// Enables sophisticated timing patterns like exponential backoff.
pub use supervision::CustomIntervalStrategy;

/// Strategy for fixed-interval retries with consistent delays.
///
/// Implements retry logic with uniform delays between attempts, suitable
/// for scenarios where consistent retry timing is preferred over
/// exponential backoff patterns.
///
/// Suitable for scenarios requiring consistent retry intervals.
pub use supervision::FixedIntervalStrategy;

/// Strategy for immediate retries without delays.
///
/// Enables rapid retry attempts for operations that fail quickly and
/// are expected to recover immediately. Useful for transient failures
/// that don't require backoff delays.
///
/// Optimal for transient failures requiring immediate recovery.
pub use supervision::NoIntervalStrategy;

/// Trait for implementing custom retry delay strategies.
///
/// Allows creation of sophisticated retry logic with custom delay
/// calculation, failure conditions, and retry limits. Provides the
/// foundation for building domain-specific retry behaviors.
///
/// Implement this trait for domain-specific retry behaviors.
pub use supervision::RetryStrategy;

/// Enumeration of available retry strategies for supervision policies.
///
/// Provides a type-safe way to specify different retry approaches
/// including immediate, fixed interval, and custom interval strategies.
/// Used by the supervision system to determine retry behavior.
///
/// Used with [`SupervisionStrategy`] for configuring fault tolerance.
pub use supervision::Strategy;

/// Comprehensive supervision strategy for actor failure handling.
///
/// Defines how the system responds to actor failures, including whether
/// to stop failed actors immediately or attempt recovery through retry
/// strategies. Forms the foundation of the fault tolerance system.
///
/// Foundation of the actor system's fault tolerance capabilities.
pub use supervision::SupervisionStrategy;

//
// System Management
//

/// Primary entry point for creating and configuring actor systems.
///
/// The `ActorSystem` factory creates the fundamental infrastructure needed
/// for actor execution, including system references, event systems, and
/// coordination mechanisms. Provides the bootstrap interface for actor
/// runtime environments.
///
/// See [`SystemRef`] and [`SystemRunner`] for system management.
pub use system::ActorSystem;

/// System-level events for coordination and lifecycle management.
///
/// Represents significant system-wide events such as shutdown signals,
/// actor registration, and system state changes. Used internally for
/// system coordination and externally for system monitoring.
///
/// Used internally for system coordination and monitoring.
pub use system::SystemEvent;

/// Reference to the actor system providing system-level operations.
///
/// Enables system-wide operations such as actor creation, system shutdown,
/// event publishing, and system configuration access. Provides the interface
/// for interacting with the actor system runtime.
///
/// Primary interface for system-wide operations and configuration.
pub use system::SystemRef;

/// System runner responsible for executing the actor system event loop.
///
/// Manages system-level event processing, coordination between system
/// components, and graceful shutdown procedures. The runner maintains
/// the system's operational state and handles system lifecycle events.
///
/// Execute using `runner.run().await` in a dedicated async task.
pub use system::SystemRunner;
