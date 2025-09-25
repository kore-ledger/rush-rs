// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Rush SQLite Database Backend
//!
//! A high-performance SQLite storage backend implementation for the Rush actor system's
//! persistent storage layer. This module provides a complete SQLite-based storage solution
//! with advanced performance optimizations, security features, and comprehensive concurrency
//! handling designed for production actor workloads.
//!
//! ## Overview
//!
//! The SQLite backend implements the store system's pluggable storage abstraction,
//! providing persistent storage for actor events and state with SQLite's proven reliability
//! and performance characteristics. This implementation leverages SQLite's advanced features
//! including Write-Ahead Logging (WAL), memory-mapped I/O, and automatic query optimization
//! to deliver excellent performance for concurrent actor systems.
//!
//! Key features of this SQLite backend include:
//!
//! - **ACID Compliance**: Full transaction support with SQLite's ACID guarantees
//! - **WAL Mode**: Write-Ahead Logging for improved concurrency and crash recovery
//! - **Memory-Mapped I/O**: Large files accessed efficiently through OS virtual memory
//! - **Automatic Optimization**: Query plan optimization with adaptive statistics
//! - **Security Hardening**: SQL injection prevention and identifier validation
//! - **Performance Monitoring**: Built-in query counting and optimization tracking
//! - **Connection Pooling**: Thread-safe connection management with mutex protection
//! - **Schema Isolation**: Table-based isolation with configurable prefixes
//!
//! ## Architecture
//!
//! The SQLite backend follows the store system's architectural patterns:
//!
//! ### Database Management
//!
//! The [`SqliteManager`] implements the [`DbManager`] trait, providing the factory
//! interface for creating storage collections and state containers. It manages:
//!
//! - Database connection lifecycle and configuration
//! - Performance monitoring and automatic optimization
//! - Thread-safe access coordination
//! - File system integration and directory management
//!
//! ### Storage Collections
//!
//! The [`SqliteCollection`] implements both [`Collection`] and [`State`] traits,
//! providing unified storage for both event collections and actor state snapshots.
//! Each collection maps to a SQLite table with optimized schema design:
//!
//! ```sql
//! -- Event collection schema
//! CREATE TABLE events (
//!     prefix TEXT NOT NULL,    -- Actor identifier/namespace
//!     sn TEXT NOT NULL,        -- Sequence number or key
//!     value BLOB NOT NULL,     -- Serialized event data
//!     PRIMARY KEY (prefix, sn)
//! );
//!
//! -- State snapshot schema
//! CREATE TABLE snapshots (
//!     prefix TEXT NOT NULL,    -- Actor identifier
//!     value BLOB NOT NULL,     -- Serialized state data
//!     PRIMARY KEY (prefix)
//! );
//! ```
//!
//! ### Performance Optimizations
//!
//! The implementation includes several performance optimizations:
//!
//! - **WAL Mode**: Enables concurrent reads during writes
//! - **Large Page Cache**: 40MB default cache for frequently accessed pages
//! - **Memory-Mapped I/O**: 256MB mmap size for efficient large file access
//! - **Temporary Tables in Memory**: Reduces I/O for intermediate results
//! - **Automatic Optimization**: Periodic PRAGMA optimize and ANALYZE
//!
//! ## Getting Started
//!
//! ### Basic Usage with Actor System
//!
//! ```ignore
//! use sqlite_db::{SqliteManager, SqliteCollection};
//! use store::PersistentActor;
//! use actor::{Actor, ActorContext};
//!
//! // Create SQLite manager
//! let sqlite_manager = SqliteManager::new("/path/to/database/dir")?;
//!
//! // Use with persistent actor
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct MyActor {
//!     id: String,
//!     counter: u64,
//! }
//!
//! #[async_trait]
//! impl Actor for MyActor {
//!     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         // Initialize SQLite storage
//!         self.start_store(
//!             "my_actors",
//!             Some(&self.id), // Use actor ID as prefix
//!             ctx,
//!             sqlite_manager,
//!             None, // No encryption in this example
//!         ).await?;
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ### Direct Database Operations
//!
//! ```ignore
//! use sqlite_db::SqliteManager;
//! use store::database::{DbManager, Collection, State};
//!
//! // Create manager and collections
//! let manager = SqliteManager::new("/data/sqlite")?;
//! let mut events = manager.create_collection("events", "actor_123")?;
//! let mut state = manager.create_state("snapshots", "actor_123")?;
//!
//! // Store events
//! events.put("001", b"event data")?;
//! events.put("002", b"more event data")?;
//!
//! // Store state snapshot
//! state.put(b"current state snapshot")?;
//!
//! // Retrieve data
//! let event_data = events.get("001")?;
//! let current_state = state.get()?;
//!
//! // Iterate events
//! for (key, data) in events.iter(false) { // false = ascending order
//!     println!("Event {}: {} bytes", key, data.len());
//! }
//! ```
//!
//! ## SQLite-Specific Features
//!
//! ### Database Configuration
//!
//! The SQLite backend automatically configures optimal settings:
//!
//! ```sql
//! -- Concurrency and reliability
//! PRAGMA journal_mode=WAL;         -- Write-Ahead Logging
//! PRAGMA synchronous=NORMAL;       -- Balance safety/performance
//! PRAGMA foreign_keys=ON;          -- Referential integrity
//!
//! -- Performance optimizations
//! PRAGMA cache_size=10000;         -- 40MB page cache
//! PRAGMA temp_store=memory;        -- Temp tables in RAM
//! PRAGMA mmap_size=268435456;      -- 256MB memory mapping
//! ```
//!
//! ### Security Measures
//!
//! The implementation includes comprehensive security hardening:
//!
//! - **SQL Injection Prevention**: All identifiers are validated and quoted
//! - **Identifier Validation**: Strict rules for table and column names
//! - **Parameter Binding**: All user data uses parameterized queries
//! - **Length Limits**: Maximum 64 characters for identifiers
//! - **Character Restrictions**: Alphanumeric and underscore only
//!
//! ### Performance Monitoring
//!
//! Built-in performance tracking provides operational insights:
//!
//! ```ignore
//! let stats = manager.performance_stats();
//! println!("Total queries: {}", stats.query_count);
//! println!("Last optimization: {}", stats.last_optimize_time);
//!
//! // Manual optimization (normally automatic)
//! manager.optimize()?;
//! ```
//!
//! ## Transaction and Concurrency Model
//!
//! ### ACID Properties
//!
//! SQLite provides full ACID compliance:
//!
//! - **Atomicity**: Each operation is atomic at the SQLite level
//! - **Consistency**: Foreign keys and constraints are enforced
//! - **Isolation**: WAL mode provides snapshot isolation for readers
//! - **Durability**: Commits are durable once WAL checkpoint completes
//!
//! ### Concurrency Characteristics
//!
//! - **Reader Concurrency**: Multiple concurrent readers in WAL mode
//! - **Writer Serialization**: Single writer with immediate reader consistency
//! - **Lock Granularity**: Database-level locking with minimal contention
//! - **Thread Safety**: Connections protected by Rust mutexes
//!
//! ### Connection Management
//!
//! Connections are managed through Arc<Mutex<Connection>> providing:
//!
//! - **Thread Safety**: Multiple actors can safely share the connection
//! - **Connection Reuse**: Single connection per database file
//! - **Error Isolation**: Connection errors don't affect other operations
//! - **Resource Management**: Automatic cleanup on manager drop
//!
//! ## File System Considerations
//!
//! ### Database Files
//!
//! SQLite creates several files in the specified directory:
//!
//! - `database.db` - Main database file
//! - `database.db-wal` - Write-ahead log file
//! - `database.db-shm` - Shared memory index
//!
//! ### Durability Guarantees
//!
//! - **Crash Recovery**: WAL mode enables automatic crash recovery
//! - **Checkpoint Durability**: Data is durable after WAL checkpoint
//! - **File System Sync**: NORMAL synchronous mode balances safety/performance
//! - **Atomic Commits**: Transaction boundaries ensure consistency
//!
//! ### Storage Efficiency
//!
//! - **Page-Based Storage**: 4KB pages with efficient space utilization
//! - **Compression Compatibility**: Works with store system's compression
//! - **Incremental Vacuum**: Automatic space reclamation
//! - **Index Optimization**: Query planner optimizes access patterns
//!
//! ## Integration with Store System
//!
//! The SQLite backend integrates seamlessly with the store system:
//!
//! ```ignore
//! use sqlite_db::SqliteManager;
//! use store::{Store, PersistentActor, FullPersistence};
//!
//! // Use with full event sourcing
//! impl PersistentActor for MyActor {
//!     type Persistence = FullPersistence;
//!
//!     fn apply(&mut self, event: &Self::Event) -> Result<(), actor::Error> {
//!         // Event application logic
//!         match event {
//!             MyEvent::Increment => { self.counter += 1; Ok(()) }
//!             // ... other events
//!         }
//!     }
//! }
//!
//! // Store will automatically use SQLite for persistence
//! let store = Store::new(
//!     "my_store",
//!     "actor_id",
//!     sqlite_manager,
//!     encryption_key,
//! )?;
//! ```
//!
//! ## Error Handling
//!
//! The SQLite backend provides comprehensive error handling:
//!
//! - **Connection Errors**: Database file access and locking issues
//! - **SQL Errors**: Malformed queries or constraint violations
//! - **Serialization Errors**: Data format and encoding problems
//! - **File System Errors**: Directory creation and permission issues
//!
//! All errors are wrapped in the store system's [`Error`] type with detailed
//! context information for debugging and monitoring.
//!
//! ## Best Practices
//!
//! ### Performance Optimization
//!
//! 1. **Use Appropriate Indexes**: The default schema includes optimized indexes
//! 2. **Monitor Query Patterns**: Use performance stats to identify bottlenecks
//! 3. **Batch Operations**: Group multiple operations when possible
//! 4. **Regular Optimization**: Automatic optimization runs every 10,000 queries
//!
//! ### Operational Considerations
//!
//! 1. **Backup Strategy**: Include WAL file in backups for consistency
//! 2. **Monitoring**: Track query counts and optimization frequency
//! 3. **Disk Space**: Monitor database growth and vacuum when needed
//! 4. **File Permissions**: Ensure write access to database directory
//!
//! ### Security Guidelines
//!
//! 1. **Identifier Validation**: All table names are validated and sanitized
//! 2. **Parameter Binding**: User data never interpolated into SQL
//! 3. **Access Control**: Implement file-level permissions as needed
//! 4. **Encryption**: Use store-level encryption for sensitive data
//!
//! The SQLite backend provides a robust, secure, and high-performance foundation
//! for persistent storage in the Rush actor system, combining SQLite's proven
//! reliability with optimizations specifically designed for actor workloads.

mod sqlite;

// Re-export public API for convenient access

/// SQLite-based storage collection implementing both Collection and State traits.
///
/// This is the primary storage implementation that provides both event collection
/// and state snapshot functionality. See the [`sqlite`] module for detailed
/// documentation and usage examples.
///
/// Key features:
/// - Unified interface for events and state storage
/// - Thread-safe concurrent access
/// - Optimized SQLite schema with performance tuning
/// - Comprehensive error handling and validation
///
/// Used through the store system's [`DbManager`] interface for creating
/// storage collections with appropriate isolation and security measures.
pub use sqlite::SqliteCollection;

/// SQLite database manager providing factory methods for storage creation.
///
/// The manager handles database connection lifecycle, performance monitoring,
/// and automatic optimization. It implements the store system's [`DbManager`]
/// trait to provide a standardized interface for creating storage collections.
///
/// Key responsibilities:
/// - Database connection management with thread-safe access
/// - Performance monitoring and automatic optimization
/// - Security validation for SQL identifiers
/// - Table creation and schema management
///
/// See the [`sqlite`] module for comprehensive usage examples and
/// configuration options.
pub use sqlite::SqliteManager;

/// Performance statistics for monitoring SQLite database operations.
///
/// Provides operational metrics including query counts and optimization
/// timing for database performance analysis and capacity planning.
///
/// Access through [`SqliteManager::performance_stats()`] for real-time
/// monitoring of database usage patterns.
pub use sqlite::PerformanceStats;
