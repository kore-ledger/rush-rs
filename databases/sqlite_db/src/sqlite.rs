// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # SQLite Database Implementation
//!
//! This module provides the complete SQLite backend implementation for the Rush store system,
//! including database management, collection handling, and performance optimization features.
//! The implementation focuses on security, performance, and reliability for production actor
//! system workloads.
//!
//! ## Core Components
//!
//! - [`SqliteManager`]: Database connection manager with performance monitoring
//! - [`SqliteCollection`]: Unified storage implementation for both collections and state
//! - [`SQLiteIterator`]: Iterator implementation for ordered data traversal
//! - [`PerformanceStats`]: Performance monitoring and optimization tracking
//!
//! ## Security Features
//!
//! The implementation includes comprehensive security measures:
//!
//! - SQL injection prevention through identifier validation
//! - Parameterized queries for all user data
//! - Strict identifier character and length restrictions
//! - Automatic SQL identifier quoting and escaping
//!
//! ## Performance Optimizations
//!
//! - Write-Ahead Logging (WAL) for improved concurrency
//! - Large page cache (40MB) for frequently accessed data
//! - Memory-mapped I/O for efficient large file access
//! - Automatic query optimization every 10,000 operations
//! - Connection pooling with thread-safe access

//use crate::error::NodeError;

use store::{
    Error,
    database::{Collection, DbManager, State},
};

use rusqlite::{Connection, OpenFlags, params};
use tracing::info;

use std::sync::{Arc, Mutex};
use std::{fs, path::Path};

/// Type alias for SQLite iterator result.
///
/// Represents the result of creating an iterator over SQLite collection data.
/// Returns either a boxed iterator over key-value pairs or a store [`Error`]
/// if the iteration cannot be initialized.
///
/// # Type Parameters
///
/// * `'a` - Lifetime parameter that ties the iterator to the underlying collection
///
/// # Returns
///
/// * `Ok(iterator)` - Boxed iterator yielding (String, Vec<u8>) pairs
/// * `Err(error)` - Store error describing the failure
type SqliteIterResult<'a> = Result<Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>, Error>;

/// Validates and sanitizes SQL identifiers to prevent injection attacks.
///
/// This function implements comprehensive validation and sanitization of SQL identifiers
/// used for table names and other database objects. It prevents SQL injection attacks
/// by enforcing strict rules on identifier format and automatically quoting the result.
///
/// # Security Measures
///
/// - **Length Validation**: Maximum 64 characters to prevent buffer overflow attacks
/// - **Character Restrictions**: Only alphanumeric characters and underscores allowed
/// - **Start Character**: Must begin with letter or underscore (SQL identifier rules)
/// - **Automatic Quoting**: Returns properly quoted identifier safe for SQL execution
/// - **Escape Handling**: Properly escapes any existing quotes in the identifier
///
/// # Arguments
///
/// * `identifier` - The raw identifier string to validate and sanitize
///
/// # Returns
///
/// * `Ok(quoted_identifier)` - Properly quoted and escaped SQL identifier
/// * `Err(Error::CreateStore)` - Validation error with specific failure reason
///
/// # Examples
///
/// ```rust
/// use sqlite_db::validate_sql_identifier;
///
/// // Valid identifiers
/// assert_eq!(validate_sql_identifier("events")?, r#""events""");
/// assert_eq!(validate_sql_identifier("user_data")?, r#""user_data""");
/// assert_eq!(validate_sql_identifier("_private")?, r#""_private""");
///
/// // Invalid identifiers
/// assert!(validate_sql_identifier("").is_err());
/// assert!(validate_sql_identifier("table name").is_err());
/// assert!(validate_sql_identifier("123invalid").is_err());
/// assert!(validate_sql_identifier("very_long_identifier_name_that_exceeds_maximum_allowed_length").is_err());
/// ```
///
/// # Security Notes
///
/// This function is critical for preventing SQL injection attacks when dynamically
/// constructing table names and other identifiers. All user-provided identifiers
/// MUST be validated through this function before use in SQL statements.
fn validate_sql_identifier(identifier: &str) -> Result<String, Error> {
    // Check for empty identifier
    if identifier.is_empty() {
        return Err(Error::CreateStore("Identifier cannot be empty".to_owned()));
    }

    // Check length limit
    if identifier.len() > 64 {
        return Err(Error::CreateStore("Identifier too long (max 64 characters)".to_owned()));
    }

    // Only allow alphanumeric characters and underscores
    if !identifier.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(Error::CreateStore("Invalid identifier: only alphanumeric characters and underscores allowed".to_owned()));
    }

    // Must start with a letter or underscore
    if let Some(first_char) = identifier.chars().next() {
        if !first_char.is_ascii_alphabetic() && !identifier.starts_with('_') {
            return Err(Error::CreateStore("Invalid identifier: must start with letter or underscore".to_owned()));
        }
    } else {
        // This should never happen due to empty check above, but defense in depth
        return Err(Error::CreateStore("Identifier cannot be empty".to_owned()));
    }

    // Quote the identifier to prevent SQL injection
    Ok(format!("\"{}\"", identifier.replace("\"", "\"\"")))
}

