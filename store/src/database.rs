// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Database Storage Abstraction
//!
//! This module provides the fundamental traits and abstractions for pluggable storage backends,
//! enabling the store system to work with different storage technologies while maintaining a
//! consistent interface. The abstraction layer supports both key-value collections and single-state
//! storage patterns, with comprehensive error handling and performance optimization features.
//!
//! ## Architecture Overview
//!
//! The storage abstraction is built around three core traits:
//!
//! - **[`DbManager`]**: Factory for creating storage instances with consistent configuration
//! - **[`Collection`]**: Key-value storage interface for event persistence and data collections
//! - **[`State`]**: Single-value storage interface for actor state snapshots
//!
//! This design enables:
//!
//! - **Backend Flexibility**: Easy switching between memory, file, database, and cloud storage
//! - **Consistent Interface**: Uniform API regardless of underlying storage technology
//! - **Performance Optimization**: Backend-specific optimizations while maintaining compatibility
//! - **Testing Support**: Memory-based implementations for unit tests and development
//!
//! ## Storage Patterns
//!
//! ### Event Collections
//!
//! Collections store ordered sequences of events with string keys, typically used for:
//! - Event sourcing logs with sequential event IDs
//! - Time-series data with timestamp keys
//! - Versioned data with version number keys
//! - Any scenario requiring ordered key-value storage with iteration support
//!
//! ### State Storage
//!
//! State storage manages single values per entity, typically used for:
//! - Actor state snapshots for fast recovery
//! - Configuration data and system settings
//! - Cached computed results
//! - Any scenario requiring single-value storage with atomic updates
//!
//! ## Implementation Guide
//!
//! ### Creating a Custom Storage Backend
//!
//! ```ignore
//! use rush_store::database::{DbManager, Collection, State};
//! use rush_store::Error;
//!
//! // Define your storage backend types
//! struct MyStorageManager {
//!     connection_pool: ConnectionPool,
//!     config: StorageConfig,
//! }
//!
//! struct MyCollection {
//!     table_name: String,
//!     prefix: String,
//!     connection: Connection,
//! }
//!
//! struct MyState {
//!     table_name: String,
//!     entity_id: String,
//!     connection: Connection,
//! }
//!
//! // Implement the manager trait
//! impl DbManager<MyCollection, MyState> for MyStorageManager {
//!     fn create_collection(&self, name: &str, prefix: &str) -> Result<MyCollection, Error> {
//!         let connection = self.connection_pool.get()?;
//!
//!         // Create tables, indexes, etc.
//!         connection.execute(&format!(
//!             "CREATE TABLE IF NOT EXISTS {} (key TEXT PRIMARY KEY, value BLOB)",
//!             name
//!         ))?;
//!
//!         Ok(MyCollection {
//!             table_name: name.to_string(),
//!             prefix: prefix.to_string(),
//!             connection,
//!         })
//!     }
//!
//!     fn create_state(&self, name: &str, prefix: &str) -> Result<MyState, Error> {
//!         let connection = self.connection_pool.get()?;
//!
//!         // Create state table
//!         connection.execute(&format!(
//!             "CREATE TABLE IF NOT EXISTS {} (entity_id TEXT PRIMARY KEY, data BLOB)",
//!             name
//!         ))?;
//!
//!         Ok(MyState {
//!             table_name: name.to_string(),
//!             entity_id: prefix.to_string(),
//!             connection,
//!         })
//!     }
//! }
//! ```
//!
//! ### Testing Your Implementation
//!
//! The module provides a comprehensive test macro for validating storage implementations:
//!
//! ```ignore
//! #[cfg(test)]
//! mod tests {
//!     use super::*;
//!     use rush_store::test_store_trait;
//!
//!     // This macro generates comprehensive tests for your storage implementation
//!     test_store_trait! {
//!         my_storage_tests: MyStorageManager: MyCollection
//!     }
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Collection Performance
//!
//! - **Indexing**: Ensure proper indexing on key columns for fast lookups
//! - **Iteration**: Optimize for both forward and reverse iteration patterns
//! - **Range Queries**: Support efficient range operations for event replay
//! - **Batch Operations**: Implement batch puts/gets where possible
//!
//! ### State Performance
//!
//! - **Atomic Updates**: Ensure state updates are atomic to prevent corruption
//! - **Connection Pooling**: Use connection pools for concurrent access
//! - **Caching**: Consider caching frequently accessed state data
//! - **Compression**: Implement compression for large state objects
//!
//! ## Error Handling
//!
//! All storage operations should provide rich error context:
//!
//! ```ignore
//! match storage.get("key") {
//!     Ok(data) => process_data(data),
//!     Err(Error::EntryNotFound(_)) => handle_missing_data(),
//!     Err(Error::Store(reason)) => {
//!         log::error!("Storage operation failed: {}", reason);
//!         retry_with_backoff()
//!     }
//!     Err(e) => return Err(e),
//! }
//! ```
//!
//! ## Thread Safety
//!
//! All storage traits require `Send + Sync` implementations to support:
//! - Concurrent access from multiple actors
//! - Sharing storage instances across threads
//! - Async operation support with proper synchronization
//!
//! Implementation should ensure:
//! - Internal synchronization for shared mutable state
//! - Connection pool thread safety
//! - Proper resource cleanup and lifetime management

use crate::error::Error;

use tracing::debug;

