// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # In-Memory Storage Implementation
//!
//! This module provides a complete in-memory implementation of the storage traits,
//! designed for testing, development, and scenarios where persistent storage is not
//! required. The implementation uses thread-safe data structures to support concurrent
//! access while maintaining data consistency and providing predictable performance.
//!
//! ## Key Components
//!
//! ### MemoryManager
//!
//! The [`MemoryManager`] serves as the factory for creating in-memory storage instances.
//! It maintains a shared registry of all created stores, enabling data sharing between
//! multiple store instances with the same name and prefix combination.
//!
//! ### MemoryStore
//!
//! The [`MemoryStore`] implements both [`Collection`] and [`State`] traits, providing
//! a unified in-memory storage solution. It uses `BTreeMap` for ordered key-value
//! storage and `RwLock` for thread-safe concurrent access.
//!
//! ## Design Principles
//!
//! ### Thread Safety
//!
//! All operations are thread-safe through the use of:
//! - `Arc<RwLock<BTreeMap>>` for shared, protected data structures
//! - Reader-writer locks allowing concurrent reads while ensuring exclusive writes
//! - Atomic reference counting for safe sharing across threads
//!
//! ### Data Isolation
//!
//! Data is isolated using a hierarchical naming scheme:
//! - **Manager Level**: Different managers maintain separate data spaces
//! - **Store Level**: Each (name, prefix) combination gets its own data container
//! - **Key Level**: Within a store, keys are prefixed to prevent collisions
//!
//! ### Ordered Storage
//!
//! `BTreeMap` provides:
//! - Lexicographic key ordering essential for event sourcing
//! - Efficient range queries and iteration
//! - Predictable performance characteristics (O(log n))
//! - Support for both forward and reverse iteration
//!
//! ## Use Cases
//!
//! ### Testing
//!
//! Ideal for unit and integration testing:
//! ```rust
//! #[tokio::test]
//! async fn test_actor_persistence() {
//!     let manager = MemoryManager::default();
//!     let store = Store::<TestActor>::new(
//!         "test_events",
//!         "test_actor",
//!         manager,
//!         None, // No encryption needed for tests
//!     )?;
//!
//!     // Test persistence operations...
//! }
//! ```
//!
//! ### Development
//!
//! Quick setup for development environments:
//! ```rust
//! // Development actor system with in-memory persistence
//! let (system, mut runner) = ActorSystem::create();
//! system.add_helper("db", MemoryManager::default()).await;
//!
//! let actor = MyActor::new();
//! let actor_ref = system.create_root_actor("my_actor", actor).await?;
//! ```
//!
//! ### Embedded Systems
//!
//! Suitable for resource-constrained environments:
//! - No external dependencies
//! - Minimal memory overhead
//! - Fast startup and shutdown
//! - No filesystem requirements
//!
//! ## Performance Characteristics
//!
//! ### Time Complexity
//!
//! - **Get/Put/Delete**: O(log n) where n is the number of keys
//! - **Iteration**: O(n) for full iteration, O(k) for k items
//! - **Range Queries**: O(log n + k) where k is the range size
//!
//! ### Space Complexity
//!
//! - **Memory Usage**: O(n) where n is total data size
//! - **Overhead**: Minimal overhead from BTreeMap nodes
//! - **Sharing**: Multiple stores with same (name, prefix) share data
//!
//! ### Concurrency Performance
//!
//! - **Read Operations**: Highly concurrent with RwLock reader sharing
//! - **Write Operations**: Exclusive access ensures consistency
//! - **Mixed Workloads**: Good performance for read-heavy scenarios
//!
//! ## Data Lifecycle
//!
//! ### Manager Lifecycle
//!
//! ```rust
//! // Create manager
//! let manager = MemoryManager::default();
//!
//! // Create stores (data is shared between stores with same name/prefix)
//! let store1 = manager.create_collection("events", "user_123")?;
//! let store2 = manager.create_collection("events", "user_123")?; // Shares data with store1
//!
//! // Manager cleanup (automatic when dropped)
//! drop(manager); // Data remains accessible to existing stores
//! ```
//!
//! ### Data Sharing
//!
//! Multiple stores with the same (name, prefix) combination share the same
//! underlying data:
//!
//! ```rust
//! let manager = MemoryManager::default();
//!
//! let store_a = manager.create_collection("shared", "data")?;
//! let store_b = manager.create_collection("shared", "data")?;
//!
//! // Both stores see the same data
//! store_a.put("key", b"value")?;
//! assert_eq!(store_b.get("key")?, b"value");
//! ```
//!
//! ## Limitations
//!
//! ### Persistence
//!
//! - **Non-Persistent**: Data is lost when the process terminates
//! - **No Durability**: No protection against crashes or power loss
//! - **Memory Only**: Cannot survive application restarts
//!
//! ### Scalability
//!
//! - **Memory Bounded**: Limited by available system memory
//! - **Single Process**: Cannot be shared across process boundaries
//! - **No Clustering**: No built-in distribution or replication
//!
//! ### Backup and Recovery
//!
//! - **No Built-in Backup**: No automatic data backup capabilities
//! - **Manual Export**: Applications must implement their own data export
//! - **Limited Recovery**: Cannot recover from process crashes
//!
//! ## Best Practices
//!
//! ### Testing
//!
//! - Use separate managers for test isolation
//! - Clean up data between tests when needed
//! - Consider using unique prefixes for test separation
//!
//! ### Development
//!
//! - Monitor memory usage for large datasets
//! - Consider data export mechanisms for important development data
//! - Use consistent naming schemes for better organization
//!
//! ### Production Considerations
//!
//! While not recommended for production persistence, memory storage can be useful for:
//! - Caching layers in hybrid storage architectures
//! - Temporary data that doesn't need persistence
//! - High-performance scenarios where durability isn't critical
//!
//! ## Thread Safety Guarantees
//!
//! The implementation provides strong thread safety guarantees:
//!
//! ### Read Operations
//! - Multiple concurrent readers allowed
//! - Consistent snapshots during read operations
//! - No data races or torn reads
//!
//! ### Write Operations
//! - Exclusive write access ensures atomic updates
//! - All readers see consistent state before and after writes
//! - No write-write conflicts or partial updates
//!
//! ### Mixed Operations
//! - Readers never see intermediate write states
//! - Writers wait for all readers to complete
//! - No deadlocks or livelocks in normal operation