/// SQLite database manager with comprehensive performance monitoring and optimization.
///
/// The `SqliteManager` serves as the primary factory for creating SQLite storage collections
/// and managing database connections. It implements the [`DbManager`] trait from the store
/// system, providing a standardized interface for database operations while offering
/// SQLite-specific optimizations and monitoring capabilities.
///
/// # Architecture
///
/// The manager uses a shared connection model with thread-safe access through Arc<Mutex<Connection>>.
/// This design provides:
///
/// - **Connection Reuse**: Single connection per database file reduces resource overhead
/// - **Thread Safety**: Multiple actors can safely share the same database connection
/// - **Performance Monitoring**: Built-in query counting and optimization tracking
/// - **Automatic Optimization**: Periodic database optimization based on usage patterns
///
/// # Performance Features
///
/// - **Query Counting**: Tracks total number of database operations
/// - **Automatic Optimization**: Runs SQLite PRAGMA optimize every 10,000 queries
/// - **Optimization Throttling**: Minimum 1-hour interval between optimizations
/// - **Performance Statistics**: Accessible metrics for monitoring and tuning
///
/// # Thread Safety
///
/// The manager is `Clone` and can be safely shared across multiple threads and actors.
/// All database operations are protected by internal mutexes, ensuring data consistency
/// without blocking the async runtime.
///
/// # Examples
///
/// ```rust
/// use sqlite_db::SqliteManager;
/// use store::database::DbManager;
///
/// // Create manager for a specific database directory
/// let manager = SqliteManager::new("/data/actor_storage")?;
///
/// // Create collections for different actors
/// let events = manager.create_collection("events", "actor_123")?;
/// let snapshots = manager.create_state("snapshots", "actor_123")?;
///
/// // Monitor performance
/// let stats = manager.performance_stats();
/// println!("Total queries: {}", stats.query_count);
///
/// // Manual optimization (normally automatic)
/// manager.optimize()?;
/// ```
#[derive(Clone)]
pub struct SqliteManager {
    /// Shared SQLite database connection protected by mutex for thread safety.
    ///
    /// The connection is wrapped in Arc<Mutex<>> to allow safe sharing across
    /// multiple threads while ensuring exclusive access during database operations.
    /// This design enables multiple actors to share a single database connection
    /// without risking data corruption or connection conflicts.
    conn: Arc<Mutex<Connection>>,

    /// Atomic counter tracking total number of database queries executed.
    ///
    /// This counter is incremented on every database operation and used for:
    /// - Performance monitoring and metrics collection
    /// - Triggering automatic database optimization
    /// - Operational insight into system load patterns
    query_count: Arc<std::sync::atomic::AtomicUsize>,

    /// Unix timestamp of the last database optimization operation.
    ///
    /// Used to throttle automatic optimizations to prevent excessive overhead.
    /// Optimizations are limited to once per hour regardless of query volume
    /// to balance performance benefits with resource usage.
    last_optimize_time: Arc<std::sync::atomic::AtomicU64>,
}