/// Database manager trait for creating and managing storage instances.
///
/// This trait serves as a factory for creating storage backends, providing a consistent
/// interface for initializing both collection-based and state-based storage while handling
/// backend-specific configuration, connection management, and resource lifecycle.
///
/// # Type Parameters
///
/// * `C` - Collection type that implements the [`Collection`] trait for key-value storage
/// * `S` - State type that implements the [`State`] trait for single-value storage
///
/// # Design Principles
///
/// The manager pattern enables:
/// - **Resource Management**: Centralized handling of connections, pools, and configuration
/// - **Factory Pattern**: Consistent creation of storage instances with proper initialization
/// - **Backend Abstraction**: Hide implementation details from storage consumers
/// - **Lifecycle Management**: Proper startup and shutdown procedures for storage resources
///
/// # Thread Safety
///
/// Implementors must ensure thread safety as managers may be shared across multiple actors
/// and async tasks. This typically involves:
/// - Internal synchronization for shared resources (connection pools, configurations)
/// - Atomic operations for resource allocation and deallocation
/// - Safe concurrent access to backend systems
///
/// # Examples
///
/// ## Database Backend Implementation
///
/// ```ignore
/// use rush_store::database::{DbManager, Collection, State};
/// use rush_store::Error;
/// use std::sync::Arc;
///
/// #[derive(Clone)]
/// struct DatabaseManager {
///     connection_pool: Arc<ConnectionPool>,
///     config: DatabaseConfig,
/// }
///
/// impl DbManager<DatabaseCollection, DatabaseState> for DatabaseManager {
///     fn create_collection(&self, name: &str, prefix: &str) -> Result<DatabaseCollection, Error> {
///         let connection = self.connection_pool.get()
///             .map_err(|e| Error::CreateStore(format!("Connection failed: {}", e)))?;
///
///         // Initialize collection table with proper indexing
///         let table_name = format!("collection_{}", name);
///         connection.execute(&format!(
///             "CREATE TABLE IF NOT EXISTS {} (
///                 key TEXT PRIMARY KEY,
///                 value BLOB NOT NULL,
///                 created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
///                 INDEX idx_prefix (key) WHERE key LIKE '{}%'
///             )",
///             table_name, prefix
///         )).map_err(|e| Error::CreateStore(format!("Table creation failed: {}", e)))?;
///
///         Ok(DatabaseCollection {
///             table_name,
///             prefix: prefix.to_string(),
///             connection,
///         })
///     }
///
///     fn create_state(&self, name: &str, prefix: &str) -> Result<DatabaseState, Error> {
///         let connection = self.connection_pool.get()
///             .map_err(|e| Error::CreateStore(format!("Connection failed: {}", e)))?;
///
///         // Initialize state table with entity-specific storage
///         let table_name = format!("state_{}", name);
///         connection.execute(&format!(
///             "CREATE TABLE IF NOT EXISTS {} (
///                 entity_id TEXT PRIMARY KEY,
///                 data BLOB NOT NULL,
///                 version INTEGER DEFAULT 1,
///                 updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
///             )",
///             table_name
///         )).map_err(|e| Error::CreateStore(format!("Table creation failed: {}", e)))?;
///
///         Ok(DatabaseState {
///             table_name,
///             entity_id: prefix.to_string(),
///             connection,
///         })
///     }
///
///     fn stop(self) -> Result<(), Error> {
///         // Close connection pool and clean up resources
///         self.connection_pool.close_all();
///         Ok(())
///     }
/// }
/// ```
///
/// ## Memory Backend for Testing
///
/// ```ignore
/// #[derive(Default, Clone)]
/// struct TestManager {
///     data: Arc<RwLock<HashMap<String, Arc<RwLock<BTreeMap<String, Vec<u8>>>>>>>,
/// }
///
/// impl DbManager<MemoryCollection, MemoryState> for TestManager {
///     fn create_collection(&self, name: &str, prefix: &str) -> Result<MemoryCollection, Error> {
///         let mut data_lock = self.data.write().unwrap();
///         let collection_key = format!("{}-{}", name, prefix);
///
///         let storage = data_lock.entry(collection_key.clone())
///             .or_insert_with(|| Arc::new(RwLock::new(BTreeMap::new())))
///             .clone();
///
///         Ok(MemoryCollection {
///             name: name.to_string(),
///             prefix: prefix.to_string(),
///             storage,
///         })
///     }
///
///     fn create_state(&self, name: &str, prefix: &str) -> Result<MemoryState, Error> {
///         // Similar implementation for state storage
///         // ...
///     }
/// }
/// ```
///
/// # Error Handling
///
/// Implementations should provide detailed error context for common failure scenarios:
/// - Connection failures with retry suggestions
/// - Permission issues with resolution steps
/// - Resource exhaustion with capacity information
/// - Configuration errors with validation details
///
/// # Performance Guidelines
///
/// - **Connection Pooling**: Use connection pools for database backends
/// - **Resource Reuse**: Reuse expensive resources across collection/state instances
/// - **Lazy Initialization**: Initialize resources only when needed
/// - **Batch Operations**: Support batch creation where possible
/// - **Monitoring**: Expose metrics for connection health and resource usage
pub trait DbManager<C, S>: Sync + Send + Clone
where
    C: Collection + 'static,
    S: State + 'static,
{
    /// Create a new collection instance for key-value storage.
    ///
    /// Creates and initializes a storage collection that supports ordered key-value pairs
    /// with iteration, range queries, and efficient lookups. Collections are typically
    /// used for event logs, time-series data, and any scenario requiring ordered storage.
    ///
    /// # Parameters
    ///
    /// * `name` - Unique collection identifier, used for table/file naming and isolation
    /// * `prefix` - Entity-specific prefix for data partitioning within the collection
    ///
    /// # Returns
    ///
    /// Returns a `Result<C, Error>` where:
    /// - `Ok(collection)` - Successfully created and initialized collection
    /// - `Err(error)` - Creation failed due to backend issues, permissions, or configuration
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::DbManager;
    ///
    /// let manager = DatabaseManager::new(connection_pool);
    ///
    /// // Create collection for user events
    /// let user_events = manager.create_collection("user_events", "user_123")?;
    ///
    /// // Create collection for system metrics with timestamp keys
    /// let metrics = manager.create_collection("metrics", "system")?;
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// Implementors should:
    /// - Initialize necessary backend structures (tables, indexes, directories)
    /// - Set up proper indexing for key-based operations and iteration
    /// - Configure appropriate storage options (compression, replication, etc.)
    /// - Validate naming conventions and prevent conflicts
    /// - Handle concurrent creation requests safely
    ///
    /// # Error Conditions
    ///
    /// Common failure scenarios:
    /// - Backend unavailable or connection failures
    /// - Insufficient permissions for storage creation
    /// - Invalid naming or conflicting collection names
    /// - Resource exhaustion (disk space, memory, connections)
    /// - Configuration errors or missing dependencies
    fn create_collection(&self, name: &str, prefix: &str) -> Result<C, Error>;

    /// Create a new state storage instance for single-value storage.
    ///
    /// Creates and initializes a storage container that manages a single value per entity,
    /// typically used for actor state snapshots, configuration data, or cached results.
    /// State storage supports atomic updates and optimized access patterns for single values.
    ///
    /// # Parameters
    ///
    /// * `name` - Unique storage identifier, used for table/file naming and isolation
    /// * `prefix` - Entity identifier whose state is being stored (e.g., actor ID)
    ///
    /// # Returns
    ///
    /// Returns a `Result<S, Error>` where:
    /// - `Ok(state)` - Successfully created and initialized state storage
    /// - `Err(error)` - Creation failed due to backend issues, permissions, or configuration
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::DbManager;
    ///
    /// let manager = DatabaseManager::new(connection_pool);
    ///
    /// // Create state storage for actor snapshot
    /// let actor_state = manager.create_state("actor_snapshots", "actor_456")?;
    ///
    /// // Create state storage for configuration
    /// let config_state = manager.create_state("system_config", "global")?;
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// Implementors should:
    /// - Initialize backend structures optimized for single-value storage
    /// - Set up atomic update mechanisms to prevent corruption
    /// - Configure appropriate consistency guarantees
    /// - Handle versioning if required for conflict resolution
    /// - Implement efficient serialization/deserialization pathways
    ///
    /// # Performance Considerations
    ///
    /// - Use primary key constraints for fast lookups
    /// - Consider compression for large state objects
    /// - Implement connection pooling for concurrent access
    /// - Cache frequently accessed state data when appropriate
    /// - Use appropriate isolation levels for transactional consistency
    fn create_state(&self, name: &str, prefix: &str) -> Result<S, Error>;

    /// Gracefully shutdown the storage manager and clean up resources.
    ///
    /// Performs cleanup operations including closing connections, flushing buffers,
    /// releasing resources, and ensuring data consistency before shutdown. This method
    /// should be called during application shutdown to prevent data loss and resource leaks.
    ///
    /// # Default Implementation
    ///
    /// The default implementation performs no cleanup operations, which is suitable
    /// for simple backends that don't require explicit resource management.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - Shutdown completed successfully
    /// - `Err(error)` - Shutdown failed, may indicate data loss or resource leaks
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::DbManager;
    ///
    /// let manager = DatabaseManager::new(connection_pool);
    ///
    /// // Use the manager...
    ///
    /// // Clean shutdown
    /// manager.stop()?;
    /// ```
    ///
    /// # Implementation Guidelines
    ///
    /// For database backends:
    /// - Close all connections in the pool
    /// - Flush any pending writes or transactions
    /// - Release file handles and network resources
    /// - Cancel background tasks and cleanup threads
    ///
    /// For file-based backends:
    /// - Flush all file buffers to disk
    /// - Close file handles and release locks
    /// - Clean up temporary files and directories
    /// - Sync filesystem metadata
    ///
    /// For distributed backends:
    /// - Gracefully disconnect from cluster nodes
    /// - Complete ongoing replication operations
    /// - Release distributed locks and resources
    /// - Update cluster membership information
    ///
    /// # Error Handling
    ///
    /// Even if shutdown errors occur, implementors should:
    /// - Continue cleanup operations where possible
    /// - Log detailed error information for debugging
    /// - Prevent cascading failures that could affect other components
    /// - Provide actionable error messages for manual intervention
    fn stop(self) -> Result<(), Error> {
        Ok(())
    }
}

