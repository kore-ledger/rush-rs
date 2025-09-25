// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Rush Store System
//!
//! A high-performance, type-safe persistent storage system for actors with encryption, compression,
//! and comprehensive event sourcing capabilities. This library provides a pluggable storage abstraction
//! that enables actors to persist their state and events with strong consistency guarantees, automatic
//! recovery mechanisms, and sophisticated data protection features.
//!
//! ## Overview
//!
//! The store system implements the event sourcing pattern, allowing actors to persist events that
//! modify their state and create periodic snapshots for efficient recovery. The system provides:
//!
//! - **Event Sourcing**: Complete audit trail of all state changes through event persistence
//! - **Snapshot Management**: Periodic state snapshots for fast recovery and space optimization
//! - **Data Encryption**: ChaCha20Poly1305 encryption for data-at-rest protection
//! - **Intelligent Compression**: Automatic GZIP compression with efficiency analysis
//! - **Storage Abstraction**: Pluggable backends supporting memory, database, and custom storage
//! - **Performance Monitoring**: Built-in metrics for compression ratios and storage efficiency
//! - **Crash Recovery**: Robust recovery mechanisms handling various failure scenarios
//!
//! ## Core Architecture
//!
//! The store system is built on several foundational components:
//!
//! ### Storage Abstraction Layer
//!
//! All storage operations are abstracted through traits that provide:
//!
//! - **Pluggable Backends**: Memory, database, and custom storage implementations
//! - **Consistent Interface**: Uniform API regardless of underlying storage technology
//! - **Thread Safety**: All storage operations are thread-safe with appropriate locking
//! - **Error Handling**: Comprehensive error reporting with detailed context
//!
//! ### Event Persistence System
//!
//! The event sourcing implementation provides:
//!
//! - **Ordered Events**: Sequential event numbering with gap detection and recovery
//! - **Event Replay**: Automatic state reconstruction by replaying persisted events
//! - **Consistency Guarantees**: Atomic operations ensuring data integrity
//! - **Performance Optimization**: Efficient event serialization and storage
//!
//! ### State Management
//!
//! Sophisticated state handling includes:
//!
//! - **Snapshot Creation**: Periodic state snapshots to reduce recovery time
//! - **Incremental Recovery**: Apply only events since the last snapshot
//! - **State Validation**: Integrity checks during recovery operations
//! - **Memory Management**: Efficient handling of large state objects
//!
//! ### Data Protection
//!
//! Comprehensive security features:
//!
//! - **Encryption at Rest**: ChaCha20Poly1305 authenticated encryption
//! - **Secure Key Management**: Memory-safe key storage with automatic cleanup
//! - **Data Integrity**: Authentication tags prevent tampering
//! - **Compression Security**: Safe compression with size limits
//!
//! ## Getting Started
//!
//! ### Basic Persistent Actor
//!
//! ```rust
//! use rush_store::*;
//! use actor::{Actor, ActorContext, Handler, Message, Event, Response};
//! use async_trait::async_trait;
//! use serde::{Serialize, Deserialize};
//!
//! // Define your actor state
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct BankAccount {
//!     account_id: String,
//!     balance: u64,
//!     transaction_count: u32,
//! }
//!
//! // Define events that modify the state
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! enum AccountEvent {
//!     Deposit { amount: u64 },
//!     Withdrawal { amount: u64 },
//!     Transfer { to: String, amount: u64 },
//! }
//!
//! impl Event for AccountEvent {}
//!
//! // Define messages your actor can handle
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! enum AccountMessage {
//!     Deposit(u64),
//!     Withdraw(u64),
//!     GetBalance,
//! }
//!
//! impl Message for AccountMessage {}
//!
//! // Implement the Actor trait
//! #[async_trait]
//! impl Actor for BankAccount {
//!     type Message = AccountMessage;
//!     type Event = AccountEvent;
//!     type Response = AccountResponse;
//!
//!     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         // Initialize storage and recover state
//!         self.start_store(
//!             "bank_accounts",
//!             None,
//!             ctx,
//!             memory::MemoryManager::default(),
//!             None, // No encryption for this example
//!         ).await?;
//!         Ok(())
//!     }
//!
//!     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         // Save final snapshot before stopping
//!         self.stop_store(ctx).await
//!     }
//! }
//!
//! // Implement persistence behavior
//! #[async_trait]
//! impl PersistentActor for BankAccount {
//!     type Persistence = FullPersistence; // Full event sourcing
//!
//!     fn apply(&mut self, event: &Self::Event) -> Result<(), actor::Error> {
//!         match event {
//!             AccountEvent::Deposit { amount } => {
//!                 self.balance += amount;
//!                 self.transaction_count += 1;
//!                 Ok(())
//!             }
//!             AccountEvent::Withdrawal { amount } => {
//!                 if self.balance >= *amount {
//!                     self.balance -= amount;
//!                     self.transaction_count += 1;
//!                     Ok()
//!                 } else {
//!                     Err(actor::Error::Functional("Insufficient funds".to_string()))
//!                 }
//!             }
//!             AccountEvent::Transfer { amount, .. } => {
//!                 if self.balance >= *amount {
//!                     self.balance -= amount;
//!                     self.transaction_count += 1;
//!                     Ok()
//!                 } else {
//!                     Err(actor::Error::Functional("Insufficient funds".to_string()))
//!                 }
//!             }
//!         }
//!     }
//! }
//!
//! // Implement message handlers
//! #[async_trait]
//! impl Handler<BankAccount> for BankAccount {
//!     async fn handle_message(
//!         &mut self,
//!         _sender: ActorPath,
//!         msg: AccountMessage,
//!         ctx: &mut ActorContext<Self>,
//!     ) -> Result<AccountResponse, actor::Error> {
//!         match msg {
//!             AccountMessage::Deposit(amount) => {
//!                 let event = AccountEvent::Deposit { amount };
//!                 self.persist(&event, ctx).await?;
//!                 Ok(AccountResponse::Success)
//!             }
//!             AccountMessage::Withdraw(amount) => {
//!                 let event = AccountEvent::Withdrawal { amount };
//!                 self.persist(&event, ctx).await?;
//!                 Ok(AccountResponse::Success)
//!             }
//!             AccountMessage::GetBalance => {
//!                 Ok(AccountResponse::Balance(self.balance))
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! ### Storage Backend Configuration
//!
//! The store system supports multiple storage backends:
//!
//! ```rust
//! use rush_store::{memory::MemoryManager, Store};
//!
//! // In-memory storage (for testing and development)
//! let memory_manager = MemoryManager::default();
//! let store = Store::new(
//!     "test_store",
//!     "actor_123",
//!     memory_manager,
//!     None, // No encryption
//! )?;
//!
//! // With encryption and compression
//! let encryption_key = b"your_32_byte_encryption_key_here!"; // 32 bytes
//! let encrypted_store = Store::new_with_compression(
//!     "secure_store",
//!     "sensitive_actor_456",
//!     memory_manager,
//!     Some(*encryption_key),
//!     true, // Enable compression
//! )?;
//! ```
//!
//! ## Advanced Features
//!
//! ### Light vs Full Persistence
//!
//! The system supports two persistence strategies:
//!
//! ```rust
//! // Full persistence: All events are stored for complete audit trail
//! impl PersistentActor for FullActor {
//!     type Persistence = FullPersistence;
//!     // ... implementation
//! }
//!
//! // Light persistence: Only latest event and state are kept
//! impl PersistentActor for LightActor {
//!     type Persistence = LightPersistence;
//!     // ... implementation
//! }
//! ```
//!
//! ### Performance Monitoring
//!
//! Monitor storage performance and efficiency:
//!
//! ```rust
//! let stats = store.compression_stats();
//! println!("Compression ratio: {:.1}%", stats.compression_efficiency() * 100.0);
//! println!("Total writes: {}", stats.total_writes);
//! println!("Compressed writes: {}", stats.compressed_writes);
//! println!("Bytes saved: {}", stats.total_bytes_written - stats.total_bytes_stored);
//!
//! // Make optimization decisions
//! if stats.compression_efficiency() < 0.1 {
//!     store.set_compression_enabled(false); // Disable if not beneficial
//! }
//! ```
//!
//! ### Recovery and Crash Handling
//!
//! The system handles various failure scenarios:
//!
//! ```rust
//! // Automatic recovery on actor start
//! async fn handle_recovery(ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!     // Recovery happens automatically in start_store()
//!     self.start_store("my_store", None, ctx, manager, key).await?;
//!
//!     // Actor state is now restored to the latest consistent state
//!     println!("Recovered to state: {:?}", self);
//!     Ok(())
//! }
//!
//! // Manual recovery and state inspection
//! let store_ref = ctx.get_child::<Store<Self>>("store").await.unwrap();
//! let response = store_ref.ask(StoreCommand::Recover).await?;
//!
//! if let StoreResponse::State(Some(recovered_state)) = response {
//!     self.update(recovered_state);
//!     println!("Manual recovery completed");
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Storage Efficiency
//!
//! - **Compression**: Reduces storage by 60-80% for typical serialized data
//! - **Encryption Overhead**: ~16 bytes per encrypted entry (nonce + auth tag)
//! - **Snapshot Strategy**: Balance recovery speed vs storage usage
//! - **Event Batching**: Group multiple events when possible
//!
//! ### Memory Management
//!
//! - **Smart Compression**: Only compresses data when beneficial
//! - **Size Limits**: Prevents resource exhaustion from malicious data
//! - **Lazy Loading**: Events loaded on-demand during recovery
//! - **Reference Counting**: Shared storage between multiple store instances
//!
//! ### Scalability Patterns
//!
//! - **Partitioning**: Use different store prefixes for data isolation
//! - **Sharding**: Distribute actors across multiple storage backends
//! - **Read Replicas**: Use read-only storage replicas for query-heavy workloads
//! - **Archival**: Move old events to cold storage while keeping recent data hot
//!
//! ## Error Handling and Recovery
//!
//! ### Error Categories
//!
//! The store system provides detailed error reporting:
//!
//! - **Storage Errors**: Backend-specific failures with recovery suggestions
//! - **Serialization Errors**: Data format issues with debugging information
//! - **Encryption Errors**: Key management and cryptographic failures
//! - **Consistency Errors**: State corruption detection and recovery options
//!
//! ### Recovery Strategies
//!
//! - **Automatic Recovery**: Self-healing for transient failures
//! - **Manual Intervention**: Tools for data recovery and repair
//! - **Backup and Restore**: State export and import capabilities
//! - **Rollback Support**: Revert to previous known-good state
//!
//! ## Integration Patterns
//!
//! ### Database Integration
//!
//! ```rust
//! // Implement custom database backend
//! struct DatabaseManager {
//!     connection: DatabasePool,
//! }
//!
//! impl DbManager<DatabaseCollection, DatabaseState> for DatabaseManager {
//!     fn create_collection(&self, name: &str, prefix: &str) -> Result<DatabaseCollection, Error> {
//!         // Create database tables, indexes, etc.
//!         Ok(DatabaseCollection::new(name, prefix, &self.connection)?)
//!     }
//!
//!     fn create_state(&self, name: &str, prefix: &str) -> Result<DatabaseState, Error> {
//!         // Create state storage tables
//!         Ok(DatabaseState::new(name, prefix, &self.connection)?)
//!     }
//! }
//! ```
//!
//! ### Monitoring and Observability
//!
//! ```rust
//! // Custom metrics collection
//! struct MetricsCollector {
//!     store_ref: ActorRef<Store<MyActor>>,
//! }
//!
//! impl MetricsCollector {
//!     async fn collect_metrics(&self) {
//!         let stats = self.store_ref.ask(StoreCommand::GetStats).await?;
//!         // Send to monitoring system
//!         metrics::gauge!("store.compression_ratio", stats.compression_ratio);
//!         metrics::counter!("store.events_persisted", stats.total_writes);
//!     }
//! }
//! ```
//!
//! ## Best Practices
//!
//! ### Actor Design
//!
//! 1. **Event Design**: Make events immutable and serializable
//! 2. **State Validation**: Validate state consistency during recovery
//! 3. **Idempotency**: Design event handlers to be idempotent when possible
//! 4. **Snapshot Frequency**: Balance recovery speed with storage overhead
//!
//! ### Security
//!
//! 1. **Key Management**: Use secure key derivation and rotation
//! 2. **Data Validation**: Validate all deserialized data
//! 3. **Access Control**: Implement proper access controls on storage backends
//! 4. **Audit Logging**: Log all storage operations for compliance
//!
//! ### Performance
//!
//! 1. **Batch Operations**: Group multiple events when possible
//! 2. **Monitor Metrics**: Track compression ratios and storage efficiency
//! 3. **Optimize Serialization**: Use efficient serialization formats
//! 4. **Cache Strategy**: Implement appropriate caching for frequently accessed data
//!
//! ## API Organization
//!
//! This crate provides a comprehensive set of types organized by functionality:
//!
//! - **Core Storage Types**: [`Store`], [`PersistentActor`], [`DbManager`], [`Collection`], [`State`]
//! - **Persistence Strategies**: [`FullPersistence`], [`LightPersistence`], [`PersistenceType`]
//! - **Commands and Responses**: [`StoreCommand`], [`StoreResponse`], [`StoreEvent`]
//! - **Storage Backends**: [`memory::MemoryManager`], [`memory::MemoryStore`]
//! - **Performance Monitoring**: [`CompressionStats`] for storage efficiency analysis
//! - **Error Handling**: [`Error`] with detailed error context and recovery information
//!
//! All types are designed to work together seamlessly, providing a cohesive
//! storage system with strong type safety, comprehensive error handling, and
//! sophisticated data protection features.

pub mod database;
pub mod error;
pub mod memory;
pub mod store;

// Re-export all public APIs for easy access

/// Comprehensive error handling for the store system.
///
/// Provides detailed error information for storage operations, with specific
/// error categories for different failure modes and recovery strategies.
/// See the [`error`] module for detailed error descriptions.
pub use error::Error;

/// Core storage abstractions and database management traits.
///
/// The [`database`] module provides the fundamental traits that enable
/// pluggable storage backends with consistent interfaces.
pub use database::*;

/// In-memory storage implementation for development and testing.
///
/// The [`memory`] module provides a complete in-memory storage backend
/// that's perfect for testing, development, and scenarios where
/// persistence is not required.
pub use memory::*;

/// Complete persistent storage system with event sourcing capabilities.
///
/// The [`store`] module contains the core [`Store`] actor, [`PersistentActor`]
/// trait, and all related types for implementing event sourcing with
/// encryption, compression, and sophisticated recovery mechanisms.
pub use store::*;