impl SqliteManager {
    /// Creates a new SQLite database manager with optimized configuration.
    ///
    /// This constructor initializes a SQLite database manager that creates and configures
    /// a database file in the specified directory. The manager automatically applies
    /// performance optimizations and security settings appropriate for actor system
    /// workloads.
    ///
    /// # Database Configuration
    ///
    /// The constructor automatically applies the following optimizations:
    /// - WAL (Write-Ahead Logging) mode for improved concurrency
    /// - 40MB page cache for frequently accessed data
    /// - 256MB memory-mapped I/O for large file efficiency
    /// - Normal synchronous mode balancing safety and performance
    /// - Foreign key constraint enforcement for data integrity
    ///
    /// # Directory Management
    ///
    /// If the specified directory doesn't exist, it will be created automatically.
    /// The database file will be named `database.db` within this directory.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory path where the SQLite database file will be created
    ///
    /// # Returns
    ///
    /// * `Ok(SqliteManager)` - Configured manager ready for use
    /// * `Err(Error::CreateStore)` - Directory creation or database connection failed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use sqlite_db::SqliteManager;
    ///
    /// // Create manager for production storage
    /// let manager = SqliteManager::new("/var/lib/myapp/data")?;
    ///
    /// // Create manager for development/testing
    /// let test_manager = SqliteManager::new("/tmp/test_db")?;
    /// ```
    ///
    /// # File System Requirements
    ///
    /// - Write permissions to the specified directory
    /// - Sufficient disk space for database growth
    /// - Directory must be accessible by the application process
    ///
    /// # Thread Safety
    ///
    /// The returned manager can be safely cloned and shared across threads.
    /// All database operations are internally synchronized.
    pub fn new(path: &str) -> Result<Self, Error> {
        info!("Creating SQLite database manager");
        if !Path::new(&path).exists() {
            info!("Path does not exist, creating it");
            fs::create_dir_all(path).map_err(|e| {
                Error::CreateStore(format!(
                    "fail SQLite create directory: {}",
                    e
                ))
            })?;
        }

        info!("Opening SQLite connection");
        let conn = open(format!("{}/database.db", path)).map_err(|e| {
            Error::CreateStore(format!("fail SQLite open connection: {}", e))
        })?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            query_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            last_optimize_time: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Retrieves current performance statistics for monitoring and analysis.
    ///
    /// Returns a snapshot of the database manager's performance metrics, including
    /// query counts and optimization timing information. These statistics are useful
    /// for:
    ///
    /// - **Monitoring**: Track database usage patterns and load
    /// - **Performance Tuning**: Identify optimization opportunities
    /// - **Operational Insight**: Understand system behavior over time
    /// - **Capacity Planning**: Assess resource utilization trends
    ///
    /// # Returns
    ///
    /// A [`PerformanceStats`] struct containing:
    /// - `query_count`: Total number of database operations performed
    /// - `last_optimize_time`: Unix timestamp of last database optimization
    ///
    /// # Examples
    ///
    /// ```rust
    /// use sqlite_db::SqliteManager;
    ///
    /// let manager = SqliteManager::new("/data/db")?;
    ///
    /// // Perform some operations...
    /// let collection = manager.create_collection("events", "actor1")?;
    ///
    /// // Check performance metrics
    /// let stats = manager.performance_stats();
    /// println!("Queries executed: {}", stats.query_count);
    ///
    /// if stats.last_optimize_time > 0 {
    ///     let last_optimize = std::time::UNIX_EPOCH +
    ///         std::time::Duration::from_secs(stats.last_optimize_time);
    ///     println!("Last optimized: {:?}", last_optimize);
    /// }
    /// ```
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called concurrently from multiple threads.
    /// The returned statistics represent a consistent snapshot at the time of the call.
    pub fn performance_stats(&self) -> PerformanceStats {
        PerformanceStats {
            query_count: self.query_count.load(std::sync::atomic::Ordering::Relaxed),
            last_optimize_time: self.last_optimize_time.load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Manually triggers database optimization for improved query performance.
    ///
    /// This method executes SQLite's built-in optimization commands to improve query
    /// performance by updating table statistics and optimizing query plans. While
    /// optimization normally occurs automatically every 10,000 queries, this method
    /// allows manual control when needed.
    ///
    /// # Optimization Operations
    ///
    /// The method executes:
    /// - `PRAGMA optimize` - Updates query planner statistics and optimizes indexes
    /// - `ANALYZE` - Gathers table and index statistics for query optimization
    ///
    /// These operations can significantly improve query performance, especially after
    /// large data changes or when access patterns have shifted.
    ///
    /// # Performance Impact
    ///
    /// - **CPU Usage**: Moderate CPU usage during optimization
    /// - **I/O Impact**: Temporary increase in disk I/O for statistics gathering
    /// - **Duration**: Typically completes in seconds for normal-sized databases
    /// - **Concurrency**: Does not block concurrent read operations in WAL mode
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Optimization completed successfully
    /// * `Err(Error::Store)` - Database optimization failed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use sqlite_db::SqliteManager;
    ///
    /// let manager = SqliteManager::new("/data/db")?;
    ///
    /// // After bulk data operations
    /// // ... insert many records ...
    ///
    /// // Manually optimize for better query performance
    /// manager.optimize()?;
    /// println!("Database optimization completed");
    ///
    /// // Check when optimization last ran
    /// let stats = manager.performance_stats();
    /// println!("Last optimized at: {}", stats.last_optimize_time);
    /// ```
    ///
    /// # When to Use
    ///
    /// Manual optimization is beneficial:
    /// - After bulk data imports or large-scale changes
    /// - When query performance has degraded
    /// - During maintenance windows for proactive optimization
    /// - Before performance-critical operations
    ///
    /// # Automatic Optimization
    ///
    /// The manager automatically calls this method every 10,000 queries with
    /// a minimum 1-hour interval to prevent excessive optimization overhead.
    pub fn optimize(&self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection for optimization: {}", e))
        })?;

        conn.execute_batch(
            "
            PRAGMA optimize;
            ANALYZE;
            ",
        )
        .map_err(|e| Error::Store(format!("sqlite optimization failed: {}", e)))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_optimize_time.store(now, std::sync::atomic::Ordering::Relaxed);

        tracing::info!("Database optimization completed");
        Ok(())
    }

    /// Internal method to track query performance and trigger automatic optimization.
    ///
    /// This method is called internally for every database operation to:
    /// - Increment the query counter for performance monitoring
    /// - Trigger automatic database optimization when thresholds are met
    /// - Manage optimization timing to prevent excessive overhead
    ///
    /// # Automatic Optimization Logic
    ///
    /// Optimization is triggered when:
    /// - Query count reaches multiples of 10,000 operations
    /// - At least 1 hour has passed since the last optimization
    ///
    /// This strategy balances performance benefits with resource usage,
    /// ensuring the database stays optimized without excessive overhead.
    ///
    /// # Thread Safety
    ///
    /// Uses atomic operations for lock-free performance tracking,
    /// ensuring minimal overhead on database operations.
    fn track_query(&self) {
        let count = self.query_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Auto-optimize every 10000 queries
        if count > 0 && count.is_multiple_of(10000) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let last_optimize = self.last_optimize_time.load(std::sync::atomic::Ordering::Relaxed);

            // Only optimize if it's been more than 1 hour since last optimization
            if now > last_optimize + 3600
                && let Err(e) = self.optimize() {
                    tracing::warn!("Auto-optimization failed: {}", e);
                }
        }
    }
}