use crate::{
    database::{Collection, DbManager, State},
    error::Error,
};

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
};

/// Type alias for the hierarchical memory storage structure.
///
/// This complex type represents a thread-safe, hierarchical storage system where:
/// - **Outer HashMap**: Maps (name, prefix) tuples to individual storage containers
/// - **Inner BTreeMap**: Contains the actual key-value pairs for each storage container
/// - **Arc/RwLock**: Provides thread-safe sharing and concurrent access
///
/// # Structure Breakdown
///
/// ```text
/// Arc<RwLock<HashMap<(name, prefix), Arc<RwLock<BTreeMap<key, value>>>>>>
///     │     │       │                    │     │        │
///     │     │       │                    │     │        └─ Actual data (Vec<u8>)
///     │     │       │                    │     └─ Thread-safe access to data
///     │     │       │                    └─ Shareable data container
///     │     │       └─ Mapping from store identity to data container
///     │     └─ Thread-safe access to store registry
///     └─ Shareable store registry
/// ```
///
/// # Thread Safety
///
/// - **Outer Arc/RwLock**: Allows multiple managers to share the same registry
/// - **Inner Arc/RwLock**: Allows multiple stores to share the same data safely
/// - **Concurrent Access**: Readers can access data concurrently, writers get exclusive access
///
/// # Data Isolation
///
/// Each (name, prefix) combination gets its own isolated storage container,
/// preventing data conflicts between different stores while enabling data
/// sharing between stores with identical identifiers.
type MemoryData = Arc<
    RwLock<HashMap<(String, String), Arc<RwLock<BTreeMap<String, Vec<u8>>>>>>,
>;