/// Single-value storage trait for actor state snapshots and configuration data.
///
/// This trait defines the interface for storing and retrieving single values per entity,
/// typically used for actor state snapshots, configuration settings, or cached computed
/// results. State storage is optimized for atomic updates and efficient access to
/// single values rather than collections of data.
///
/// # Use Cases
///
/// - **Actor Snapshots**: Periodic state saves for fast actor recovery
/// - **Configuration Storage**: System and user configuration persistence
/// - **Cached Results**: Storing expensive computation results
/// - **Single-Value Entities**: Any scenario requiring atomic single-value updates
///
/// # Thread Safety
///
/// Implementations must be thread-safe (`Send + Sync`) to support concurrent access
/// from multiple actors and async contexts. This typically requires internal
/// synchronization or backend-level concurrency control.
///
/// # Design Principles
///
/// - **Atomic Operations**: All updates should be atomic to prevent corruption
/// - **Consistency**: Maintain data consistency across concurrent operations
/// - **Performance**: Optimize for single-value access patterns
/// - **Reliability**: Provide strong durability guarantees for critical data
///
/// # Examples
///
/// ## Basic State Operations
///
/// ```ignore
/// use rush_store::database::State;
/// use rush_store::Error;
///
/// async fn manage_actor_state(mut state: impl State) -> Result<(), Error> {
///     // Check if state exists
///     match state.get() {
///         Ok(data) => {
///             println!("Existing state found: {} bytes", data.len());
///             // Process existing state...
///         }
///         Err(Error::EntryNotFound(_)) => {
///             println!("No existing state, creating default");
///             let default_state = create_default_state();
///             state.put(&default_state)?;
///         }
///         Err(e) => return Err(e),
///     }
///
///     // Update state
///     let updated_state = modify_state(&state.get()?)?;
///     state.put(&updated_state)?;
///
///     // Ensure data is persisted
///     state.flush()?;
///
///     Ok(())
/// }
/// ```
///
/// ## Configuration Management
///
/// ```ignore
/// use serde::{Serialize, Deserialize};
/// use bincode;
///
/// #[derive(Serialize, Deserialize)]
/// struct AppConfig {
///     max_connections: u32,
///     timeout_seconds: u64,
///     enable_features: Vec<String>,
/// }
///
/// async fn load_config(state: &impl State) -> Result<AppConfig, Error> {
///     match state.get() {
///         Ok(data) => {
///             bincode::deserialize(&data)
///                 .map_err(|e| Error::Store(format!("Config deserialization failed: {}", e)))
///         }
///         Err(Error::EntryNotFound(_)) => {
///             // Return default configuration
///             Ok(AppConfig::default())
///         }
///         Err(e) => Err(e),
///     }
/// }
///
/// async fn save_config(state: &mut impl State, config: &AppConfig) -> Result<(), Error> {
///     let data = bincode::serialize(config)
///         .map_err(|e| Error::Store(format!("Config serialization failed: {}", e)))?;
///     state.put(&data)?;
///     state.flush() // Ensure immediate persistence
/// }
/// ```
///
/// # Implementation Guidelines
///
/// ## Data Format
///
/// State storage works with raw bytes (`Vec<u8>`), allowing flexibility in serialization:
/// - Use efficient serialization formats (bincode, protobuf, msgpack)
/// - Consider compression for large state objects
/// - Implement versioning for schema evolution
/// - Validate data integrity during deserialization
///
/// ## Error Handling
///
/// Distinguish between different error types:
/// - `EntryNotFound`: Normal case for initial state creation
/// - `Store`: Persistent failures requiring retry or fallback
/// - Infrastructure errors: Connection, permission, or resource issues
///
/// ## Performance Optimization
///
/// - Use appropriate backend storage types (key-value stores, document databases)
/// - Implement connection pooling for database backends
/// - Consider caching for frequently accessed state
/// - Optimize serialization format for your data patterns
/// - Use transactions for consistency when supported
///
/// ## Consistency Guarantees
///
/// - Implement atomic updates to prevent partial state corruption
/// - Use appropriate isolation levels for concurrent access
/// - Consider versioning or optimistic locking for conflict resolution
/// - Provide clear consistency guarantees in documentation
pub trait State: Sync + Send + 'static {
    /// Get the storage instance identifier.
    ///
    /// Returns the name or identifier of this state storage instance, typically used
    /// for logging, debugging, and resource management. This should be a stable
    /// identifier that uniquely identifies the storage instance.
    ///
    /// # Returns
    ///
    /// A string slice containing the storage instance name
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::State;
    ///
    /// let state = create_actor_state("actor_123")?;
    /// println!("Using state storage: {}", state.name());
    /// ```
    fn name(&self) -> &str;

    /// Retrieve the current state value.
    ///
    /// Fetches the stored state data as raw bytes. The caller is responsible for
    /// deserializing the data into the appropriate format. If no state has been
    /// stored yet, this will return an `EntryNotFound` error.
    ///
    /// # Returns
    ///
    /// Returns a `Result<Vec<u8>, Error>` where:
    /// - `Ok(data)` - Successfully retrieved state data as raw bytes
    /// - `Err(EntryNotFound(_))` - No state has been stored yet (normal for new entities)
    /// - `Err(Store(_))` - Storage backend error occurred during retrieval
    /// - `Err(Get(_))` - Specific retrieval error with detailed context
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::State, Error};
    /// use serde::Deserialize;
    ///
    /// async fn load_typed_state<T: for<'de> Deserialize<'de>>(
    ///     state: &impl State
    /// ) -> Result<Option<T>, Error> {
    ///     match state.get() {
    ///         Ok(data) => {
    ///             let typed_state = bincode::deserialize(&data)
    ///                 .map_err(|e| Error::Store(format!("Deserialization failed: {}", e)))?;
    ///             Ok(Some(typed_state))
    ///         }
    ///         Err(Error::EntryNotFound(_)) => Ok(None), // No state yet
    ///         Err(e) => Err(e),
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Notes
    ///
    /// - Consider caching frequently accessed state to reduce backend calls
    /// - For large state objects, consider implementing lazy loading patterns
    /// - Monitor retrieval latency and implement appropriate timeouts
    fn get(&self) -> Result<Vec<u8>, Error>;

    /// Store or update the state value.
    ///
    /// Atomically stores the provided data as the new state value, replacing any
    /// existing state. The operation should be atomic to prevent corruption from
    /// concurrent updates or system failures during the write operation.
    ///
    /// # Parameters
    ///
    /// * `data` - Raw bytes representing the serialized state data
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - State successfully stored
    /// - `Err(Store(_))` - Storage backend error occurred during write
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::State, Error};
    /// use serde::Serialize;
    ///
    /// async fn save_typed_state<T: Serialize>(
    ///     state: &mut impl State,
    ///     value: &T
    /// ) -> Result<(), Error> {
    ///     let data = bincode::serialize(value)
    ///         .map_err(|e| Error::Store(format!("Serialization failed: {}", e)))?;
    ///
    ///     state.put(&data)?;
    ///     state.flush()?; // Ensure immediate persistence
    ///     Ok(())
    /// }
    ///
    /// // Atomic state update with validation
    /// async fn update_state_atomically(
    ///     state: &mut impl State,
    ///     update_fn: impl Fn(StateData) -> Result<StateData, Error>
    /// ) -> Result<(), Error> {
    ///     // Get current state
    ///     let current_data = state.get().unwrap_or_default();
    ///     let current_state: StateData = bincode::deserialize(&current_data)
    ///         .map_err(|e| Error::Store(format!("Invalid state format: {}", e)))?;
    ///
    ///     // Apply update
    ///     let new_state = update_fn(current_state)?;
    ///
    ///     // Validate before storing
    ///     if !new_state.is_valid() {
    ///         return Err(Error::Store("State validation failed".to_string()));
    ///     }
    ///
    ///     // Store atomically
    ///     let new_data = bincode::serialize(&new_state)
    ///         .map_err(|e| Error::Store(format!("Serialization failed: {}", e)))?;
    ///     state.put(&new_data)
    /// }
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// - Ensure atomic updates to prevent partial state corruption
    /// - Consider implementing versioning for conflict detection
    /// - Use appropriate transaction isolation levels for database backends
    /// - Implement proper error handling and rollback mechanisms
    /// - Consider compression for large state objects
    fn put(&mut self, data: &[u8]) -> Result<(), Error>;

    /// Delete the stored state value.
    ///
    /// Removes the current state value from storage. After this operation,
    /// subsequent `get()` calls will return `EntryNotFound` until a new
    /// value is stored with `put()`.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - State successfully deleted
    /// - `Err(EntryNotFound(_))` - No state was present to delete
    /// - `Err(Store(_))` - Storage backend error occurred during deletion
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::State, Error};
    ///
    /// async fn reset_actor_state(state: &mut impl State) -> Result<(), Error> {
    ///     match state.del() {
    ///         Ok(()) => {
    ///             println!("State successfully reset");
    ///             Ok(())
    ///         }
    ///         Err(Error::EntryNotFound(_)) => {
    ///             println!("No state to reset (already clean)");
    ///             Ok(()) // Not an error condition
    ///         }
    ///         Err(e) => {
    ///             println!("Failed to reset state: {}", e);
    ///             Err(e)
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Use Cases
    ///
    /// - Actor cleanup during shutdown or restart
    /// - Resetting configuration to defaults
    /// - Implementing "forget" or privacy compliance features
    /// - Clearing cached data when it becomes invalid
    fn del(&mut self) -> Result<(), Error>;

    /// Remove all state data for this storage instance.
    ///
    /// Performs a complete cleanup of all data associated with this state storage
    /// instance. This is typically used during testing, development, or when
    /// performing complete system resets. Unlike `del()`, which removes the current
    /// state value, `purge()` removes all associated metadata and history.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - All data successfully purged
    /// - `Err(Store(_))` - Storage backend error occurred during purge
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::State;
    ///
    /// async fn cleanup_test_data(state: &mut impl State) -> Result<(), Error> {
    ///     state.purge()?;
    ///     println!("All test data cleaned up for: {}", state.name());
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Warning
    ///
    /// This operation is destructive and irreversible. Use with caution in
    /// production environments. Consider implementing backup/restore mechanisms
    /// before purging important data.
    ///
    /// # Use Cases
    ///
    /// - Test cleanup and isolation
    /// - Development environment resets
    /// - Data privacy compliance (complete data removal)
    /// - System maintenance and cleanup operations
    fn purge(&mut self) -> Result<(), Error>;

    /// Ensure all pending state changes are persisted to durable storage.
    ///
    /// Forces any buffered writes or pending transactions to be committed to
    /// persistent storage. This provides stronger durability guarantees at the
    /// cost of potentially reduced performance. The default implementation
    /// performs no operation, which is suitable for backends that don't buffer writes.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - All data successfully flushed to persistent storage
    /// - `Err(Store(_))` - Flush operation failed, data may be lost
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::State;
    ///
    /// async fn save_critical_state(
    ///     state: &mut impl State,
    ///     critical_data: &[u8]
    /// ) -> Result<(), Error> {
    ///     // Store the data
    ///     state.put(critical_data)?;
    ///
    ///     // Ensure it's immediately persisted
    ///     state.flush()?;
    ///
    ///     println!("Critical state safely persisted");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # When to Use
    ///
    /// - After storing critical state that must survive system crashes
    /// - Before system shutdown to ensure data consistency
    /// - In testing to ensure deterministic state persistence
    /// - When implementing synchronous semantics over asynchronous storage
    ///
    /// # Performance Impact
    ///
    /// Flush operations can be expensive as they typically involve:
    /// - Forcing data to disk through OS buffers
    /// - Committing database transactions
    /// - Synchronizing distributed storage systems
    /// - Waiting for replication acknowledgments
    ///
    /// Use judiciously in performance-critical code paths.
    fn flush(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// Key-value collection storage trait for ordered data persistence.
///
/// This trait defines the interface for storing and retrieving ordered key-value pairs,
/// typically used for event logs, time-series data, versioned information, or any
/// scenario requiring ordered storage with efficient iteration and range queries.
/// Collections support both forward and reverse iteration, range operations, and
/// efficient key-based lookups.
///
/// # Use Cases
///
/// - **Event Sourcing**: Storing sequential events with incrementing IDs
/// - **Time-Series Data**: Metrics, logs, or measurements with timestamp keys
/// - **Versioned Storage**: Document versions, configuration history
/// - **Audit Trails**: Ordered sequence of actions or changes
/// - **Message Queues**: Ordered message storage with position-based access
///
/// # Key Ordering
///
/// Collections maintain keys in lexicographic (string) order, which enables:
/// - Efficient range queries and iteration
/// - Natural ordering for timestamp-based keys (ISO 8601 format)
/// - Predictable iteration behavior
/// - Support for prefix-based filtering
///
/// # Thread Safety
///
/// Implementations must be thread-safe (`Send + Sync`) to support concurrent access
/// from multiple actors and async contexts. This typically requires internal
/// synchronization or backend-level concurrency control.
///
/// # Design Principles
///
/// - **Ordered Storage**: Maintain consistent key ordering for predictable iteration
/// - **Efficient Access**: Optimize for both single-key lookups and range operations
/// - **Scalable Iteration**: Support efficient iteration over large datasets
/// - **Atomic Operations**: Ensure consistency for individual operations
///
/// # Examples
///
/// ## Event Log Implementation
///
/// ```ignore
/// use rush_store::{database::Collection, Error};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct Event {
///     timestamp: u64,
///     event_type: String,
///     data: serde_json::Value,
/// }
///
/// async fn store_event(
///     collection: &mut impl Collection,
///     event: &Event
/// ) -> Result<String, Error> {
///     // Use zero-padded timestamp as key for natural ordering
///     let key = format!("{:020}", event.timestamp);
///     let data = bincode::serialize(event)
///         .map_err(|e| Error::Store(format!("Serialization failed: {}", e)))?;
///
///     collection.put(&key, &data)?;
///     Ok(key)
/// }
///
/// async fn get_events_since(
///     collection: &impl Collection,
///     since_timestamp: u64
/// ) -> Result<Vec<Event>, Error> {
///     let since_key = format!("{:020}", since_timestamp);
///     let mut events = Vec::new();
///
///     for (key, data) in collection.iter(false) {
///         if key > since_key {
///             let event: Event = bincode::deserialize(&data)
///                 .map_err(|e| Error::Store(format!("Deserialization failed: {}", e)))?;
///             events.push(event);
///         }
///     }
///
///     Ok(events)
/// }
/// ```
///
/// ## Range Queries
///
/// ```ignore
/// use rush_store::database::Collection;
///
/// async fn get_recent_entries(
///     collection: &impl Collection,
///     count: usize
/// ) -> Result<Vec<(String, Vec<u8>)>, Error> {
///     let mut results = Vec::new();
///     let mut iter = collection.iter(true); // Reverse iteration for most recent
///
///     for _ in 0..count {
///         if let Some(entry) = iter.next() {
///             results.push(entry);
///         } else {
///             break;
///         }
///     }
///
///     results.reverse(); // Return in chronological order
///     Ok(results)
/// }
/// ```
///
/// # Implementation Guidelines
///
/// ## Key Design
///
/// Choose key formats that support your access patterns:
/// - **Sequential IDs**: Zero-padded numbers (`{:020}`) for natural ordering
/// - **Timestamps**: ISO 8601 format (`YYYY-MM-DDTHH:MM:SS.sssZ`) for time-based ordering
/// - **Hierarchical**: Dot-separated paths (`level1.level2.item`) for nested organization
/// - **Prefixed**: Category prefixes (`user.123.action.456`) for namespace separation
///
/// ## Performance Optimization
///
/// - Use appropriate indexing strategies for your key patterns
/// - Implement connection pooling for database backends
/// - Consider bloom filters or caching for frequently accessed keys
/// - Optimize iteration patterns for your specific use cases
/// - Use batch operations when available in the backend
///
/// ## Error Handling
///
/// Distinguish between different error scenarios:
/// - `EntryNotFound`: Normal case for non-existent keys
/// - `Store`: Persistent failures requiring retry or investigation
/// - Infrastructure errors: Connection, permission, or resource issues
pub trait Collection: Sync + Send + 'static {
    /// Get the collection identifier.
    ///
    /// Returns the name or identifier of this collection instance, typically used
    /// for logging, debugging, and resource management. This should be a stable
    /// identifier that uniquely identifies the collection.
    ///
    /// # Returns
    ///
    /// A string slice containing the collection name
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// let events = create_event_collection("user_events")?;
    /// println!("Using collection: {}", events.name());
    /// ```
    fn name(&self) -> &str;

    /// Retrieve the value associated with a specific key.
    ///
    /// Fetches the data stored under the given key. Keys are compared using
    /// lexicographic ordering, and the lookup should be efficient (typically O(log n)
    /// for tree-based storage or O(1) for hash-based storage with appropriate indexing).
    ///
    /// # Parameters
    ///
    /// * `key` - The string key to retrieve the value for
    ///
    /// # Returns
    ///
    /// Returns a `Result<Vec<u8>, Error>` where:
    /// - `Ok(data)` - Successfully retrieved data as raw bytes
    /// - `Err(EntryNotFound(_))` - Key does not exist in the collection
    /// - `Err(Get(_))` - Storage backend error occurred during retrieval
    /// - `Err(Store(_))` - General storage system error
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::Collection, Error};
    ///
    /// async fn get_event_data(
    ///     collection: &impl Collection,
    ///     event_id: &str
    /// ) -> Result<Option<EventData>, Error> {
    ///     match collection.get(event_id) {
    ///         Ok(data) => {
    ///             let event = bincode::deserialize(&data)
    ///                 .map_err(|e| Error::Store(format!("Invalid event data: {}", e)))?;
    ///             Ok(Some(event))
    ///         }
    ///         Err(Error::EntryNotFound(_)) => Ok(None),
    ///         Err(e) => Err(e),
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Considerations
    ///
    /// - Implement appropriate indexing for key-based lookups
    /// - Consider caching frequently accessed keys
    /// - Monitor access patterns to optimize storage layout
    /// - Use connection pooling for database backends
    fn get(&self, key: &str) -> Result<Vec<u8>, Error>;

    /// Store or update a key-value pair in the collection.
    ///
    /// Associates the given data with the specified key. If the key already exists,
    /// the previous value is replaced. The operation should be atomic to prevent
    /// corruption from concurrent updates.
    ///
    /// # Parameters
    ///
    /// * `key` - The string key to associate with the data
    /// * `data` - Raw bytes representing the serialized value
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - Key-value pair successfully stored
    /// - `Err(Store(_))` - Storage backend error occurred during write
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::Collection, Error};
    /// use serde::Serialize;
    ///
    /// async fn store_typed_data<T: Serialize>(
    ///     collection: &mut impl Collection,
    ///     key: &str,
    ///     value: &T
    /// ) -> Result<(), Error> {
    ///     let data = bincode::serialize(value)
    ///         .map_err(|e| Error::Store(format!("Serialization failed: {}", e)))?;
    ///
    ///     collection.put(key, &data)?;
    ///
    ///     // Optionally flush for immediate persistence
    ///     collection.flush()?;
    ///
    ///     Ok(())
    /// }
    ///
    /// // Batch insertion for efficiency
    /// async fn store_events_batch(
    ///     collection: &mut impl Collection,
    ///     events: Vec<(String, EventData)>
    /// ) -> Result<(), Error> {
    ///     for (key, event) in events {
    ///         let data = bincode::serialize(&event)
    ///             .map_err(|e| Error::Store(format!("Serialization failed: {}", e)))?;
    ///         collection.put(&key, &data)?;
    ///     }
    ///
    ///     // Single flush after all operations
    ///     collection.flush()?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// - Ensure atomic updates to prevent partial corruption
    /// - Maintain key ordering for iteration consistency
    /// - Consider implementing batch operations for efficiency
    /// - Handle concurrent updates appropriately
    /// - Validate key format if your backend has restrictions
    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error>;

    /// Remove a key-value pair from the collection.
    ///
    /// Deletes the entry with the specified key from the collection. If the key
    /// doesn't exist, this may return an error or be treated as a no-op depending
    /// on the implementation.
    ///
    /// # Parameters
    ///
    /// * `key` - The string key to remove from the collection
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - Key successfully removed or didn't exist
    /// - `Err(EntryNotFound(_))` - Key was not found (if implementation reports this)
    /// - `Err(Store(_))` - Storage backend error occurred during deletion
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::{database::Collection, Error};
    ///
    /// async fn cleanup_old_events(
    ///     collection: &mut impl Collection,
    ///     cutoff_timestamp: u64
    /// ) -> Result<usize, Error> {
    ///     let mut deleted_count = 0;
    ///     let cutoff_key = format!("{:020}", cutoff_timestamp);
    ///
    ///     // Collect keys to delete (separate from iteration to avoid concurrent modification)
    ///     let keys_to_delete: Vec<String> = collection.iter(false)
    ///         .take_while(|(key, _)| key < &cutoff_key)
    ///         .map(|(key, _)| key)
    ///         .collect();
    ///
    ///     // Delete collected keys
    ///     for key in keys_to_delete {
    ///         match collection.del(&key) {
    ///             Ok(()) => deleted_count += 1,
    ///             Err(Error::EntryNotFound(_)) => {}, // Already deleted, ignore
    ///             Err(e) => return Err(e),
    ///         }
    ///     }
    ///
    ///     collection.flush()?;
    ///     Ok(deleted_count)
    /// }
    /// ```
    fn del(&mut self, key: &str) -> Result<(), Error>;

    /// Get the last (highest) key-value pair in the collection.
    ///
    /// Returns the key-value pair with the lexicographically highest key, which
    /// corresponds to the most recent entry in time-ordered collections. This is
    /// optimized for common patterns like getting the latest event or highest ID.
    ///
    /// # Returns
    ///
    /// Returns an `Option<(String, Vec<u8>)>` where:
    /// - `Some((key, data))` - The highest key and its associated data
    /// - `None` - Collection is empty
    ///
    /// # Default Implementation
    ///
    /// The default implementation uses reverse iteration to get the first (highest) entry.
    /// Backends may override this with more efficient implementations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// async fn get_latest_event(
    ///     collection: &impl Collection
    /// ) -> Result<Option<(u64, EventData)>, Error> {
    ///     match collection.last() {
    ///         Some((key, data)) => {
    ///             let event_id: u64 = key.parse()
    ///                 .map_err(|e| Error::Store(format!("Invalid event ID: {}", e)))?;
    ///             let event: EventData = bincode::deserialize(&data)
    ///                 .map_err(|e| Error::Store(format!("Invalid event data: {}", e)))?;
    ///             Ok(Some((event_id, event)))
    ///         }
    ///         None => Ok(None),
    ///     }
    /// }
    ///
    /// async fn get_next_sequence_id(collection: &impl Collection) -> u64 {
    ///     match collection.last() {
    ///         Some((key, _)) => {
    ///             key.parse::<u64>().unwrap_or(0) + 1
    ///         }
    ///         None => 0, // Start from 0 for empty collection
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Notes
    ///
    /// - This operation should be O(1) or O(log n) depending on backend
    /// - Consider caching the result if called frequently
    /// - Some backends may optimize this with special indexing
    fn last(&self) -> Option<(String, Vec<u8>)> {
        let mut iter = self.iter(true);
        let value = iter.next();
        debug!("Last value: {:?}", value);
        value
    }

    /// Remove all key-value pairs from the collection.
    ///
    /// Performs a complete cleanup of all data in the collection. This operation
    /// is irreversible and should be used with caution. It's typically used during
    /// testing, development, or maintenance operations.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - All data successfully purged
    /// - `Err(Store(_))` - Storage backend error occurred during purge
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// async fn reset_event_log(collection: &mut impl Collection) -> Result<(), Error> {
    ///     println!("Purging all events from: {}", collection.name());
    ///     collection.purge()?;
    ///     println!("Event log reset complete");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Warning
    ///
    /// This operation is destructive and irreversible. Ensure you have appropriate
    /// backups or confirmation mechanisms before calling this in production.
    ///
    /// # Use Cases
    ///
    /// - Test cleanup and isolation between test runs
    /// - Development environment resets
    /// - Maintenance operations with full data lifecycle management
    /// - Emergency cleanup procedures
    fn purge(&mut self) -> Result<(), Error>;

    /// Create an iterator over all key-value pairs in the collection.
    ///
    /// Returns an iterator that yields key-value pairs in either forward (ascending)
    /// or reverse (descending) key order. The iterator supports efficient traversal
    /// of large collections and is the foundation for range queries and batch processing.
    ///
    /// # Parameters
    ///
    /// * `reverse` - If `true`, iterate in descending key order; if `false`, ascending order
    ///
    /// # Returns
    ///
    /// Returns a boxed iterator yielding `(String, Vec<u8>)` pairs representing
    /// keys and their associated data in the requested order.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// async fn process_all_events(collection: &impl Collection) -> Result<(), Error> {
    ///     println!("Processing events in chronological order:");
    ///     for (key, data) in collection.iter(false) {
    ///         let event: EventData = bincode::deserialize(&data)
    ///             .map_err(|e| Error::Store(format!("Invalid event: {}", e)))?;
    ///         process_event(&key, &event).await?;
    ///     }
    ///     Ok(())
    /// }
    ///
    /// async fn get_recent_events(
    ///     collection: &impl Collection,
    ///     limit: usize
    /// ) -> Result<Vec<EventData>, Error> {
    ///     let mut events = Vec::new();
    ///
    ///     // Use reverse iteration to get most recent first
    ///     for (_, data) in collection.iter(true).take(limit) {
    ///         let event: EventData = bincode::deserialize(&data)
    ///             .map_err(|e| Error::Store(format!("Invalid event: {}", e)))?;
    ///         events.push(event);
    ///     }
    ///
    ///     events.reverse(); // Return in chronological order
    ///     Ok(events)
    /// }
    ///
    /// async fn find_events_by_prefix(
    ///     collection: &impl Collection,
    ///     prefix: &str
    /// ) -> Vec<(String, Vec<u8>)> {
    ///     collection.iter(false)
    ///         .filter(|(key, _)| key.starts_with(prefix))
    ///         .collect()
    /// }
    /// ```
    ///
    /// # Implementation Notes
    ///
    /// - Iterator should be lazy and not load all data into memory at once
    /// - Support efficient range scans for database backends
    /// - Handle concurrent modifications gracefully (snapshot or consistent view)
    /// - Consider implementing specialized range iterators for better performance
    /// - Ensure proper resource cleanup when iterator is dropped
    ///
    /// # Performance Considerations
    ///
    /// - Large collections: Iterator should not materialize all data upfront
    /// - Database backends: Use appropriate cursor or stream APIs
    /// - Memory usage: Implement streaming rather than collecting all results
    /// - Concurrent access: Consider read consistency guarantees needed
    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>;

    /// Ensure all pending changes are persisted to durable storage.
    ///
    /// Forces any buffered writes, pending transactions, or cached data to be
    /// committed to persistent storage. This provides stronger durability guarantees
    /// at the cost of potentially reduced performance. The default implementation
    /// performs no operation, suitable for backends that don't buffer writes.
    ///
    /// # Returns
    ///
    /// Returns a `Result<(), Error>` where:
    /// - `Ok(())` - All data successfully flushed to persistent storage
    /// - `Err(Store(_))` - Flush operation failed, some data may be lost
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// async fn store_critical_events(
    ///     collection: &mut impl Collection,
    ///     events: Vec<(String, Vec<u8>)>
    /// ) -> Result<(), Error> {
    ///     // Store all events
    ///     for (key, data) in events {
    ///         collection.put(&key, &data)?;
    ///     }
    ///
    ///     // Ensure they're all persisted before continuing
    ///     collection.flush()?;
    ///
    ///     println!("All critical events safely persisted");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # When to Use
    ///
    /// - After storing critical data that must survive system crashes
    /// - Before system shutdown to ensure data consistency
    /// - In testing to ensure deterministic persistence behavior
    /// - When implementing synchronous semantics over asynchronous storage
    /// - After batch operations to ensure all changes are durable
    fn flush(&self) -> Result<(), Error> {
        Ok(())
    }

    /// Retrieve a range of values from the collection.
    ///
    /// Returns values from the collection within a specified range, supporting both
    /// forward and reverse direction iteration. This is useful for pagination,
    /// time-based queries, or processing data in chunks.
    ///
    /// # Parameters
    ///
    /// * `from` - Starting key for the range (exclusive). If `None`, starts from beginning/end
    /// * `quantity` - Number of values to return. Positive for forward, negative for reverse
    ///
    /// # Returns
    ///
    /// Returns a `Result<Vec<Vec<u8>>, Error>` where:
    /// - `Ok(values)` - Vector of values in the requested range and direction
    /// - `Err(EntryNotFound(_))` - Starting key was not found
    /// - `Err(Store(_))` - Storage backend error occurred during range query
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_store::database::Collection;
    ///
    /// async fn paginate_events(
    ///     collection: &impl Collection,
    ///     last_seen_key: Option<String>,
    ///     page_size: usize
    /// ) -> Result<Vec<EventData>, Error> {
    ///     let values = collection.get_by_range(last_seen_key, page_size as isize)?;
    ///
    ///     let mut events = Vec::new();
    ///     for value in values {
    ///         let event: EventData = bincode::deserialize(&value)
    ///             .map_err(|e| Error::Store(format!("Invalid event: {}", e)))?;
    ///         events.push(event);
    ///     }
    ///
    ///     Ok(events)
    /// }
    ///
    /// async fn get_recent_events_before(
    ///     collection: &impl Collection,
    ///     before_key: String,
    ///     count: usize
    /// ) -> Result<Vec<Vec<u8>>, Error> {
    ///     // Negative quantity for reverse direction
    ///     collection.get_by_range(Some(before_key), -(count as isize))
    /// }
    /// ```
    ///
    /// # Default Implementation
    ///
    /// The default implementation uses the iterator interface to provide range functionality.
    /// Backends may override this with more efficient range query implementations.
    ///
    /// # Behavior Details
    ///
    /// - **Exclusive Range**: The `from` key is excluded from results
    /// - **Direction**: Positive quantity = forward, negative = reverse
    /// - **Quantity**: Absolute value determines maximum number of results
    /// - **Early Termination**: Returns fewer results if end of collection is reached
    ///
    /// # Performance Notes
    ///
    /// - Database backends should implement efficient range scans
    /// - Consider implementing server-side filtering for large datasets
    /// - Use appropriate indexing strategies for range queries
    /// - Monitor query patterns to optimize storage layout
    fn get_by_range(
        &self,
        from: Option<String>,
        quantity: isize,
    ) -> Result<Vec<Vec<u8>>, Error> {
        fn convert<'a>(
            iter: impl Iterator<Item = (String, Vec<u8>)> + 'a,
        ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
            Box::new(iter)
        }
        let (mut iter, quantity) = match from {
            Some(key) => {
                // Find the key
                let iter = if quantity >= 0 {
                    self.iter(false)
                } else {
                    self.iter(true)
                };
                let mut iter = iter.peekable();
                loop {
                    let Some((current_key, _)) = iter.peek() else {
                        return Err(Error::EntryNotFound("None".to_owned()));
                    };
                    if current_key == &key {
                        break;
                    }
                    iter.next();
                }
                iter.next(); // Exclusive From
                (convert(iter), quantity.abs())
            }
            None => {
                if quantity >= 0 {
                    (self.iter(false), quantity)
                } else {
                    (self.iter(true), quantity.abs())
                }
            }
        };
        let mut result = Vec::new();
        let mut counter = 0;
        while counter < quantity {
            let Some((_, event)) = iter.next() else {
                break;
            };
            result.push(event);
            counter += 1;
        }
        Ok(result)
    }
}