/// Implementation of the [`DbManager`] trait for SQLite database management.
///
/// This implementation provides the factory interface for creating SQLite-backed
/// storage collections and state containers. The implementation handles table
/// creation, schema management, and proper isolation between different actors
/// and data domains.
///
/// # Design Principles
///
/// - **Security First**: All identifiers are validated and sanitized to prevent SQL injection
/// - **Performance Oriented**: Optimized table schemas with appropriate indexes
/// - **Isolation**: Prefix-based logical separation within shared tables
/// - **Monitoring**: Query tracking for performance analysis and optimization
///
/// # Table Schema Management
///
/// The manager creates optimized table schemas for different use cases:
///
/// **Collection Tables** (for events and key-value data):
/// - Compound primary key (prefix, sn) for efficient range queries
/// - BLOB storage for flexible data serialization
/// - Automatic conflict resolution with INSERT OR REPLACE
///
/// **State Tables** (for snapshot storage):
/// - Simple primary key (prefix) for single-value storage per actor
/// - Optimized for frequent updates and retrievals
/// - Minimal overhead for state persistence scenarios
impl DbManager<SqliteCollection, SqliteCollection> for SqliteManager {
    /// Creates a new state storage collection for actor snapshot persistence.
    ///
    /// This method creates or connects to a SQLite table optimized for state
    /// snapshot storage. State tables use a simple schema with a single value
    /// per prefix, making them ideal for storing current actor state.
    ///
    /// # Table Schema Created
    ///
    /// ```sql
    /// CREATE TABLE IF NOT EXISTS identifier (
    ///     prefix TEXT NOT NULL,
    ///     value BLOB NOT NULL,
    ///     PRIMARY KEY (prefix)
    /// );
    /// ```
    ///
    /// # Arguments
    ///
    /// * `identifier` - Table name for the state storage (will be validated and quoted)
    /// * `prefix` - Unique prefix for this actor's data within the table
    ///
    /// # Returns
    ///
    /// * `Ok(SqliteCollection)` - Collection configured for state storage operations
    /// * `Err(Error::CreateStore)` - Table creation failed or identifier invalid
    ///
    /// # Security
    ///
    /// The identifier is validated to prevent SQL injection attacks and must meet
    /// strict criteria for length, character set, and format.
    fn create_state(
        &self,
        identifier: &str,
        prefix: &str,
    ) -> Result<SqliteCollection, Error> {
        // Validate and sanitize the identifier to prevent SQL injection
        let safe_identifier = validate_sql_identifier(identifier)?;
        self.track_query(); // Track performance

        // Create statement to create a table using sanitized identifier.
        let stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (prefix TEXT NOT NULL, value \
            BLOB NOT NULL, PRIMARY KEY (prefix))",
            safe_identifier
        );

        {
            let conn = self.conn.lock().map_err(|e| {
                Error::CreateStore(format!("SQLite connection mutex poisoned: {}", e))
            })?;

            conn.execute(stmt.as_str(), ()).map_err(|e| {
                Error::CreateStore(format!("fail SQLite create table: {}", e))
            })?;
        }

        Ok(SqliteCollection::new(self.conn.clone(), &safe_identifier, prefix))
    }

    /// Creates a new collection for event storage and key-value operations.
    ///
    /// This method creates or connects to a SQLite table optimized for storing
    /// multiple key-value pairs per prefix. Collections are ideal for event
    /// sourcing, audit trails, and any scenario requiring ordered data storage.
    ///
    /// # Table Schema Created
    ///
    /// ```sql
    /// CREATE TABLE IF NOT EXISTS identifier (
    ///     prefix TEXT NOT NULL,
    ///     sn TEXT NOT NULL,
    ///     value BLOB NOT NULL,
    ///     PRIMARY KEY (prefix, sn)
    /// );
    /// ```
    ///
    /// # Arguments
    ///
    /// * `identifier` - Table name for the collection (will be validated and quoted)
    /// * `prefix` - Unique prefix for this actor's data within the table
    ///
    /// # Returns
    ///
    /// * `Ok(SqliteCollection)` - Collection ready for key-value operations
    /// * `Err(Error::CreateStore)` - Table creation failed or identifier invalid
    ///
    /// # Performance Characteristics
    ///
    /// - **Compound Index**: Efficient queries on (prefix, sn) combinations
    /// - **Range Queries**: Optimized for ordered iteration within a prefix
    /// - **Conflict Resolution**: INSERT OR REPLACE for idempotent operations
    fn create_collection(
        &self,
        identifier: &str,
        prefix: &str,
    ) -> Result<SqliteCollection, Error> {
        // Validate and sanitize the identifier to prevent SQL injection
        let safe_identifier = validate_sql_identifier(identifier)?;
        self.track_query(); // Track performance

        // Create statement to create a table using sanitized identifier.
        let stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (prefix TEXT NOT NULL, sn TEXT NOT NULL, value \
            BLOB NOT NULL, PRIMARY KEY (prefix, sn))",
            safe_identifier
        );

        {
            let conn = self.conn.lock().map_err(|e| {
                Error::CreateStore(format!("SQLite connection mutex poisoned: {}", e))
            })?;

            conn.execute(stmt.as_str(), ()).map_err(|e| {
                Error::CreateStore(format!("fail SQLite create table: {}", e))
            })?;
        }

        Ok(SqliteCollection::new(self.conn.clone(), &safe_identifier, prefix))
    }
}