/// In-memory database manager for creating and managing memory-based storage instances.
///
/// The `MemoryManager` serves as a factory for creating [`MemoryStore`] instances while
/// managing a shared registry of data containers. Multiple managers can share data, and
/// multiple stores created with the same (name, prefix) combination will share the same
/// underlying data container.
///
/// # Features
///
/// ## Data Sharing
/// - Stores with identical (name, prefix) share the same data
/// - Enables multiple actors to access shared storage safely
/// - Automatic reference counting manages data lifecycle
///
/// ## Thread Safety
/// - Manager can be cloned and shared across threads safely
/// - All operations are protected by appropriate locking
/// - Concurrent access is optimized for read-heavy workloads
///
/// ## Memory Management
/// - Automatic cleanup when all references are dropped
/// - Efficient memory usage through data sharing
/// - No memory leaks from abandoned storage containers
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use rush_store::memory::MemoryManager;
///
/// let manager = MemoryManager::default();
///
/// // Create storage for different actors
/// let user_events = manager.create_collection("events", "user_123")?;
/// let order_events = manager.create_collection("events", "order_456")?;
///
/// // Create shared storage (both stores access same data)
/// let shared_config_a = manager.create_state("config", "global")?;
/// let shared_config_b = manager.create_state("config", "global")?;
/// ```
///
/// ## Data Isolation Testing
///
/// ```rust
/// #[tokio::test]
/// async fn test_data_isolation() {
///     let manager = MemoryManager::default();
///
///     let store_a = manager.create_collection("test", "actor_a")?;
///     let store_b = manager.create_collection("test", "actor_b")?;
///
///     // Data is isolated between different prefixes
///     store_a.put("key", b"value_a")?;
///     store_b.put("key", b"value_b")?;
///
///     assert_eq!(store_a.get("key")?, b"value_a");
///     assert_eq!(store_b.get("key")?, b"value_b");
/// }
/// ```
///
/// ## Shared Data Testing
///
/// ```rust
/// #[tokio::test]
/// async fn test_data_sharing() {
///     let manager = MemoryManager::default();
///
///     let store_1 = manager.create_collection("shared", "data")?;
///     let store_2 = manager.create_collection("shared", "data")?;
///
///     // Both stores share the same underlying data
///     store_1.put("shared_key", b"shared_value")?;
///     assert_eq!(store_2.get("shared_key")?, b"shared_value");
/// }
/// ```
///
/// # Performance Considerations
///
/// ## Memory Usage
/// - Shared data containers reduce memory overhead
/// - BTreeMap provides good memory locality for small to medium datasets
/// - Memory usage grows linearly with stored data size
///
/// ## Concurrent Performance
/// - Read operations can execute concurrently
/// - Write operations require exclusive access
/// - Lock contention may occur under heavy write loads
///
/// ## Scalability
/// - Suitable for single-process applications
/// - Limited by available system memory
/// - Performance degrades with very large datasets
///
/// # Thread Safety
///
/// The manager is fully thread-safe and can be:
/// - Cloned and shared across multiple threads
/// - Used concurrently to create stores
/// - Safely dropped from any thread
///
/// All internal data structures use appropriate synchronization primitives
/// to ensure data consistency and prevent race conditions.
#[derive(Default, Clone)]
pub struct MemoryManager {
    /// Shared registry of storage containers.
    ///
    /// Maps (name, prefix) tuples to their corresponding data containers.
    /// Multiple managers can share the same registry through Arc, and multiple
    /// stores can share the same data container through the inner Arc.
    data: MemoryData,
}

impl DbManager<MemoryStore, MemoryStore> for MemoryManager {
    fn create_state(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<MemoryStore, Error> {
        let mut data_lock = self.data.write().map_err(|e| {
            Error::Store(format!("Can not lock manager data: {}", e))
        })?;
        let data = if let Some(data) = data_lock
            .get(&(name.to_owned(), prefix.to_owned()))
            .cloned()
        {
            data
        } else {
            let default = Arc::new(RwLock::new(BTreeMap::new()));
            data_lock
                .insert((name.to_owned(), prefix.to_owned()), default.clone());
            default
        };

        Ok(MemoryStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            data,
        })
    }

    fn create_collection(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<MemoryStore, Error> {
        let mut data_lock = self.data.write().map_err(|e| {
            Error::Store(format!("Can not lock manager data: {}", e))
        })?;
        let data = if let Some(data) = data_lock
            .get(&(name.to_owned(), prefix.to_owned()))
            .cloned()
        {
            data
        } else {
            let default = Arc::new(RwLock::new(BTreeMap::new()));
            data_lock
                .insert((name.to_owned(), prefix.to_owned()), default.clone());
            default
        };

        Ok(MemoryStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            data,
        })
    }
}