#[macro_export]
macro_rules! test_store_trait {
    ($name:ident: $type:ty: $type2:ty) => {
        #[cfg(test)]
        mod $name {
            use super::*;
            use $crate::error::Error;

            #[test]
            fn test_create_collection() {
                let manager = <$type>::default();
                let store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                assert_eq!(Collection::name(&store), "test");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_create_state() {
                let manager = <$type>::default();
                let store: $type2 =
                    manager.create_state("test", "test").unwrap();
                assert_eq!(State::name(&store), "test");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_put_get_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key", b"value").unwrap();
                assert_eq!(Collection::get(&store, "key").unwrap(), b"value");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_put_get_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value").unwrap();
                assert_eq!(State::get(&store).unwrap(), b"value");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_del_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key", b"value").unwrap();
                Collection::del(&mut store, "key").unwrap();
                assert_eq!(
                    Collection::get(&store, "key"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_del_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value").unwrap();
                State::del(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_iter() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                let mut iter = store.iter(false);
                assert_eq!(
                    iter.next(),
                    Some(("key1".to_string(), b"value1".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key2".to_string(), b"value2".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert_eq!(iter.next(), None);
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_iter_reverse() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                let mut iter = store.iter(true);
                assert_eq!(
                    iter.next(),
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key2".to_string(), b"value2".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key1".to_string(), b"value1".to_vec()))
                );
                assert_eq!(iter.next(), None);
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_last() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                let last = store.last();
                assert_eq!(
                    last,
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_get_by_range() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                let result = store.get_by_range(None, 2).unwrap();
                assert_eq!(
                    result,
                    vec![b"value1".to_vec(), b"value2".to_vec()]
                );
                let result =
                    store.get_by_range(Some("key3".to_string()), -2).unwrap();
                assert_eq!(
                    result,
                    vec![b"value2".to_vec(), b"value1".to_vec()]
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_purge_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                assert_eq!(
                    Collection::get(&store, "key1"),
                    Ok(b"value1".to_vec())
                );
                assert_eq!(
                    Collection::get(&store, "key2"),
                    Ok(b"value2".to_vec())
                );
                assert_eq!(
                    Collection::get(&store, "key3"),
                    Ok(b"value3".to_vec())
                );
                Collection::purge(&mut store).unwrap();
                assert_eq!(
                    Collection::get(&store, "key1"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert_eq!(
                    Collection::get(&store, "key2"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert_eq!(
                    Collection::get(&store, "key3"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_purge_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value1").unwrap();
                assert_eq!(State::get(&store), Ok(b"value1".to_vec()));
                State::purge(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );

                State::put(&mut store, b"value2").unwrap();
                assert_eq!(State::get(&store), Ok(b"value2".to_vec()));
                State::purge(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }
        }
    };
}