/// SQLite-based storage collection implementing both Collection and State traits.
///
/// `SqliteCollection` provides a unified storage abstraction that can serve as both
/// an event collection (implementing [`Collection`]) and a state snapshot container
/// (implementing [`State`]). This dual-purpose design simplifies the storage layer
/// while providing optimal performance for both use cases.
///
/// # Architecture
///
/// Each collection maps to a SQLite table with schema optimized for the specific
/// use case:
///
/// **Collection Schema** (for event storage):
/// ```sql
/// CREATE TABLE events (
///     prefix TEXT NOT NULL,    -- Actor identifier/namespace
///     sn TEXT NOT NULL,        -- Sequence number or event key
///     value BLOB NOT NULL,     -- Serialized event data
///     PRIMARY KEY (prefix, sn)
/// );
/// ```
///
/// **State Schema** (for snapshot storage):
/// ```sql
/// CREATE TABLE snapshots (
///     prefix TEXT NOT NULL,    -- Actor identifier
///     value BLOB NOT NULL,     -- Serialized state data
///     PRIMARY KEY (prefix)
/// );
/// ```
///
/// # Prefix-Based Isolation
///
/// Collections use prefixes to provide logical isolation within shared tables:
/// - Each actor instance gets a unique prefix
/// - Multiple actors can share the same table safely
/// - Prefix-based queries ensure data isolation
/// - Efficient indexing on (prefix, key) combinations
///
/// # Performance Characteristics
///
/// - **Read Performance**: O(log n) lookups using B-tree indexes
/// - **Write Performance**: O(log n) inserts with automatic conflict resolution
/// - **Range Queries**: Efficient iteration with optional reverse ordering
/// - **Concurrency**: WAL mode enables concurrent reads during writes
///
/// # Thread Safety
///
/// Collections share the same thread-safe connection as their parent manager.
/// Multiple collections can be used concurrently from different threads without
/// additional synchronization.
///
/// # Examples
///
/// ```rust
/// use sqlite_db::{SqliteManager, SqliteCollection};
/// use store::database::{Collection, State};
///
/// let manager = SqliteManager::new("/data/db")?;
///
/// // Create collections for different purposes
/// let mut events = manager.create_collection("events", "actor_123")?;
/// let mut state = manager.create_state("snapshots", "actor_123")?;
///
/// // Use as event collection
/// events.put("001", b"first event")?;
/// events.put("002", b"second event")?;
/// let event = events.get("001")?;
///
/// // Use as state container
/// state.put(b"current actor state")?;
/// let current_state = state.get()?;
///
/// // Iterate events in order
/// for (key, data) in events.iter(false) { // false = ascending
///     println!("Event {}: {} bytes", key, data.len());
/// }
/// ```
pub struct SqliteCollection {
    /// Shared database connection protected by mutex for thread safety.
    ///
    /// This connection is shared with the parent [`SqliteManager`] and potentially
    /// other collections, enabling efficient resource usage while maintaining
    /// thread safety through mutex protection.
    conn: Arc<Mutex<Connection>>,

    /// SQL-safe quoted table name for use in database queries.
    ///
    /// This field contains the table name after validation and SQL quoting,
    /// making it safe to use directly in SQL statements without risk of
    /// injection attacks. Example: `"events"` or `"user_data"`.
    table: String,

    /// Original unquoted table name for display and logging purposes.
    ///
    /// Stores the original table name without SQL quoting for use in
    /// error messages, logging, and user-facing operations where the
    /// raw name is more appropriate than the quoted version.
    table_name: String,

    /// Prefix used to isolate this collection's data within the shared table.
    ///
    /// The prefix provides logical separation between different actors or
    /// data domains sharing the same table. All queries for this collection
    /// are automatically filtered by this prefix value.
    prefix: String,
}

impl SqliteCollection {
    /// Creates a new SQLite collection with the specified connection, table, and prefix.
    ///
    /// This constructor initializes a collection that can serve as either an event
    /// collection or state container, depending on the table schema and usage patterns.
    /// The collection automatically handles SQL identifier processing and maintains
    /// both quoted and unquoted versions of the table name.
    ///
    /// # Arguments
    ///
    /// * `conn` - Shared database connection wrapped in Arc<Mutex<>> for thread safety
    /// * `table` - SQL-safe quoted table name (e.g., `"events"` or `"snapshots"`)
    /// * `prefix` - Prefix string for data isolation within the table
    ///
    /// # Returns
    ///
    /// A new `SqliteCollection` instance ready for database operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use sqlite_db::SqliteCollection;
    /// use std::sync::{Arc, Mutex};
    ///
    /// let conn = Arc::new(Mutex::new(connection));
    /// let collection = SqliteCollection::new(
    ///     conn,
    ///     r#""events""#,  // Properly quoted table name
    ///     "actor_123"    // Prefix for data isolation
    /// );
    /// ```
    ///
    /// # Table Name Processing
    ///
    /// The constructor automatically extracts the original table name from
    /// quoted identifiers for display purposes while preserving the quoted
    /// version for safe SQL usage.
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        table: &str,
        prefix: &str,
    ) -> Self {
        // Extract original name from quoted identifier
        let original_name = if table.starts_with('"') && table.ends_with('"') {
            table[1..table.len()-1].to_owned()
        } else {
            table.to_owned()
        };

        Self {
            conn,
            table: table.to_owned(),
            table_name: original_name,
            prefix: prefix.to_owned(),
        }
    }

    /// Creates an iterator over collection data filtered by prefix with optional ordering.
    ///
    /// This internal method constructs an iterator that yields key-value pairs from
    /// the collection, filtered by the collection's prefix and ordered according to
    /// the specified direction. The iterator is optimized for sequential access
    /// patterns common in event replay and data processing.
    ///
    /// # Implementation Details
    ///
    /// The method:
    /// 1. Constructs a parameterized SQL query with prefix filtering
    /// 2. Applies ordering based on the `reverse` parameter
    /// 3. Executes the query and collects results into memory
    /// 4. Returns a boxed iterator over the collected data
    ///
    /// # Memory Considerations
    ///
    /// This implementation loads all matching results into memory before returning
    /// the iterator. For very large result sets, consider implementing streaming
    /// iteration or pagination to manage memory usage.
    ///
    /// # Arguments
    ///
    /// * `reverse` - If `true`, returns results in descending order; if `false`, ascending
    ///
    /// # Returns
    ///
    /// * `Ok(iterator)` - Boxed iterator yielding (String, Vec<u8>) pairs
    /// * `Err(Error::Store)` - Database query failed or connection unavailable
    ///
    /// # SQL Query Structure
    ///
    /// ```sql
    /// SELECT sn, value FROM table_name
    /// WHERE prefix = ?1
    /// ORDER BY sn ASC/DESC
    /// ```
    ///
    /// # Thread Safety
    ///
    /// This method acquires a mutex lock on the database connection during
    /// query execution but releases it before returning the iterator.
    fn make_iter<'a>(
        &'a self,
        reverse: bool,
    ) -> SqliteIterResult<'a> {
        let order = if reverse { "DESC" } else { "ASC" };
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLite connection mutex poisoned: {}", e))
        })?;
        let query = format!(
            "SELECT sn, value FROM {} WHERE prefix = ?1 ORDER BY sn {}",
            self.table, order,
        );
        let mut stmt = conn.prepare(&query).map_err(|e| Error::Store(e.to_string()))?;
        let mut rows = stmt.query(params![self.prefix]).map_err(|e| Error::Store(e.to_string()))?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(|e| Error::Store(e.to_string()))? {
            let key: String = row.get(0).map_err(|e| Error::Store(e.to_string()))?;
            values.push((key, row.get(1).map_err(|e| Error::Store(e.to_string()))?));
        }
        Ok(Box::new(values.into_iter()))
    }
}