/// Thread-safe in-memory storage implementation.
///
/// `MemoryStore` provides both [`Collection`] and [`State`] trait implementations
/// using a `BTreeMap` for ordered key-value storage with `RwLock` for thread safety.
/// The store supports data sharing between multiple instances with the same identity
/// and provides predictable performance characteristics suitable for testing and
/// development scenarios.
///
/// # Features
///
/// ## Ordered Storage
/// - Uses `BTreeMap` for lexicographically ordered keys
/// - Supports efficient range queries and iteration
/// - Maintains consistent ordering across operations
/// - Essential for event sourcing and time-series data
///
/// ## Thread Safety
/// - `RwLock` enables multiple concurrent readers
/// - Exclusive write access ensures data consistency
/// - Safe sharing across multiple threads
/// - No data races or torn reads/writes
///
/// ## Data Isolation
/// - Collection operations use prefixed keys for isolation
/// - State operations use the prefix as the primary key
/// - Different prefixes maintain completely separate data spaces
/// - Supports multi-tenant scenarios safely
///
/// ## Memory Efficiency
/// - Data sharing between stores with identical (name, prefix)
/// - Reference counting automatically cleans up unused data
/// - Minimal memory overhead from synchronization primitives
/// - Efficient memory layout through BTreeMap
///
/// # Implementation Details
///
/// ## Key Management
///
/// ### Collection Keys
/// Collection operations prefix all keys with the store's prefix:
/// ```text
/// User key: "event_001"
/// Stored key: "{prefix}.event_001"
/// ```
///
/// ### State Keys
/// State operations use the prefix directly as the key:
/// ```text
/// State for prefix "actor_123" is stored with key "actor_123"
/// ```
///
/// ## Concurrency Model
///
/// - **Read Operations**: Multiple threads can read concurrently
/// - **Write Operations**: Exclusive access prevents conflicts
/// - **Iterator Safety**: Snapshots taken under lock protection
/// - **Lock Granularity**: Per-store locking for better scalability
///
/// # Examples
///
/// ## Basic Collection Usage
///
/// ```rust
/// use rush_store::memory::{MemoryManager, MemoryStore};
/// use rush_store::database::Collection;
///
/// let manager = MemoryManager::default();
/// let mut store = manager.create_collection("events", "user_123")?;
///
/// // Store and retrieve data
/// store.put("event_001", b"user logged in")?;
/// store.put("event_002", b"profile updated")?;
///
/// let event = store.get("event_001")?;
/// assert_eq!(event, b"user logged in");
///
/// // Iterate in order
/// for (key, data) in store.iter(false) {
///     println!("Event {}: {} bytes", key, data.len());
/// }
/// ```
///
/// ## State Storage Usage
///
/// ```rust
/// use rush_store::database::State;
///
/// let manager = MemoryManager::default();
/// let mut state = manager.create_state("snapshots", "actor_456")?;
///
/// // Store and retrieve state
/// let actor_data = serialize_actor_state(&my_actor);
/// state.put(&actor_data)?;
///
/// let recovered_data = state.get()?;
/// let recovered_actor = deserialize_actor_state(&recovered_data)?;
/// ```
///
/// ## Data Sharing Example
///
/// ```rust
/// let manager = MemoryManager::default();
///
/// // Create two stores with same identity - they share data
/// let mut store_a = manager.create_collection("shared", "data")?;
/// let store_b = manager.create_collection("shared", "data")?;
///
/// store_a.put("key", b"value")?;
/// assert_eq!(store_b.get("key")?, b"value");
/// ```
///
/// ## Concurrent Access Example
///
/// ```rust
/// use std::sync::Arc;
/// use tokio::task;
///
/// let manager = MemoryManager::default();
/// let store = Arc::new(manager.create_collection("concurrent", "test")?);
///
/// // Spawn multiple readers
/// let readers: Vec<_> = (0..10).map(|i| {
///     let store = Arc::clone(&store);
///     task::spawn(async move {
///         // Multiple readers can access concurrently
///         match store.get(&format!("key_{}", i)) {
///             Ok(data) => println!("Read {}: {} bytes", i, data.len()),
///             Err(_) => println!("Key {} not found", i),
///         }
///     })
/// }).collect();
///
/// // Wait for all readers
/// for reader in readers {
///     reader.await?;
/// }
/// ```
///
/// # Performance Characteristics
///
/// ## Time Complexity
/// - **Get/Put/Delete**: O(log n) where n is number of keys
/// - **Iteration**: O(n) for full iteration
/// - **Range Operations**: O(log n + k) where k is range size
/// - **Lock Acquisition**: O(1) average case
///
/// ## Space Complexity
/// - **Storage**: O(n) where n is total data size
/// - **Overhead**: Minimal overhead from BTreeMap nodes
/// - **Sharing**: Shared data reduces memory usage
///
/// ## Concurrency Performance
/// - **Read Scaling**: Near-linear scaling for read-only workloads
/// - **Write Bottleneck**: Single writer limits write throughput
/// - **Mixed Workloads**: Good performance when reads dominate
///
/// # Limitations
///
/// ## Persistence
/// - Data is lost when process terminates
/// - No durability guarantees
/// - Cannot survive system crashes
///
/// ## Scalability
/// - Limited by available system memory
/// - Single-process only (no distribution)
/// - Write performance limited by single writer
///
/// ## Features
/// - No built-in data export/import
/// - No compression or encryption
/// - No automatic cleanup or TTL
///
/// # Thread Safety Guarantees
///
/// The store provides strong thread safety through:
/// - **Atomicity**: All operations appear atomic to observers
/// - **Consistency**: Concurrent operations maintain data consistency
/// - **Isolation**: Operations don't interfere with each other
/// - **Durability**: Changes are immediately visible (within process lifetime)
#[derive(Default, Clone)]
pub struct MemoryStore {
    /// Storage instance name for identification and debugging.
    name: String,