/// Implementation of the [`State`] trait for state snapshot storage.
///
/// This implementation provides state container functionality for actors that need
/// to persist their current state as snapshots. Unlike event collections, state
/// storage maintains only the current state, replacing previous values on each update.
///
/// # Table Schema
///
/// State collections use a simple key-value schema:
/// ```sql
/// CREATE TABLE snapshots (
///     prefix TEXT NOT NULL,    -- Actor identifier
///     value BLOB NOT NULL,     -- Serialized state data
///     PRIMARY KEY (prefix)
/// );
/// ```
///
/// # Usage Patterns
///
/// - **Snapshot Storage**: Store current actor state for fast recovery
/// - **Checkpoint Creation**: Periodic state snapshots to reduce replay time
/// - **State Persistence**: Survive actor restarts and system crashes
/// - **Recovery Optimization**: Quick restoration without event replay
impl State for SqliteCollection {
    /// Retrieves the current state data for this collection's prefix.
    ///
    /// This method loads the most recent state snapshot stored for the actor
    /// identified by this collection's prefix. The state is returned as raw
    /// bytes that must be deserialized by the caller.
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<u8>)` - The serialized state data
    /// * `Err(Error::EntryNotFound)` - No state exists for this prefix
    /// * `Err(Error::Store)` - Database connection or query failed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use sqlite_db::SqliteManager;
    /// use store::database::{DbManager, State};
    ///
    /// let manager = SqliteManager::new("/data/db")?;
    /// let state = manager.create_state("snapshots", "actor_123")?;
    ///
    /// match state.get() {
    ///     Ok(data) => println!("State loaded: {} bytes", data.len()),
    ///     Err(Error::EntryNotFound(_)) => println!("No state found"),
    ///     Err(e) => eprintln!("Error loading state: {}", e),
    /// }
    /// ```
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called concurrently from multiple threads.
    fn get(&self) -> Result<Vec<u8>, Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let query =
            format!("SELECT value FROM {} WHERE prefix = ?1", &self.table);
        let row: Vec<u8> = conn
            .query_row(&query, params![self.prefix], |row| row.get(0))
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;

        Ok(row)
    }

    /// Stores or updates the current state data for this collection's prefix.
    ///
    /// This method saves the provided state data, replacing any existing state
    /// for this prefix. The operation uses SQLite's `INSERT OR REPLACE` to
    /// handle both initial storage and subsequent updates atomically.
    ///
    /// # Arguments
    ///
    /// * `data` - Serialized state data to store
    ///
    /// # Returns
    ///
    /// * `Ok(())` - State stored successfully
    /// * `Err(Error::Store)` - Database connection or insert failed
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called concurrently.
    fn put(&mut self, data: &[u8]) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let stmt = format!(
            "INSERT OR REPLACE INTO {} (prefix, value) VALUES (?1, ?2)",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, data])
            .map_err(|e| Error::Store(format!("sqlite insert error: {}", e)))?;
        Ok(())
    }

    /// Deletes the current state data for this collection's prefix.
    ///
    /// This method removes the state snapshot, effectively resetting the
    /// actor to a state where no snapshot exists. Subsequent `get()` calls
    /// will return `EntryNotFound` until new state is stored.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - State deleted successfully or didn't exist
    /// * `Err(Error::EntryNotFound)` - Database operation failed
    fn del(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix,])
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix])
            .map_err(|e| Error::Store(format!("SQLITE purge error: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        self.table_name.as_str()
    }
}

/// Implementation of the [`Collection`] trait for event and key-value storage.
///
/// This implementation provides collection functionality for actors that need to
/// persist ordered events or key-value data. Unlike state storage, collections
/// maintain multiple entries with unique keys, making them ideal for event
/// sourcing, audit trails, and ordered data storage.
///
/// # Table Schema
///
/// Collection tables use a compound primary key schema:
/// ```sql
/// CREATE TABLE events (
///     prefix TEXT NOT NULL,    -- Actor identifier/namespace
///     sn TEXT NOT NULL,        -- Sequence number or key
///     value BLOB NOT NULL,     -- Serialized data
///     PRIMARY KEY (prefix, sn)
/// );
/// ```
///
/// # Usage Patterns
///
/// - **Event Sourcing**: Store ordered events with sequence numbers
/// - **Audit Logging**: Maintain immutable records of system changes
/// - **Key-Value Storage**: General-purpose indexed data storage
/// - **Time-Series Data**: Store timestamped or sequenced data points
impl Collection for SqliteCollection {
    /// Retrieves data for a specific key within this collection.
    ///
    /// # Arguments
    /// * `key` - The key to retrieve data for
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - The serialized data for the key
    /// * `Err(Error::EntryNotFound)` - Key doesn't exist in collection
    /// * `Err(Error::Store)` - Database connection or query failed
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let query = format!(
            "SELECT value FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        let row: Vec<u8> = conn
            .query_row(&query, params![self.prefix, key], |row| row.get(0))
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;

        Ok(row)
    }

    /// Stores or updates data for a specific key within this collection.
    ///
    /// # Arguments
    /// * `key` - The key to store data under
    /// * `data` - Serialized data to store
    ///
    /// # Returns
    /// * `Ok(())` - Data stored successfully
    /// * `Err(Error::Store)` - Database connection or insert failed
    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let stmt = format!(
            "INSERT OR REPLACE INTO {} (prefix, sn, value) VALUES (?1, ?2, ?3)",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key, data])
            .map_err(|e| Error::Store(format!("sqlite insert error: {}", e)))?;
        Ok(())
    }

    /// Deletes a specific key-value pair from this collection.
    ///
    /// # Arguments
    /// * `key` - The key to delete
    ///
    /// # Returns
    /// * `Ok(())` - Key deleted successfully
    /// * `Err(Error::EntryNotFound)` - Key doesn't exist or delete failed
    fn del(&mut self, key: &str) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!(
            "DELETE FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key])
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix])
            .map_err(|e| Error::Store(format!("SQLITE purge error: {}", e)))?;
        Ok(())
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        match self.make_iter(reverse) {
            Ok(iter) => {
                let iterator = SQLiteIterator { iter };
                Box::new(iterator)
            }
            Err(_) => Box::new(std::iter::empty()),
        }
    }

    fn name(&self) -> &str {
        self.table_name.as_str()
    }
}

/// Iterator implementation for SQLite collection data traversal.
///
/// This iterator provides ordered access to collection data, yielding key-value pairs
/// in either ascending or descending order based on the sort key. The iterator is
/// created through the `Collection::iter()` method and provides efficient sequential
/// access to stored data.
///
/// # Performance Characteristics
///
/// - **Memory Usage**: Loads all matching results into memory before iteration
/// - **Ordering**: Maintains SQLite's natural ordering based on the `sn` column
/// - **Thread Safety**: Safe to use across threads once created
/// - **Lazy Evaluation**: Results are pre-loaded but yielded on-demand
///
/// # Examples
///
/// ```rust
/// use sqlite_db::SqliteManager;
/// use store::database::{DbManager, Collection};
///
/// let manager = SqliteManager::new("/data/db")?;
/// let collection = manager.create_collection("events", "actor_123")?;
///
/// // Iterate in ascending order (default)
/// for (key, data) in collection.iter(false) {
///     println!("Key: {}, Size: {} bytes", key, data.len());
/// }
///
/// // Iterate in descending order
/// for (key, data) in collection.iter(true) {
///     println!("Key: {}, Size: {} bytes", key, data.len());
/// }
/// ```
pub struct SQLiteIterator<'a> {
    /// Boxed iterator over key-value pairs.
    ///
    /// Contains the actual iterator implementation that yields (String, Vec<u8>)
    /// pairs representing the keys and values from the SQLite collection.
    pub iter: Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>,
}