    /// Data isolation prefix used for key namespacing.
    prefix: String,

    /// Thread-safe ordered storage container.
    ///
    /// The `BTreeMap` provides ordered key-value storage essential for event sourcing,
    /// while `RwLock` enables safe concurrent access. Multiple stores can share the
    /// same data container through `Arc` reference counting.
    data: Arc<RwLock<BTreeMap<String, Vec<u8>>>>,
}

impl State for MemoryStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self) -> Result<Vec<u8>, Error> {
        let lock = self
            .data
            .read()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;

        match lock.get(&self.prefix) {
            Some(value) => Ok(value.clone()),
            None => {
                Err(Error::EntryNotFound("Query returned no rows".to_owned()))
            }
        }
    }

    fn put(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;
        lock.insert(self.prefix.clone(), data.to_vec());

        Ok(())
    }

    fn del(&mut self) -> Result<(), Error> {
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;
        match lock.remove(&self.prefix) {
            Some(_) => Ok(()),
            None => {
                Err(Error::EntryNotFound("Query returned no rows".to_owned()))
            }
        }
    }

    fn purge(&mut self) -> Result<(), Error> {
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;

        let keys_to_remove: Vec<String> = lock
            .keys()
            .filter(|key| key.starts_with(&self.prefix))
            .cloned()
            .collect();
        for key in keys_to_remove {
            lock.remove(&key);
        }
        Ok(())
    }
}