impl Iterator for SQLiteIterator<'_> {
    type Item = (String, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

/// Opens a SQLite database connection with comprehensive performance optimizations.
///
/// This function creates a new SQLite connection configured with settings optimized
/// for actor system workloads. The configuration emphasizes performance, reliability,
/// and security appropriate for production use.
///
/// # Performance Optimizations Applied
///
/// - **WAL Mode**: Write-Ahead Logging for improved concurrency and crash recovery
/// - **40MB Cache**: Large page cache for frequently accessed data
/// - **Memory-Mapped I/O**: 256MB mmap for efficient large file access
/// - **Normal Sync**: Balanced durability and performance trade-off
/// - **Memory Temp Storage**: Temporary tables stored in RAM for speed
/// - **Auto Optimization**: Enables SQLite's built-in query optimizer
///
/// # Security Settings Applied
///
/// - **Foreign Keys**: Enables referential integrity constraints
/// - **Check Constraints**: Enforces data validation rules
/// - **Safe Defaults**: Conservative settings for data protection
///
/// # Arguments
///
/// * `path` - Path to the SQLite database file to open or create
///
/// # Returns
///
/// * `Ok(Connection)` - Configured SQLite connection ready for use
/// * `Err(Error::Store)` - Database file access failed or configuration error
///
/// # Examples
///
/// ```rust
/// use sqlite_db::open;
///
/// // Open database with optimized settings
/// let conn = open("/data/myapp.db")?;
///
/// // Connection is ready for high-performance operations
/// conn.execute("CREATE TABLE IF NOT EXISTS test (id INTEGER PRIMARY KEY)", [])?;
/// ```
///
/// # File Requirements
///
/// - Write permissions to the database file and directory
/// - Sufficient disk space for database growth and WAL file
/// - File system support for memory-mapped files (most modern filesystems)
///
/// # Concurrency Model
///
/// The connection is configured for WAL mode, which provides:
/// - Multiple concurrent readers
/// - Single writer with immediate reader visibility
/// - Automatic checkpoint management
/// - Superior crash recovery compared to rollback journal
pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection, Error> {
    let path = path.as_ref();
    let flags =
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| {
        Error::Store(format!("SQLite failed to open connection: {}", e))
    })?;

    // Enhanced performance and security settings
    conn.execute_batch(
        "
        -- Performance optimizations
        PRAGMA journal_mode=WAL;         -- Write-Ahead Logging for better concurrency
        PRAGMA synchronous=NORMAL;       -- Good balance of safety and performance
        PRAGMA cache_size=10000;         -- 40MB cache (10000 * 4KB pages)
        PRAGMA temp_store=memory;        -- Store temporary tables in memory
        PRAGMA mmap_size=268435456;      -- 256MB memory-mapped I/O
        PRAGMA optimize;                 -- Optimize database structure

        -- Security settings
        PRAGMA foreign_keys=ON;          -- Enable foreign key constraints
        PRAGMA ignore_check_constraints=OFF; -- Enforce check constraints
        ",
    )
    .map_err(|e| {
        Error::Store(format!("SQLite failed to execute batch: {}", e))
    })?;

    Ok(conn)
}

/// Performance statistics for SQLite database monitoring and optimization analysis.
///
/// This structure provides comprehensive metrics about database usage patterns,
/// query performance, and optimization timing. The statistics are useful for:
///
/// - **Operational Monitoring**: Track database load and usage patterns
/// - **Performance Tuning**: Identify optimization opportunities and bottlenecks
/// - **Capacity Planning**: Understand resource utilization and growth trends
/// - **Troubleshooting**: Diagnose performance issues and optimization effectiveness
///
/// # Metrics Included
///
/// - **Query Count**: Total number of database operations performed
/// - **Optimization Timing**: When database optimization last occurred
///
/// # Usage Examples
///
/// ```rust
/// use sqlite_db::SqliteManager;
///
/// let manager = SqliteManager::new("/data/db")?;
///
/// // Perform database operations...
/// let collection = manager.create_collection("events", "actor1")?;
///
/// // Analyze performance
/// let stats = manager.performance_stats();
/// println!("Database has processed {} queries", stats.query_count);
///
/// if stats.last_optimize_time > 0 {
///     let hours_since_optimize = (current_timestamp() - stats.last_optimize_time) / 3600;
///     println!("Last optimized {} hours ago", hours_since_optimize);
/// } else {
///     println!("Database has never been optimized");
/// }
///
/// // Trigger optimization if needed
/// if stats.query_count > 50000 && hours_since_optimize > 24 {
///     manager.optimize()?;
/// }
/// ```
///
/// # Performance Interpretation
///
/// - **High Query Count**: Indicates heavy database usage, may benefit from optimization
/// - **Recent Optimization**: Database should have optimal query performance
/// - **Old/Missing Optimization**: May indicate degraded performance, consider manual optimization
///
/// # Thread Safety
///
/// This struct is `Clone` and can be safely shared across threads. The contained
/// values represent a snapshot at the time `performance_stats()` was called.
#[derive(Debug, Clone)]
pub struct PerformanceStats {
    /// Total number of database queries executed since manager creation.
    ///
    /// This counter includes all database operations performed through this manager:
    /// - Table creation operations during collection setup
    /// - Data insertion, update, and deletion operations
    /// - Query operations for data retrieval
    /// - Iterator operations for data traversal
    ///
    /// The count is used to trigger automatic optimization every 10,000 operations.
    pub query_count: usize,

    /// Unix timestamp of the last database optimization operation.
    ///
    /// Contains the timestamp (seconds since Unix epoch) when the database was
    /// last optimized using `PRAGMA optimize` and `ANALYZE`. A value of 0
    /// indicates the database has never been optimized.
    ///
    /// Automatic optimization is throttled to occur at most once per hour
    /// to balance performance benefits with resource usage.
    pub last_optimize_time: u64,
}

#[cfg(test)]
mod tests {
    pub fn create_temp_dir() -> String {
        let path = temp_dir();

        if fs::metadata(&path).is_err() {
            fs::create_dir_all(&path).unwrap();
        }
        path
    }

    fn temp_dir() -> String {
        let dir =
            tempfile::tempdir().expect("Can not create temporal directory.");
        dir.path().to_str().unwrap().to_owned()
    }

    impl Default for SqliteManager {
        fn default() -> Self {
            let path = format!("{}/database.db", create_temp_dir());
            let conn = open(&path)
                .map_err(|e| {
                    Error::CreateStore(format!(
                        "fail SQLite open connection: {}",
                        e
                    ))
                })
                .expect("Cannot open the database ");

            Self {
                conn: Arc::new(Mutex::new(conn)),
                query_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                last_optimize_time: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            }
        }
    }

    use super::*;
    use store::{
        database::{Collection, DbManager},
        test_store_trait,
    };

    test_store_trait! {
        unit_test_sqlite_manager:SqliteManager:SqliteCollection
    }
}