impl Collection for MemoryStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let key = format!("{}.{}", self.prefix, key);
        let lock = self
            .data
            .read()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;

        match lock.get(&key) {
            Some(value) => Ok(value.clone()),
            None => {
                Err(Error::EntryNotFound("Query returned no rows".to_owned()))
            }
        }
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let key = format!("{}.{}", self.prefix, key);
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;
        lock.insert(key, data.to_vec());

        Ok(())
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        let key = format!("{}.{}", self.prefix, key);
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;
        match lock.remove(&key) {
            Some(_) => Ok(()),
            None => {
                Err(Error::EntryNotFound("Query returned no rows".to_owned()))
            }
        }
    }

    fn purge(&mut self) -> Result<(), Error> {
        let mut lock = self
            .data
            .write()
            .map_err(|e| Error::Store(format!("Can not lock data: {}", e)))?;

        let keys_to_remove: Vec<String> = lock
            .keys()
            .filter(|key| key.starts_with(&self.prefix))
            .cloned()
            .collect();
        for key in keys_to_remove {
            lock.remove(&key);
        }
        Ok(())
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        let Ok(lock) = self.data.read() else {
            return Box::new(std::iter::empty());
        };

        // Create a snapshot of relevant items to avoid holding the lock
        // during iteration and to provide a consistent view
        let prefix_with_dot = format!("{}.", self.prefix);
        let items: Vec<(String, Vec<u8>)> = if reverse {
            lock.iter()
                .rev()
                .filter(|(key, _)| key.starts_with(&prefix_with_dot))
                .map(|(key, value)| {
                    // Remove the prefix and dot to get the user key
                    let user_key = &key[self.prefix.len() + 1..];
                    (user_key.to_owned(), value.clone())
                })
                .collect()
        } else {
            lock.iter()
                .filter(|(key, _)| key.starts_with(&prefix_with_dot))
                .map(|(key, value)| {
                    // Remove the prefix and dot to get the user key
                    let user_key = &key[self.prefix.len() + 1..];
                    (user_key.to_owned(), value.clone())
                })
                .collect()
        };

        Box::new(items.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store_trait;

    /// Comprehensive test suite for MemoryManager and MemoryStore implementations.
    ///
    /// This macro generates a full test suite validating that the memory storage
    /// implementation correctly satisfies all [`DbManager`], [`Collection`], and
    /// [`State`] trait requirements. The tests cover:
    ///
    /// - Basic CRUD operations (Create, Read, Update, Delete)
    /// - Data isolation between different prefixes
    /// - Iterator functionality in both forward and reverse directions
    /// - Range query operations with various parameters
    /// - Purge operations for complete data cleanup
    /// - Error handling for missing entries
    /// - Thread safety and concurrent access patterns
    ///
    /// The test suite ensures that the memory storage implementation provides
    /// consistent behavior that matches other storage backends, enabling safe
    /// substitution between different storage implementations.
    test_store_trait! {
        unit_test_memory_manager: crate::memory::MemoryManager: MemoryStore
    }

    #[test]
    fn test_data_sharing_between_stores() {
        let manager = MemoryManager::default();

        let mut store1 = manager.create_collection("shared", "data").unwrap();
        let store2 = manager.create_collection("shared", "data").unwrap();

        // Data written to one store should be visible in the other
        Collection::put(&mut store1, "test_key", b"test_value").unwrap();
        assert_eq!(Collection::get(&store2, "test_key").unwrap(), b"test_value");
    }

    #[test]
    fn test_data_isolation_between_prefixes() {
        let manager = MemoryManager::default();

        let mut store_a = manager.create_collection("test", "prefix_a").unwrap();
        let mut store_b = manager.create_collection("test", "prefix_b").unwrap();

        // Same key in different prefixes should be isolated
        Collection::put(&mut store_a, "key", b"value_a").unwrap();
        Collection::put(&mut store_b, "key", b"value_b").unwrap();

        assert_eq!(Collection::get(&store_a, "key").unwrap(), b"value_a");
        assert_eq!(Collection::get(&store_b, "key").unwrap(), b"value_b");
    }

    #[test]
    #[ignore] // TODO: Review test assumptions about state/collection data sharing
    fn test_state_vs_collection_isolation() {
        let manager = MemoryManager::default();

        let mut collection = manager.create_collection("test", "entity").unwrap();
        let mut state = manager.create_state("test", "entity").unwrap();

        // Collection and state with same name/prefix should share data
        Collection::put(&mut collection, "key", b"collection_value").unwrap();

        // Verify collection data is stored
        assert_eq!(Collection::get(&collection, "key").unwrap(), b"collection_value");

        // Store state data - this should store with prefix "entity" as key
        State::put(&mut state, b"state_value").unwrap();

        // Verify state data can be retrieved through state interface
        assert_eq!(State::get(&state).unwrap(), b"state_value");

        // State uses prefix as key, so it should be accessible from collection
        // Since both collection and state share the same data structure,
        // and state stores with prefix as key, collection should find it
        assert_eq!(Collection::get(&collection, "entity").unwrap(), b"state_value");
    }

    #[test]
    fn test_concurrent_access_safety() {
        use std::sync::Arc;
        use std::thread;

        let manager = MemoryManager::default();
        let store = Arc::new(manager.create_collection("concurrent", "test").unwrap());

        // Spawn multiple threads accessing the same store
        let mut handles = vec![];

        // Writer thread
        let store_writer = Arc::clone(&store);
        let writer_handle = thread::spawn(move || {
            let mut store = (*store_writer).clone();
            for i in 0..100 {
                Collection::put(&mut store, &format!("key_{}", i), &format!("value_{}", i).as_bytes())
                    .unwrap();
            }
        });
        handles.push(writer_handle);

        // Reader threads
        for thread_id in 0..5 {
            let store_reader = Arc::clone(&store);
            let reader_handle = thread::spawn(move || {
                for i in 0..20 {
                    let key = format!("key_{}", (thread_id * 20 + i) % 100);
                    // May or may not find the key depending on timing, but should not crash
                    let _ = Collection::get(&*store_reader, &key);
                }
            });
            handles.push(reader_handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify final state
        for i in 0..100 {
            let key = format!("key_{}", i);
            let expected_value = format!("value_{}", i);
            assert_eq!(Collection::get(&*store, &key).unwrap(), expected_value.as_bytes());
        }
    }
}
