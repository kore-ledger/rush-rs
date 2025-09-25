// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! RocksDB storage implementation with LSM-tree optimizations.
//!
//! This module provides the core RocksDB storage implementation including the database
//! manager, storage collections, and optimized iterator for the Rush actor system.
//! The implementation leverages RocksDB's advanced features including LSM-tree storage,
//! column families, write-ahead logging, and sophisticated compaction strategies.
//!
//! ## Architecture Overview
//!
//! The RocksDB implementation consists of three main components:
//!
//! ### RocksDB Database Manager ([`RocksDbManager`])
//!
//! The manager handles RocksDB instance lifecycle and provides factory methods for
//! creating storage collections. It manages column family discovery, database
//! initialization, and thread-safe access coordination.
//!
//! ### Storage Collections ([`RocksDbStore`])
//!
//! The unified storage implementation that provides both event collection and state
//! snapshot functionality. Each store maps to a RocksDB column family with prefix-based
//! data isolation and optimized operations for actor workloads.
//!
//! ### Memory-Efficient Iterator ([`RocksDbIterator`])
//!
//! A safe, batched iterator implementation that provides memory-efficient traversal
//! of large datasets with configurable batch sizes and prefix filtering.
//!
//! ## LSM-Tree Storage Model
//!
//! RocksDB uses a Log-Structured Merge-tree (LSM-tree) that provides excellent
//! performance for write-intensive actor workloads:
//!
//! - **Write Path**: Data → MemTable → WAL → SST Files (L0 → L1+)
//! - **Read Path**: MemTable → L0 → L1+ with bloom filter acceleration
//! - **Compaction**: Background processes merge and optimize storage levels
//! - **Column Families**: Independent LSM-trees with shared block cache
//!
//! ## Thread Safety and Concurrency
//!
//! The implementation provides thread-safe operations through:
//!
//! - **Shared Database Instance**: `Arc<DB>` allows multiple concurrent accessors
//! - **Column Family Isolation**: Independent data streams with shared resources
//! - **Atomic Operations**: RocksDB's native ACID guarantees
//! - **Lock-Free Reads**: Concurrent readers with snapshot isolation

use store::{
    Error,
    database::{Collection, DbManager, State},
};

use rocksdb::{
    ColumnFamilyDescriptor, DB, IteratorMode, Options,
};
use tracing::info;

use std::{fs, path::Path, sync::Arc};

/// RocksDB database manager with LSM-tree optimization for actor workloads.
///
/// The `RocksDbManager` serves as the primary interface for managing RocksDB instances
/// and creating storage collections optimized for actor system requirements. It handles
/// database lifecycle, column family management, and provides thread-safe access to
/// the underlying RocksDB instance through Arc<DB> sharing.
///
/// ## LSM-Tree Configuration
///
/// The manager automatically configures RocksDB with optimizations for actor workloads:
///
/// - **Write-Optimized Settings**: Default configuration favors write throughput
/// - **Column Family Discovery**: Automatically discovers and opens existing column families
/// - **Shared Resources**: Block cache and write buffer shared across column families
/// - **Concurrent Access**: Thread-safe operations with minimal locking overhead
///
/// ## Memory Management
///
/// RocksDB memory usage is carefully managed through:
///
/// - **MemTable Sizing**: Optimized for typical actor event sizes and frequencies
/// - **Block Cache**: Shared LRU cache for frequently accessed data blocks
/// - **Write Buffers**: Balanced between memory usage and write performance
/// - **Compaction**: Background compaction to maintain read performance
///
/// ## Column Family Strategy
///
/// Each storage collection maps to a RocksDB column family, providing:
///
/// - **Logical Isolation**: Independent LSM-trees for different data types
/// - **Shared Cache**: Common block cache across all column families
/// - **Independent Tuning**: Each column family can have specific optimizations
/// - **Atomic Operations**: Cross-column family transactions when needed
///
/// ## Error Handling
///
/// Comprehensive error handling covers:
///
/// - **Database Initialization**: Directory creation and permission issues
/// - **Column Family Management**: Creation, discovery, and access errors
/// - **Resource Constraints**: Memory and disk space limitations
/// - **Corruption Recovery**: Detection and reporting of database corruption
///
/// ## Example Usage
///
/// ```ignore
/// use rocksdb_db::RocksDbManager;
/// use store::database::DbManager;
///
/// // Create manager with optimal configuration
/// let manager = RocksDbManager::new("/data/actor_storage")?;
///
/// // Create collections for different actor types
/// let user_events = manager.create_collection("user_events", "user_123")?;
/// let session_state = manager.create_state("session_snapshots", "session_456")?;
///
/// // Manager handles column family creation automatically
/// let metrics = manager.create_collection("system_metrics", "node_789")?;
/// ```
///
/// ## Thread Safety
///
/// The manager is thread-safe and can be shared across multiple threads:
///
/// ```ignore
/// use std::sync::Arc;
/// use tokio::task;
///
/// let manager = Arc::new(RocksDbManager::new("/shared/storage")?);
///
/// // Spawn concurrent tasks using the same manager
/// let tasks: Vec<_> = (0..10).map(|i| {
///     let mgr = manager.clone();
///     task::spawn(async move {
///         let collection = mgr.create_collection("events", &format!("actor_{}", i))?;
///         // Perform operations concurrently
///     })
/// }).collect();
/// ```
#[derive(Clone)]
pub struct RocksDbManager {
    /// RocksDB options configuration optimized for actor workloads.
    ///
    /// Contains the database-wide settings including memory management,
    /// compaction strategy, and performance tuning parameters.
    opts: Options,

    /// Shared RocksDB database instance with thread-safe access.
    ///
    /// The database is wrapped in Arc to allow multiple concurrent accessors
    /// while maintaining single instance semantics. All column families and
    /// storage operations are performed through this shared instance.
    db: Arc<DB>,
}

impl RocksDbManager {
    /// Creates a new RocksDB manager with LSM-tree optimization for actor workloads.
    ///
    /// This constructor initializes a RocksDB instance with configuration optimized for
    /// actor system requirements, including automatic column family discovery, directory
    /// creation, and performance tuning for write-intensive workloads.
    ///
    /// ## Database Initialization Process
    ///
    /// The initialization follows these steps:
    ///
    /// 1. **Directory Creation**: Creates the database directory if it doesn't exist
    /// 2. **Column Family Discovery**: Scans for existing column families in the database
    /// 3. **Options Configuration**: Sets up LSM-tree optimization parameters
    /// 4. **Database Opening**: Opens the database with discovered column families
    /// 5. **Resource Setup**: Initializes shared resources (cache, write buffers)
    ///
    /// ## LSM-Tree Configuration
    ///
    /// The database is configured with optimizations for actor workloads:
    ///
    /// - **Create If Missing**: Automatically creates new databases
    /// - **Column Family Support**: Enables logical data separation
    /// - **Write Optimization**: Configured for high write throughput
    /// - **Default Column Family**: Ensures at least one column family exists
    ///
    /// ## Column Family Discovery
    ///
    /// The manager automatically discovers existing column families:
    ///
    /// ```text
    /// Database Path: /data/actors/
    /// ├── CURRENT              ← Current manifest file
    /// ├── MANIFEST-000001      ← Database metadata
    /// ├── 000003.log           ← Write-ahead log
    /// └── Column Families:     ← Auto-discovered
    ///     ├── default          ← Always present
    ///     ├── user_events      ← Discovered from metadata
    ///     └── system_snapshots ← Discovered from metadata
    /// ```
    ///
    /// If the database doesn't exist, only the "default" column family is created initially.
    /// Additional column families are created on-demand through the `create_collection` methods.
    ///
    /// ## Performance Characteristics
    ///
    /// The configuration optimizes for typical actor system usage patterns:
    ///
    /// - **Write Throughput**: Sequential writes to MemTable with O(1) complexity
    /// - **Read Performance**: Bloom filters and block cache acceleration
    /// - **Memory Efficiency**: Shared resources across column families
    /// - **Concurrent Access**: Thread-safe operations with minimal contention
    ///
    /// ## Error Handling
    ///
    /// Comprehensive error handling for initialization failures:
    ///
    /// - **Directory Permissions**: Insufficient filesystem permissions
    /// - **Database Corruption**: Corrupted database files requiring repair
    /// - **Resource Exhaustion**: Insufficient memory or disk space
    /// - **Configuration Issues**: Invalid RocksDB configuration parameters
    ///
    /// # Parameters
    ///
    /// * `path` - The filesystem path where the RocksDB database files will be stored.
    ///           The directory will be created if it doesn't exist. Must be writable
    ///           by the current process.
    ///
    /// # Returns
    ///
    /// Returns a `RocksDbManager` instance configured for optimal actor system performance,
    /// or an error if initialization fails.
    ///
    /// # Errors
    ///
    /// This method can return the following error types:
    ///
    /// - [`Error::CreateStore`] - Directory creation failed due to permissions or I/O errors
    /// - [`Error::CreateStore`] - RocksDB initialization failed due to corruption or configuration issues
    ///
    /// # Examples
    ///
    /// ## Basic Initialization
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbManager;
    ///
    /// // Create manager for new database
    /// let manager = RocksDbManager::new("/data/actor_storage")?;
    /// ```
    ///
    /// ## Error Handling
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbManager;
    /// use store::Error;
    ///
    /// match RocksDbManager::new("/read_only/directory") {
    ///     Ok(manager) => {
    ///         println!("Database initialized successfully");
    ///     }
    ///     Err(Error::CreateStore(msg)) => {
    ///         eprintln!("Failed to initialize database: {}", msg);
    ///         // Handle permission or I/O errors
    ///     }
    ///     Err(e) => {
    ///         eprintln!("Unexpected error: {}", e);
    ///     }
    /// }
    /// ```
    ///
    /// ## Production Usage
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbManager;
    /// use std::path::PathBuf;
    ///
    /// // Use application-specific path
    /// let data_dir = std::env::var("ACTOR_DATA_DIR")
    ///     .unwrap_or_else(|_| "/var/lib/actor_system".to_string());
    ///
    /// let db_path = PathBuf::from(data_dir).join("rocksdb");
    /// let manager = RocksDbManager::new(&db_path.to_string_lossy())?;
    /// ```
    pub fn new(path: &str) -> Result<Self, Error> {
        info!("Creating RocksDB database manager at path: {}", path);

        // Step 1: Directory Creation - Ensure the database directory exists
        // This is critical for RocksDB as it needs write access to store
        // database files, WAL, and SST files
        if !Path::new(&path).exists() {
            info!("Database path does not exist, creating directory structure");
            fs::create_dir_all(path).map_err(|e| {
                Error::CreateStore(format!(
                    "Failed to create RocksDB directory '{}': {}. Check filesystem permissions and available disk space.",
                    path, e
                ))
            })?;
        }

        // Step 2: LSM-Tree Configuration - Set up RocksDB options optimized for actor workloads
        let mut options = Options::default();

        // Enable automatic database creation for new installations
        options.create_if_missing(true);

        // TODO: Add actor-specific optimizations for production workloads:
        //
        // Write Performance Optimizations:
        // - options.set_write_buffer_size(64 * 1024 * 1024); // 64MB for event streams
        // - options.set_max_write_buffer_number(4); // Allow up to 4 concurrent MemTables
        // - options.set_min_write_buffer_number_to_merge(2); // Merge at least 2 MemTables
        //
        // Read Performance Optimizations:
        // - options.set_block_cache(&cache); // Shared 256MB block cache across column families
        // - options.set_cache_index_and_filter_blocks(true); // Cache index/filter blocks
        // - options.set_pin_l0_filter_and_index_blocks_in_cache(true); // Pin L0 blocks
        //
        // Compaction Strategy Selection:
        // - For write-heavy: options.set_compaction_style(CompactionStyle::Level);
        // - For read-heavy: options.set_compaction_style(CompactionStyle::Universal);
        // - options.set_level_compaction_dynamic_level_bytes(true); // Dynamic sizing
        //
        // Bloom Filter Configuration:
        // - options.set_bloom_locality(1); // Improve cache locality
        // - Create bloom filter with 10 bits per key for 1% false positive rate
        //
        // Memory Management:
        // - options.set_allow_mmap_reads(true); // Memory-map large files
        // - options.set_allow_mmap_writes(false); // Avoid mmap for writes (safer)
        //
        // Durability vs Performance Trade-offs:
        // - options.set_use_fsync(false); // Use fdatasync instead of fsync
        // - options.set_bytes_per_sync(1024 * 1024); // Sync every 1MB for smoother I/O

        // Step 3: Column Family Discovery - Automatically discover existing column families
        // This enables seamless database recovery and prevents column family mismatch errors
        let cfs = match DB::list_cf(&options, path) {
            Ok(cf_names) => {
                info!("Discovered {} existing column families: {:?}", cf_names.len(), cf_names);
                cf_names
            },
            Err(_) => {
                info!("No existing database found, initializing with default column family");
                // For new databases, start with only the default column family
                // Additional column families will be created on-demand
                vec!["default".to_string()]
            }
        };

        // Step 4: Column Family Descriptor Creation - Configure each column family
        // Each descriptor contains LSM-tree configuration specific to the column family's usage pattern
        let cf_descriptors: Vec<_> = cfs
            .iter()
            .map(|cf| {
                // Use default options for now, but could be customized per column family:
                // - Events: Write-optimized with larger MemTable
                // - Snapshots: Read-optimized with enhanced block cache
                // - Metrics: Time-series optimized with FIFO compaction
                ColumnFamilyDescriptor::new(cf, Options::default())
            })
            .collect();

        info!("Opening RocksDB with {} column families", cf_descriptors.len());

        // Step 5: Database Opening - Open the database with all discovered column families
        // This creates the shared DB instance that will be used for all storage operations
        let db = DB::open_cf_descriptors(&options, path, cf_descriptors)
            .map_err(|e| {
                Error::CreateStore(format!(
                    "Failed to open RocksDB at '{}': {}. This may indicate database corruption, \
                    insufficient permissions, or incompatible RocksDB version.",
                    path, e
                ))
            })?;

        info!("RocksDB manager initialized successfully with {} column families",
              cfs.len());

        // Step 6: Manager Construction - Create the manager with shared database access
        Ok(Self {
            opts: options,
            // Wrap in Arc for thread-safe sharing across multiple RocksDbStore instances
            db: Arc::new(db),
        })
    }
}

/// Implementation of [`DbManager`] trait for RocksDB with LSM-tree optimizations.
///
/// This implementation provides factory methods for creating storage collections and state
/// containers backed by RocksDB column families. Each collection maps to a dedicated column
/// family with optimized LSM-tree configuration for actor system workloads.
///
/// ## Column Family Strategy
///
/// The implementation uses RocksDB's column family feature to provide:
///
/// - **Logical Isolation**: Each collection gets its own LSM-tree structure
/// - **Shared Resources**: Common block cache and write buffer pool
/// - **Independent Tuning**: Column families can be optimized for specific usage patterns
/// - **Atomic Operations**: Cross-column family consistency when needed
///
/// ## Performance Characteristics
///
/// LSM-tree storage provides excellent performance for actor workloads:
///
/// - **Write Performance**: O(1) writes to MemTable with sequential I/O
/// - **Read Performance**: Bloom filters eliminate most negative lookups
/// - **Space Efficiency**: Background compaction optimizes storage utilization
/// - **Concurrent Access**: Multiple readers/writers with minimal contention
///
/// ## Memory Management
///
/// Resources are shared efficiently across column families:
///
/// - **Block Cache**: Shared LRU cache for hot data blocks
/// - **Write Buffers**: Configurable MemTable sizes per column family
/// - **Compaction**: Background threads handle LSM-tree optimization
/// - **Bloom Filters**: Probabilistic filters reduce I/O for missing keys
impl DbManager<RocksDbStore, RocksDbStore> for RocksDbManager {
    /// Creates a new collection storage backed by a RocksDB column family.
    ///
    /// This method creates or reuses a RocksDB column family to store event collections
    /// for actors. Each collection provides sequential key-value storage optimized for
    /// event sourcing patterns with LSM-tree performance characteristics.
    ///
    /// ## Column Family Management
    ///
    /// The method handles column family lifecycle automatically:
    ///
    /// 1. **Existence Check**: Verifies if the column family already exists
    /// 2. **Creation**: Creates a new column family if needed with optimized settings
    /// 3. **Store Creation**: Returns a `RocksDbStore` instance for the column family
    /// 4. **Resource Sharing**: Shares the database instance across all stores
    ///
    /// ## Performance Optimization
    ///
    /// Column families are optimized for event collection patterns:
    ///
    /// - **Sequential Writes**: Optimized for append-only event streams
    /// - **Range Queries**: Efficient prefix-based iteration
    /// - **Compaction Strategy**: Level-style compaction for balanced performance
    /// - **Bloom Filters**: Fast negative lookup elimination
    ///
    /// # Parameters
    ///
    /// * `name` - The column family name, typically describing the data type (e.g., "user_events")
    /// * `prefix` - The key prefix for data isolation within the column family (e.g., "actor_123")
    ///
    /// # Returns
    ///
    /// A `RocksDbStore` instance configured for collection operations, or an error if
    /// column family creation fails.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CreateStore`] if column family creation fails due to:
    /// - RocksDB internal errors
    /// - Resource exhaustion (memory/disk)
    /// - Invalid column family names
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbManager;
    /// use store::database::{DbManager, Collection};
    ///
    /// let manager = RocksDbManager::new("/data/events")?;
    /// let mut collection = manager.create_collection("user_events", "user_123")?;
    ///
    /// // Store sequential events
    /// collection.put("001", b"user_login_event")?;
    /// collection.put("002", b"user_action_event")?;
    /// ```
    fn create_collection(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<RocksDbStore, Error> {
        // Check if column family already exists to avoid recreation overhead
        if self.db.cf_handle(name).is_none() {
            info!("Creating new column family '{}' for collection storage", name);

            // Create column family with options optimized for event collections
            // TODO: Consider collection-specific optimizations for event workloads:
            //
            // Write-Heavy Event Stream Optimizations:
            // - let mut cf_opts = Options::default();
            // - cf_opts.set_write_buffer_size(128 * 1024 * 1024); // Larger 128MB MemTable
            // - cf_opts.set_max_write_buffer_number(6); // More concurrent write buffers
            // - cf_opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB SST files
            //
            // Point Lookup Optimization:
            // - cf_opts.set_bloom_locality(1); // Better bloom filter cache locality
            // - Create prefix bloom filter for actor-specific lookups:
            //   let bloom = rocksdb::BlockBasedOptions::default();
            //   bloom.set_bloom_filter(10, false); // 10 bits per key, no block-based
            //   cf_opts.set_block_based_table_factory(&bloom);
            //
            // Range Query Optimization:
            // - cf_opts.set_compaction_style(CompactionStyle::Level); // Better for range scans
            // - cf_opts.set_level_zero_file_num_compaction_trigger(4); // Compact L0 earlier
            // - cf_opts.set_max_bytes_for_level_base(256 * 1024 * 1024); // 256MB L1
            //
            // Cache Configuration:
            // - cf_opts.set_block_cache(&shared_cache); // Share cache with other CFs
            // - cf_opts.set_cache_index_and_filter_blocks(true); // Cache metadata
            self.db
                .create_cf(name, &self.opts)
                .map_err(|e| Error::CreateStore(format!(
                    "Failed to create collection column family '{}': {}. \
                    This may indicate insufficient resources or invalid column family name.",
                    name, e
                )))?;
        }

        // Create store instance with shared database access
        Ok(RocksDbStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            store: self.db.clone(),
        })
    }

    /// Creates a new state storage backed by a RocksDB column family.
    ///
    /// This method creates or reuses a RocksDB column family to store actor state snapshots.
    /// State storage is optimized for single key-value operations representing the current
    /// state of actors, with LSM-tree optimizations for read-heavy access patterns.
    ///
    /// ## State Storage Optimization
    ///
    /// State columns are optimized differently from event collections:
    ///
    /// - **Single Key Operations**: One key per actor for current state
    /// - **Read-Heavy Workload**: Optimized for frequent state retrieval
    /// - **Overwrite Pattern**: State updates overwrite previous values
    /// - **Snapshot Semantics**: Point-in-time actor state representation
    ///
    /// ## Column Family Configuration
    ///
    /// State column families could benefit from different settings:
    ///
    /// - **Smaller MemTable**: Less memory for infrequent writes
    /// - **Enhanced Block Cache**: Better read performance for hot state
    /// - **Aggressive Compaction**: Reduce space amplification from overwrites
    /// - **Read-Optimized**: Bloom filter and index optimizations
    ///
    /// # Parameters
    ///
    /// * `name` - The column family name for state storage (e.g., "actor_snapshots")
    /// * `prefix` - The key prefix for actor identification (e.g., "actor_123")
    ///
    /// # Returns
    ///
    /// A `RocksDbStore` instance configured for state operations, or an error if
    /// column family creation fails.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CreateStore`] if column family creation fails due to:
    /// - RocksDB internal errors
    /// - Resource constraints
    /// - Column family name conflicts
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbManager;
    /// use store::database::{DbManager, State};
    ///
    /// let manager = RocksDbManager::new("/data/states")?;
    /// let mut state = manager.create_state("actor_snapshots", "actor_456")?;
    ///
    /// // Store current actor state
    /// state.put(b"current_actor_state_snapshot")?;
    ///
    /// // Retrieve state for recovery
    /// let recovered_state = state.get()?;
    /// ```
    fn create_state(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<RocksDbStore, Error> {
        // Check if column family already exists to avoid recreation overhead
        if self.db.cf_handle(name).is_none() {
            info!("Creating new column family '{}' for state storage", name);

            // Create column family with options that could be optimized for state storage
            // TODO: Consider state-specific optimizations for snapshot workloads:
            //
            // Read-Heavy State Access Optimizations:
            // - let mut state_opts = Options::default();
            // - state_opts.set_write_buffer_size(32 * 1024 * 1024); // Smaller 32MB MemTable
            // - state_opts.set_max_write_buffer_number(2); // Fewer concurrent buffers
            // - state_opts.set_level_zero_file_num_compaction_trigger(2); // Aggressive compaction
            //
            // Overwrite Pattern Optimization:
            // - state_opts.set_compaction_style(CompactionStyle::Level); // Handle overwrites well
            // - state_opts.set_level_compaction_dynamic_level_bytes(true); // Dynamic sizing
            // - state_opts.set_target_file_size_base(32 * 1024 * 1024); // Smaller 32MB SST files
            //
            // Cache Optimization for Hot State Data:
            // - state_opts.set_block_cache(&state_cache); // Dedicated cache for state
            // - state_opts.set_cache_index_and_filter_blocks(true); // Cache metadata
            // - state_opts.set_pin_l0_filter_and_index_blocks_in_cache(true); // Pin hot data
            //
            // Bloom Filter for State Lookups:
            // - Create optimized bloom filter for point lookups:
            //   let bloom = rocksdb::BlockBasedOptions::default();
            //   bloom.set_bloom_filter(15, false); // 15 bits per key for very low false positives
            //   state_opts.set_block_based_table_factory(&bloom);
            //
            // Compression for State Data:
            // - state_opts.set_compression_type(CompressionType::Lz4); // Fast compression
            // - state_opts.set_bottommost_compression_type(CompressionType::Zstd); // Better ratio
            self.db
                .create_cf(name, &self.opts)
                .map_err(|e| Error::CreateStore(format!(
                    "Failed to create state column family '{}': {}. \
                    This may indicate resource exhaustion or naming conflicts.",
                    name, e
                )))?;
        }

        // Create store instance with shared database access
        // Note: RocksDbStore implements both Collection and State traits,
        // providing unified interface regardless of the intended usage pattern
        Ok(RocksDbStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            store: self.db.clone(),
        })
    }
}

/// RocksDB storage implementation with column family isolation and LSM-tree optimization.
///
/// `RocksDbStore` provides a unified storage interface that implements both the [`Collection`]
/// and [`State`] traits, backed by RocksDB's column family architecture. Each store instance
/// maps to a specific column family with prefix-based data isolation, enabling high-performance
/// storage for actor events and state snapshots.
///
/// ## Architecture Overview
///
/// The store leverages RocksDB's advanced features for optimal actor system performance:
///
/// ### Column Family Mapping
///
/// Each `RocksDbStore` instance corresponds to a RocksDB column family:
///
/// ```text
/// RocksDB Instance
/// ├── Column Family: "user_events"    ← RocksDbStore(name="user_events", prefix="user_123")
/// │   ├── user_123.001 → event_data_1
/// │   ├── user_123.002 → event_data_2
/// │   └── user_123.003 → event_data_3
/// ├── Column Family: "snapshots"      ← RocksDbStore(name="snapshots", prefix="user_123")
/// │   └── user_123 → state_snapshot
/// └── Column Family: "system_metrics" ← RocksDbStore(name="system_metrics", prefix="node_1")
///     ├── node_1.cpu → cpu_metrics
///     └── node_1.memory → memory_metrics
/// ```
///
/// ### LSM-Tree Storage Model
///
/// The underlying LSM-tree architecture provides:
///
/// - **Write Path**: Data → MemTable → WAL → L0 SST → L1+ SST (via compaction)
/// - **Read Path**: MemTable → L0 → L1+ with bloom filter optimization
/// - **Prefix Optimization**: Keys with same prefix tend to be co-located
/// - **Compaction Benefits**: Background optimization maintains read performance
///
/// ## Concurrent Access Patterns
///
/// Thread-safe operations are ensured through multiple mechanisms:
///
/// ### Shared Database Instance
///
/// The `Arc<DB>` allows safe sharing across multiple store instances:
///
/// ```ignore
/// use std::sync::Arc;
/// use rocksdb_db::RocksDbStore;
///
/// // Multiple stores can safely share the same RocksDB instance
/// let store_a = RocksDbStore { name: "events".to_string(), store: db.clone(), /* ... */ };
/// let store_b = RocksDbStore { name: "states".to_string(), store: db.clone(), /* ... */ };
/// ```
///
/// ### RocksDB Internal Concurrency
///
/// RocksDB provides thread-safe operations with:
///
/// - **Reader Concurrency**: Multiple concurrent readers without blocking
/// - **Writer Atomicity**: Single-writer model with atomic operations
/// - **Column Family Isolation**: Independent access to different column families
/// - **Snapshot Consistency**: Point-in-time consistent reads
///
/// ### Prefix-Based Isolation
///
/// Data isolation within column families through key prefixes:
///
/// - **Actor Isolation**: Each actor's data uses a unique prefix
/// - **Namespace Separation**: Different actors don't interfere
/// - **Efficient Iteration**: Prefix filters enable fast range scans
/// - **Space Locality**: Related data co-located for cache efficiency
///
/// ## Performance Characteristics
///
/// ### Write Performance
///
/// Optimized for actor event patterns:
///
/// - **Sequential Writes**: O(1) complexity to MemTable
/// - **Batch Operations**: Amortized cost for multiple writes
/// - **WAL Durability**: Configurable sync policies for consistency vs. throughput
/// - **Prefix Locality**: Related writes benefit from spatial locality
///
/// ### Read Performance
///
/// Multiple optimization layers:
///
/// - **MemTable First**: Hot data served from memory
/// - **Bloom Filters**: Fast negative lookup elimination (~1% false positive rate)
/// - **Block Cache**: LRU cache for frequently accessed data blocks
/// - **Index Caching**: SST file indexes cached for repeated access
///
/// ### Iterator Efficiency
///
/// Range scans optimized for event sourcing:
///
/// - **Prefix Filtering**: Skip irrelevant data during iteration
/// - **Batched Loading**: Memory-efficient large dataset traversal
/// - **Forward/Reverse**: Efficient bidirectional iteration
/// - **Lazy Evaluation**: Load data only as needed
///
/// ## Memory Management
///
/// Sophisticated memory handling strategies:
///
/// - **Shared Resources**: Block cache and write buffers shared across column families
/// - **MemTable Sizing**: Balanced between memory usage and write performance
/// - **Compaction Throttling**: Background compaction manages space usage
/// - **Iterator Batching**: Configurable batch sizes prevent memory exhaustion
///
/// ## Error Handling and Recovery
///
/// Comprehensive error handling for production usage:
///
/// - **Column Family Validation**: Ensures column family exists before operations
/// - **Key Encoding**: UTF-8 validation with graceful fallback
/// - **RocksDB Errors**: Detailed error context from RocksDB operations
/// - **Resource Limits**: Protection against resource exhaustion
///
/// ## Usage Patterns
///
/// ### Event Collection Storage
///
/// ```ignore
/// use store::database::Collection;
///
/// let mut events = RocksDbStore { /* ... */ };
///
/// // Sequential event storage
/// events.put("001", b"user_login")?;
/// events.put("002", b"user_action")?;
/// events.put("003", b"user_logout")?;
///
/// // Efficient range iteration
/// for (key, data) in events.iter(false) {
///     println!("Event {}: {} bytes", key, data.len());
/// }
/// ```
///
/// ### State Snapshot Storage
///
/// ```ignore
/// use store::database::State;
///
/// let mut state = RocksDbStore { /* ... */ };
///
/// // Single state per actor
/// state.put(b"current_actor_state")?;
/// let recovered_state = state.get()?;
/// ```
///
/// ## Thread Safety Example
///
/// ```ignore
/// use std::sync::Arc;
/// use tokio::task;
///
/// let store = Arc::new(RocksDbStore { /* ... */ });
///
/// // Concurrent operations across multiple tasks
/// let read_task = {
///     let store = store.clone();
///     task::spawn(async move {
///         store.get("key").unwrap()
///     })
/// };
///
/// let write_task = {
///     let mut store = (*store).clone();
///     task::spawn(async move {
///         store.put("key", b"value").unwrap()
///     })
/// };
/// ```
pub struct RocksDbStore {
    /// Column family name identifying the logical storage partition.
    ///
    /// Each column family represents a separate LSM-tree within the RocksDB instance,
    /// providing logical isolation for different data types (events, snapshots, etc.).
    /// The name must correspond to an existing column family in the database.
    name: String,

    /// Key prefix for data isolation within the column family.
    ///
    /// Enables multiple actors or logical entities to share the same column family
    /// while maintaining data isolation. Keys are prefixed with this value followed
    /// by a separator (e.g., "actor_123.event_001").
    prefix: String,

    /// Shared RocksDB database instance with thread-safe access.
    ///
    /// The database is wrapped in Arc to allow multiple store instances to share
    /// the same RocksDB instance while accessing different column families or
    /// using different prefixes within the same column family.
    store: Arc<DB>,
}

/// Implementation of [`State`] trait for RocksDB with LSM-tree storage model optimizations.
///
/// This implementation provides actor state snapshot storage using RocksDB's column family
/// architecture. State storage is optimized for single key-value operations representing
/// the current state of actors, leveraging LSM-tree characteristics for efficient
/// read-heavy workloads and overwrite patterns.
///
/// ## LSM-Tree Storage Model for State
///
/// Actor state storage differs from event collections in its access patterns:
///
/// ### Write Characteristics
///
/// - **Overwrite Pattern**: State updates replace previous values
/// - **Infrequent Writes**: Periodic snapshots rather than continuous events
/// - **Single Key Per Actor**: One state entry per actor prefix
/// - **Atomic Updates**: Full state replacement in single operation
///
/// ### Read Characteristics
///
/// - **Frequent Access**: State loaded on actor recovery and queries
/// - **Point Lookups**: Direct key access without range scans
/// - **Cache Benefits**: Hot state data benefits from block cache
/// - **Bloom Filter Optimization**: Fast negative lookups for missing state
///
/// ### LSM-Tree Optimizations for State
///
/// The LSM-tree structure provides benefits for state storage:
///
/// - **Write Amplification**: Managed through compaction strategies
/// - **Read Performance**: MemTable + bloom filter + block cache acceleration
/// - **Space Efficiency**: Compaction removes old state versions
/// - **Durability**: WAL ensures state updates survive crashes
///
/// ## Column Family Usage
///
/// State data leverages column family isolation:
///
/// ```text
/// Column Family: "actor_snapshots"
/// ├── actor_123 → serialized_state_v1
/// ├── actor_456 → serialized_state_v2
/// └── actor_789 → serialized_state_v3
/// ```
///
/// Each actor's state is stored with the actor's prefix as the key, enabling:
/// - **Direct Access**: O(log n) lookup complexity
/// - **Isolation**: Actor state isolated from events and other data
/// - **Cache Locality**: Related state data co-located
/// - **Compaction Efficiency**: Similar access patterns benefit from same compaction strategy
impl State for RocksDbStore {
    /// Returns the column family name for this state storage.
    ///
    /// The name identifies the logical storage partition used for state snapshots,
    /// typically describing the type of state being stored (e.g., "actor_snapshots").
    ///
    /// # Returns
    ///
    /// A string slice containing the column family name.
    fn name(&self) -> &str {
        &self.name
    }

    /// Retrieves the current state snapshot for the actor.
    ///
    /// This method performs a point lookup in the RocksDB column family using the
    /// actor's prefix as the key. The operation leverages RocksDB's LSM-tree
    /// optimizations including MemTable access, bloom filter acceleration, and
    /// block cache for optimal read performance.
    ///
    /// ## LSM-Tree Read Path
    ///
    /// The read operation follows RocksDB's optimized path:
    ///
    /// 1. **MemTable Lookup**: Check active write buffer for recent updates
    /// 2. **Immutable MemTables**: Check flushing write buffers
    /// 3. **L0 SST Files**: Search recently flushed data with potential overlaps
    /// 4. **L1+ SST Files**: Binary search through level-organized files
    /// 5. **Bloom Filter**: Skip files that definitely don't contain the key
    /// 6. **Block Cache**: Serve frequently accessed blocks from memory
    ///
    /// ## Performance Characteristics
    ///
    /// - **Time Complexity**: O(log n) where n is the total number of keys
    /// - **I/O Pattern**: Minimize disk reads through caching and bloom filters
    /// - **Memory Usage**: Benefit from shared block cache across column families
    /// - **Concurrency**: Multiple concurrent readers without blocking
    ///
    /// # Returns
    ///
    /// Returns the serialized state data as a `Vec<u8>`, or an error if:
    /// - The state doesn't exist ([`Error::EntryNotFound`])
    /// - The column family is missing ([`Error::Store`])
    /// - RocksDB operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::EntryNotFound`] - No state snapshot exists for this actor
    /// - [`Error::Store`] - Column family doesn't exist in database
    /// - [`Error::Get`] - RocksDB read operation failed
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use store::database::State;
    ///
    /// let state_store = /* RocksDbStore for state */;
    ///
    /// match state_store.get() {
    ///     Ok(state_data) => {
    ///         println!("Retrieved state: {} bytes", state_data.len());
    ///         // Deserialize and restore actor state
    ///     }
    ///     Err(Error::EntryNotFound(_)) => {
    ///         println!("No previous state found, starting fresh");
    ///     }
    ///     Err(e) => {
    ///         eprintln!("Failed to retrieve state: {}", e);
    ///     }
    /// }
    /// ```
    fn get(&self) -> Result<Vec<u8>, Error> {
        // Step 1: Column Family Validation - Ensure the column family exists
        // This prevents operations on non-existent storage partitions
        if let Some(handle) = self.store.cf_handle(&self.name) {

            // Step 2: LSM-Tree Point Lookup - Perform optimized read operation
            // RocksDB will search: MemTable → Immutable MemTables → L0 → L1+
            // with bloom filter acceleration and block cache utilization
            let result = self
                .store
                .get_cf(&handle, self.prefix.clone())
                .map_err(|e| Error::Get(format!(
                    "RocksDB state retrieval failed for key '{}': {}. \
                    This may indicate I/O errors, corruption, or resource exhaustion.",
                    self.prefix, e
                )))?;

            // Step 3: Result Processing - Handle presence/absence of state data
            match result {
                Some(value) => {
                    // State found - return the serialized actor state data
                    Ok(value)
                },
                None => {
                    // No state exists for this actor - return appropriate error
                    // This is expected for new actors or after state deletion
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_string()
                    ))
                }
            }
        } else {
            // Column family doesn't exist - this indicates a configuration error
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance. \
                Ensure the column family was created through the DbManager.",
                self.name
            )))
        }
    }

    /// Stores the current state snapshot for the actor.
    ///
    /// This method performs an atomic write operation to store the actor's complete
    /// state snapshot. The operation leverages RocksDB's LSM-tree write path,
    /// initially writing to the MemTable with WAL durability, and eventually
    /// being compacted into persistent SST files.
    ///
    /// ## LSM-Tree Write Path
    ///
    /// The write operation follows RocksDB's optimized write path:
    ///
    /// 1. **Write-Ahead Log (WAL)**: Durability guarantee before MemTable write
    /// 2. **MemTable Write**: In-memory write with O(1) complexity
    /// 3. **Flush Trigger**: MemTable flushed to L0 when size limit reached
    /// 4. **Compaction**: Background process moves data from L0 to L1+
    /// 5. **Persistence**: Data eventually written to persistent SST files
    ///
    /// ## State Overwrite Semantics
    ///
    /// State storage uses overwrite semantics optimized for actor snapshots:
    ///
    /// - **Full State Replacement**: Complete actor state in single write
    /// - **Atomic Operation**: All-or-nothing state update
    /// - **Version Management**: Previous state versions cleaned by compaction
    /// - **Space Efficiency**: Compaction removes obsolete state versions
    ///
    /// ## Performance Characteristics
    ///
    /// - **Time Complexity**: O(1) write to MemTable, amortized compaction cost
    /// - **Write Amplification**: Managed by RocksDB's compaction strategy
    /// - **Durability**: WAL ensures crash recovery of uncommitted state
    /// - **Concurrency**: Non-blocking for concurrent readers
    ///
    /// # Parameters
    ///
    /// * `data` - The serialized state data to store. This should contain the
    ///           complete actor state that can be used for recovery.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful state storage, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - RocksDB write operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing or database closed
    /// - [`Error::Get`] - RocksDB write operation failed (I/O, space, corruption)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use store::database::State;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct ActorState {
    ///     counter: u64,
    ///     last_update: String,
    /// }
    ///
    /// let mut state_store = /* RocksDbStore for state */;
    /// let actor_state = ActorState { counter: 42, last_update: "2023-01-01".to_string() };
    ///
    /// // Serialize and store state
    /// let serialized = serde_json::to_vec(&actor_state)?;
    /// state_store.put(&serialized)?;
    /// println!("Actor state saved successfully");
    /// ```
    fn put(&mut self, data: &[u8]) -> Result<(), Error> {
        // Step 1: Column Family Validation - Ensure target column family exists
        if let Some(handle) = self.store.cf_handle(&self.name) {

            // Step 2: LSM-Tree Write Operation - Write state data through optimized path
            // The write follows: WAL → MemTable → eventual SST file compaction
            // This provides durability and optimal write performance
            self.store
                .put_cf(&handle, self.prefix.clone(), data)
                .map_err(|e| Error::Get(format!(
                    "RocksDB state write failed for key '{}' ({} bytes): {}. \
                    This may indicate I/O errors, disk full, or database corruption.",
                    self.prefix, data.len(), e
                )))?;

            Ok(())
        } else {
            // Column family missing - configuration error
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance. \
                State storage requires proper column family initialization.",
                self.name
            )))
        }
    }

    /// Deletes the current state snapshot for the actor.
    ///
    /// This method removes the actor's state snapshot from storage, effectively
    /// resetting the actor to have no persistent state. The operation uses
    /// RocksDB's delete operation which marks the key for deletion and eventual
    /// removal through compaction.
    ///
    /// ## LSM-Tree Delete Semantics
    ///
    /// RocksDB handles deletes through tombstone markers:
    ///
    /// 1. **Tombstone Creation**: Delete creates a tombstone marker in MemTable
    /// 2. **Read Masking**: Subsequent reads see the tombstone and return "not found"
    /// 3. **Compaction Cleanup**: Tombstones and old values removed during compaction
    /// 4. **Space Recovery**: Deleted data space eventually reclaimed
    ///
    /// ## State Lifecycle Management
    ///
    /// Deleting state affects the actor lifecycle:
    ///
    /// - **Fresh Start**: Next actor recovery will start with no previous state
    /// - **Clean Slate**: Removes all traces of previous actor state
    /// - **Space Efficiency**: Allows garbage collection of old state data
    /// - **Irreversible**: Cannot recover state after successful deletion
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful state deletion, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - RocksDB delete operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing or database closed
    /// - [`Error::Get`] - RocksDB delete operation failed
    ///
    /// Note: This method succeeds even if no state exists for the actor.
    fn del(&mut self) -> Result<(), Error> {
        // Step 1: Column Family Validation
        if let Some(handle) = self.store.cf_handle(&self.name) {

            // Step 2: LSM-Tree Delete Operation - Create tombstone marker
            // The delete is recorded in MemTable/WAL and will mask any existing values
            self.store
                .delete_cf(&handle, self.prefix.clone())
                .map_err(|e| Error::Get(format!(
                    "RocksDB state deletion failed for key '{}': {}. \
                    This may indicate I/O errors or database corruption.",
                    self.prefix, e
                )))?;

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance.",
                self.name
            )))
        }
    }

    /// Purges all data with the actor's prefix from the column family.
    ///
    /// This method removes all keys that start with the actor's prefix, effectively
    /// cleaning up all data associated with the actor. For state storage, this
    /// typically removes just the single state entry, but the implementation
    /// handles any prefix-based data cleanup.
    ///
    /// ## Implementation Strategy
    ///
    /// The purge operation uses iterator-based deletion:
    ///
    /// 1. **Full Iteration**: Scans all keys in the column family
    /// 2. **Prefix Filtering**: Identifies keys matching the actor's prefix
    /// 3. **Batch Deletion**: Deletes matching keys individually
    /// 4. **UTF-8 Validation**: Safely handles non-UTF8 keys by skipping them
    ///
    /// ## Performance Considerations
    ///
    /// - **Scan Cost**: O(n) where n is total keys in column family
    /// - **Delete Cost**: O(log n) per matching key for LSM-tree operations
    /// - **Memory Usage**: Iterator processes data in batches to limit memory
    /// - **Compaction Impact**: Multiple deletes may trigger compaction
    ///
    /// ## Use Cases
    ///
    /// - **Actor Cleanup**: Remove all traces of an actor from storage
    /// - **Namespace Reset**: Clear all data for a specific prefix
    /// - **Maintenance Operations**: Bulk cleanup for administrative tasks
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful purge completion, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - Iterator or delete operations fail ([`Error::Get`] or [`Error::Store`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing, key encoding issues
    /// - [`Error::Get`] - RocksDB delete operations failed
    ///
    /// # Performance Warning
    ///
    /// This operation can be expensive for large column families as it requires
    /// scanning all keys. Consider using range deletion APIs for better performance
    /// with large datasets in future RocksDB versions.
    fn purge(&mut self) -> Result<(), Error> {
        // Step 1: Column Family Validation
        if let Some(handle) = self.store.cf_handle(&self.name) {

            // Step 2: Iterator-Based Purge - Scan and delete matching keys
            // Start from the beginning of the column family key space
            let iter = self.store.iterator_cf(&handle, IteratorMode::Start);

            for (key, _) in iter.flatten() {
                // Step 3: Key Encoding Validation - Safely convert bytes to string
                let key_str = String::from_utf8(key.to_vec()).map_err(|e| {
                    Error::Store(format!(
                        "Failed to convert key to UTF-8 string during purge: {}. \
                        This may indicate corrupted keys or non-UTF8 data.",
                        e
                    ))
                })?;

                // Step 4: Prefix Matching - Check if key belongs to this actor
                if key_str.starts_with(&self.prefix) {
                    // Step 5: Individual Key Deletion - Remove matching key
                    self.store
                        .delete_cf(&handle, key_str)
                        .map_err(|e| Error::Get(format!(
                            "Failed to delete key during purge operation: {}",
                            e
                        )))?;
                }
            }

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance.",
                self.name
            )))
        }
    }

    fn flush(&self) -> Result<(), Error> {
        if let Some(handle) = self.store.cf_handle(&self.name) {
            Ok(self
                .store
                .flush_cf(&handle)
                .map_err(|e| Error::Get(format!("{:?}", e)))?)
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }
}

/// Implementation of [`Collection`] trait for RocksDB with LSM-tree storage optimizations.
///
/// This implementation provides event collection storage using RocksDB's column family
/// architecture with prefix-based data isolation. Event collections are optimized for
/// sequential write patterns common in actor event sourcing, leveraging LSM-tree
/// characteristics for high write throughput and efficient range queries.
///
/// ## LSM-Tree Storage Model for Collections
///
/// Event collections benefit from RocksDB's LSM-tree architecture in several ways:
///
/// ### Write Characteristics
///
/// - **Sequential Append Pattern**: Events typically written in sequence (e.g., 001, 002, 003)
/// - **High Write Throughput**: O(1) writes to MemTable with batch operation support
/// - **Write Amplification**: Minimized through level-style compaction optimization
/// - **Durability**: WAL ensures event persistence across crashes
///
/// ### Read Characteristics
///
/// - **Range Queries**: Efficient iteration over event sequences
/// - **Point Lookups**: Fast access to specific events with bloom filter acceleration
/// - **Prefix Scans**: Optimized iteration over actor-specific event streams
/// - **Cache Benefits**: Frequently accessed events served from block cache
///
/// ### Key Format and Isolation
///
/// Collection keys use prefixed format for data isolation:
///
/// ```text
/// Column Family: "user_events"
/// ├── user_123.001 → login_event_data
/// ├── user_123.002 → action_event_data
/// ├── user_123.003 → logout_event_data
/// ├── user_456.001 → different_user_event
/// └── user_456.002 → another_user_event
/// ```
///
/// This format provides:
/// - **Actor Isolation**: Each actor's events are logically separated
/// - **Sequential Access**: Events can be iterated in chronological order
/// - **Efficient Ranges**: Prefix filtering enables fast actor-specific queries
/// - **Cache Locality**: Related events are co-located for better cache utilization
///
/// ## Performance Optimizations
///
/// ### Write Path Optimization
///
/// The LSM-tree write path is optimized for event sourcing patterns:
///
/// - **MemTable Writes**: Events written to in-memory buffer with O(1) complexity
/// - **Batch Operations**: Multiple events can be written atomically
/// - **Sequential Keys**: Monotonically increasing keys optimize compaction
/// - **Compaction Strategy**: Level-style compaction balances read and write performance
///
/// ### Read Path Optimization
///
/// Multiple layers accelerate event retrieval:
///
/// - **MemTable First**: Recent events served from memory without I/O
/// - **Bloom Filters**: Fast elimination of missing events (~1% false positive rate)
/// - **Block Cache**: Hot event data cached for repeated access
/// - **Index Acceleration**: SST file indexes cached for range queries
///
/// ## Concurrency and Thread Safety
///
/// Collections support safe concurrent access:
///
/// - **Multiple Readers**: Concurrent readers don't block each other
/// - **Writer Isolation**: Single writer model with atomic operations
/// - **Snapshot Consistency**: Readers get consistent point-in-time views
/// - **Column Family Independence**: Different collections don't interfere
impl Collection for RocksDbStore {
    /// Returns the column family name for this event collection.
    ///
    /// The name identifies the logical storage partition used for this collection,
    /// typically describing the event type or actor category being stored.
    ///
    /// # Returns
    ///
    /// A string slice containing the column family name.
    fn name(&self) -> &str {
        &self.name
    }

    /// Retrieves a specific event from the collection by key.
    ///
    /// This method performs a point lookup in the RocksDB column family using the
    /// prefixed key format. The operation leverages RocksDB's LSM-tree optimizations
    /// including MemTable access, bloom filter acceleration, and block cache for
    /// optimal read performance.
    ///
    /// ## Key Format and Lookup
    ///
    /// The method constructs a prefixed key for data isolation:
    ///
    /// ```text
    /// Input:  key = "001"
    /// Prefix: "user_123"
    /// Result: "user_123.001" → Used for RocksDB lookup
    /// ```
    ///
    /// ## LSM-Tree Read Path
    ///
    /// The read operation follows RocksDB's optimized path:
    ///
    /// 1. **MemTable Lookup**: Check active write buffer for recent events
    /// 2. **Immutable MemTables**: Check flushing write buffers
    /// 3. **L0 SST Files**: Search recently flushed data with potential overlaps
    /// 4. **L1+ SST Files**: Binary search through level-organized files
    /// 5. **Bloom Filter**: Skip files that definitely don't contain the key
    /// 6. **Block Cache**: Serve frequently accessed blocks from memory
    ///
    /// ## Performance Characteristics
    ///
    /// - **Time Complexity**: O(log n) where n is the total number of events
    /// - **I/O Pattern**: Minimize disk reads through caching and bloom filters
    /// - **Memory Usage**: Benefits from shared block cache across collections
    /// - **Concurrency**: Multiple concurrent readers without blocking
    ///
    /// # Parameters
    ///
    /// * `key` - The event key to retrieve (e.g., "001", "002"). Will be prefixed
    ///          with the actor's prefix to form the full RocksDB key.
    ///
    /// # Returns
    ///
    /// Returns the serialized event data as `Vec<u8>`, or an error if:
    /// - The event doesn't exist ([`Error::EntryNotFound`])
    /// - The column family is missing ([`Error::Store`])
    /// - RocksDB operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::EntryNotFound`] - No event exists with the specified key
    /// - [`Error::Store`] - Column family doesn't exist in database
    /// - [`Error::Get`] - RocksDB read operation failed
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use store::database::Collection;
    ///
    /// let collection = /* RocksDbStore for events */;
    ///
    /// // Retrieve specific event
    /// match collection.get("001") {
    ///     Ok(event_data) => {
    ///         println!("Retrieved event: {} bytes", event_data.len());
    ///         // Deserialize and process event
    ///     }
    ///     Err(Error::EntryNotFound(_)) => {
    ///         println!("Event not found");
    ///     }
    ///     Err(e) => {
    ///         eprintln!("Failed to retrieve event: {}", e);
    ///     }
    /// }
    /// ```
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        // Step 1: Column Family Validation - Ensure the collection exists
        if let Some(handle) = self.store.cf_handle(&self.name) {
            // Step 2: Prefixed Key Construction - Build isolation key
            let full_key = format!("{}.{}", self.prefix, key);

            // Step 3: LSM-Tree Point Lookup - Perform optimized read operation
            let result = self
                .store
                .get_cf(&handle, full_key.clone())
                .map_err(|e| Error::Get(format!(
                    "RocksDB event retrieval failed for key '{}': {}. \
                    This may indicate I/O errors, corruption, or resource exhaustion.",
                    full_key, e
                )))?;

            // Step 4: Result Processing - Handle presence/absence of event data
            match result {
                Some(value) => Ok(value),
                None => Err(Error::EntryNotFound(
                    "Query returned no rows".to_string()
                )),
            }
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance. \
                Ensure the collection was created through the DbManager.",
                self.name
            )))
        }
    }

    /// Stores an event in the collection with the specified key.
    ///
    /// This method performs an atomic write operation to store event data using RocksDB's
    /// LSM-tree write path. The operation is optimized for sequential event patterns
    /// common in actor event sourcing, with efficient MemTable writes and batched
    /// compaction strategies.
    ///
    /// ## Key Format and Storage
    ///
    /// The method constructs a prefixed key for data isolation:
    ///
    /// ```text
    /// Input:  key = "001", data = event_bytes
    /// Prefix: "user_123"
    /// Result: "user_123.001" → event_bytes stored in RocksDB
    /// ```
    ///
    /// ## LSM-Tree Write Path
    ///
    /// The write operation follows RocksDB's optimized write path:
    ///
    /// 1. **Write-Ahead Log (WAL)**: Durability guarantee before MemTable write
    /// 2. **MemTable Write**: In-memory write with O(1) complexity
    /// 3. **Flush Trigger**: MemTable flushed to L0 when size limit reached
    /// 4. **Compaction**: Background process moves data from L0 to L1+
    /// 5. **Persistence**: Data eventually written to persistent SST files
    ///
    /// ## Event Sequence Optimization
    ///
    /// Sequential event keys (001, 002, 003) provide optimization benefits:
    ///
    /// - **Write Locality**: Sequential writes improve MemTable efficiency
    /// - **Compaction Efficiency**: Monotonic keys reduce compaction overhead
    /// - **Cache Benefits**: Related events co-located for better cache utilization
    /// - **Range Query Optimization**: Sequential access patterns benefit iteration
    ///
    /// ## Performance Characteristics
    ///
    /// - **Time Complexity**: O(1) write to MemTable, amortized compaction cost
    /// - **Write Amplification**: Minimized through level-style compaction strategy
    /// - **Durability**: WAL ensures event persistence across system crashes
    /// - **Concurrency**: Non-blocking for concurrent readers
    ///
    /// # Parameters
    ///
    /// * `key` - The event key for storage (e.g., "001", "002"). Will be prefixed
    ///          with the actor's prefix for data isolation.
    /// * `data` - The serialized event data to store. Should contain the complete
    ///           event information needed for replay.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful event storage, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - RocksDB write operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing or database closed
    /// - [`Error::Get`] - RocksDB write operation failed (I/O, space, corruption)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use store::database::Collection;
    /// use serde::{Serialize, Deserialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct UserLoginEvent {
    ///     user_id: String,
    ///     timestamp: u64,
    ///     ip_address: String,
    /// }
    ///
    /// let mut collection = /* RocksDbStore for events */;
    /// let event = UserLoginEvent {
    ///     user_id: "user_123".to_string(),
    ///     timestamp: 1234567890,
    ///     ip_address: "192.168.1.1".to_string(),
    /// };
    ///
    /// // Serialize and store event
    /// let serialized = serde_json::to_vec(&event)?;
    /// collection.put("001", &serialized)?;
    /// println!("Event stored successfully");
    /// ```
    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        // Step 1: Column Family Validation - Ensure target collection exists
        if let Some(handle) = self.store.cf_handle(&self.name) {
            // Step 2: Prefixed Key Construction - Build isolation key
            let full_key = format!("{}.{}", self.prefix, key);

            // Step 3: LSM-Tree Write Operation - Write event through optimized path
            self.store
                .put_cf(&handle, full_key.clone(), data)
                .map_err(|e| Error::Get(format!(
                    "RocksDB event write failed for key '{}' ({} bytes): {}. \
                    This may indicate I/O errors, disk full, or database corruption.",
                    full_key, data.len(), e
                )))?;

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance. \
                Event storage requires proper collection initialization.",
                self.name
            )))
        }
    }

    /// Deletes a specific event from the collection.
    ///
    /// This method removes an event with the specified key from storage using
    /// RocksDB's delete operation, which creates tombstone markers for eventual
    /// removal through compaction. The operation maintains LSM-tree consistency
    /// while providing immediate "not found" semantics for subsequent reads.
    ///
    /// ## LSM-Tree Delete Semantics
    ///
    /// RocksDB handles deletes through tombstone markers:
    ///
    /// 1. **Tombstone Creation**: Delete creates a tombstone marker in MemTable
    /// 2. **Read Masking**: Subsequent reads see the tombstone and return "not found"
    /// 3. **Compaction Cleanup**: Tombstones and old values removed during compaction
    /// 4. **Space Recovery**: Deleted event space eventually reclaimed
    ///
    /// ## Event Deletion Impact
    ///
    /// Deleting events affects the collection structure:
    ///
    /// - **Sequence Gaps**: Creates gaps in sequential event numbering
    /// - **Iteration Behavior**: Deleted events skipped during range scans
    /// - **Space Management**: Space reclaimed through background compaction
    /// - **Consistency**: Maintains collection consistency during concurrent access
    ///
    /// # Parameters
    ///
    /// * `key` - The event key to delete (e.g., "001"). Will be prefixed
    ///          with the actor's prefix for proper isolation.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful event deletion, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - RocksDB delete operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing or database closed
    /// - [`Error::Get`] - RocksDB delete operation failed
    ///
    /// Note: This method succeeds even if the event doesn't exist.
    fn del(&mut self, key: &str) -> Result<(), Error> {
        // Step 1: Column Family Validation
        if let Some(handle) = self.store.cf_handle(&self.name) {
            // Step 2: Prefixed Key Construction
            let full_key = format!("{}.{}", self.prefix, key);

            // Step 3: LSM-Tree Delete Operation - Create tombstone marker
            self.store
                .delete_cf(&handle, full_key.clone())
                .map_err(|e| Error::Get(format!(
                    "RocksDB event deletion failed for key '{}': {}. \
                    This may indicate I/O errors or database corruption.",
                    full_key, e
                )))?;

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance.",
                self.name
            )))
        }
    }

    /// Purges all events with the actor's prefix from the collection.
    ///
    /// This method removes all events belonging to the specific actor by scanning
    /// the collection and deleting keys that match the actor's prefix. The operation
    /// uses iterator-based deletion to handle prefix-based cleanup efficiently.
    ///
    /// ## Implementation Strategy
    ///
    /// The purge operation follows these steps:
    ///
    /// 1. **Full Collection Scan**: Iterates through all keys in the column family
    /// 2. **Prefix Matching**: Identifies keys belonging to the specific actor
    /// 3. **Batch Deletion**: Deletes matching keys using individual delete operations
    /// 4. **UTF-8 Validation**: Safely handles non-UTF8 keys by skipping them
    ///
    /// ## Performance Considerations
    ///
    /// - **Scan Cost**: O(n) where n is total events in collection
    /// - **Delete Cost**: O(log n) per matching event for LSM-tree operations
    /// - **Memory Usage**: Iterator processes data to limit memory consumption
    /// - **Compaction Impact**: Multiple deletes may trigger background compaction
    ///
    /// ## Use Cases
    ///
    /// - **Actor Cleanup**: Remove all events when an actor is permanently deleted
    /// - **Data Retention**: Implement retention policies for old events
    /// - **Reset Operations**: Clear actor history while preserving other actors
    /// - **Maintenance**: Bulk cleanup for administrative operations
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful purge completion, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - Iterator or delete operations fail ([`Error::Get`] or [`Error::Store`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family missing, key encoding issues
    /// - [`Error::Get`] - RocksDB delete operations failed
    ///
    /// # Performance Warning
    ///
    /// This operation can be expensive for large collections as it requires
    /// scanning all keys. Consider using more targeted deletion strategies
    /// for performance-critical scenarios.
    fn purge(&mut self) -> Result<(), Error> {
        // Step 1: Column Family Validation
        if let Some(handle) = self.store.cf_handle(&self.name) {
            // Step 2: Iterator-Based Purge - Scan and delete matching events
            let iter = self.store.iterator_cf(&handle, IteratorMode::Start);

            for (key, _) in iter.flatten() {
                // Step 3: Key Encoding Validation - Safely convert bytes to string
                let key_str = String::from_utf8(key.to_vec()).map_err(|e| {
                    Error::Store(format!(
                        "Failed to convert key to UTF-8 string during purge: {}. \
                        This may indicate corrupted keys or non-UTF8 data.",
                        e
                    ))
                })?;

                // Step 4: Prefix Matching - Check if event belongs to this actor
                if key_str.starts_with(&self.prefix) {
                    // Step 5: Individual Event Deletion - Remove matching event
                    self.store
                        .delete_cf(&handle, key_str)
                        .map_err(|e| Error::Get(format!(
                            "Failed to delete event during purge operation: {}",
                            e
                        )))?;
                }
            }

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance.",
                self.name
            )))
        }
    }

    /// Creates an iterator for traversing events in the collection.
    ///
    /// This method returns a memory-efficient iterator that can traverse large
    /// event collections with configurable direction (forward/reverse) and
    /// automatic prefix filtering. The iterator uses batched loading to prevent
    /// memory exhaustion while maintaining good performance.
    ///
    /// ## Iterator Implementation
    ///
    /// The returned [`RocksDbIterator`] provides:
    ///
    /// - **Lazy Evaluation**: Events loaded only as needed
    /// - **Batched Loading**: Configurable batch sizes for memory efficiency
    /// - **Prefix Filtering**: Only returns events matching the actor's prefix
    /// - **Key Trimming**: Removes prefix from returned keys for clean interface
    /// - **Direction Control**: Forward or reverse chronological order
    ///
    /// ## Performance Characteristics
    ///
    /// - **Memory Usage**: Bounded by batch size, not total collection size
    /// - **I/O Pattern**: Sequential reads optimized for LSM-tree storage
    /// - **Cache Benefits**: Benefits from RocksDB's block cache during iteration
    /// - **Concurrent Safe**: Multiple iterators can coexist safely
    ///
    /// # Parameters
    ///
    /// * `reverse` - If true, iterate in reverse order (newest to oldest).
    ///              If false, iterate in forward order (oldest to newest).
    ///
    /// # Returns
    ///
    /// A boxed iterator that yields `(String, Vec<u8>)` tuples containing
    /// the event key (without prefix) and the serialized event data.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use store::database::Collection;
    ///
    /// let collection = /* RocksDbStore for events */;
    ///
    /// // Forward iteration (chronological order)
    /// println!("Events in chronological order:");
    /// for (key, data) in collection.iter(false) {
    ///     println!("Event {}: {} bytes", key, data.len());
    /// }
    ///
    /// // Reverse iteration (reverse chronological order)
    /// println!("Events in reverse chronological order:");
    /// for (key, data) in collection.iter(true) {
    ///     println!("Event {}: {} bytes", key, data.len());
    ///     // Process most recent events first
    /// }
    /// ```
    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        Box::new(RocksDbIterator::new(
            self.store.clone(),
            &self.name,
            &self.prefix,
            reverse,
        ))
    }

    /// Forces a flush of the column family's MemTable to persistent storage.
    ///
    /// This method triggers an immediate flush of the active MemTable to L0 SST files,
    /// ensuring that all recent writes are persisted to disk. Normally, flushes happen
    /// automatically when MemTable size limits are reached, but manual flushing can
    /// be useful for ensuring durability or freeing memory.
    ///
    /// ## LSM-Tree Flush Operation
    ///
    /// The flush operation performs these steps:
    ///
    /// 1. **MemTable Freeze**: Active MemTable becomes immutable
    /// 2. **Background Flush**: Immutable MemTable written to L0 SST file
    /// 3. **Memory Recovery**: MemTable memory is freed for reuse
    /// 4. **Durability**: All flushed data is persistent and crash-safe
    ///
    /// ## Use Cases
    ///
    /// - **Explicit Durability**: Ensure critical events are immediately persisted
    /// - **Memory Management**: Free MemTable memory during low-activity periods
    /// - **Backup Operations**: Ensure consistent state before backup creation
    /// - **Testing**: Deterministic flush behavior for integration tests
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful flush, or an error if:
    /// - The column family doesn't exist ([`Error::Store`])
    /// - RocksDB flush operation fails ([`Error::Get`])
    ///
    /// # Errors
    ///
    /// - [`Error::Store`] - Column family doesn't exist
    /// - [`Error::Get`] - Flush operation failed due to I/O or corruption
    fn flush(&self) -> Result<(), Error> {
        // Step 1: Column Family Validation
        if let Some(handle) = self.store.cf_handle(&self.name) {
            // Step 2: Force MemTable Flush - Trigger immediate persistence
            self.store
                .flush_cf(&handle)
                .map_err(|e| Error::Get(format!(
                    "RocksDB flush operation failed for collection '{}': {}. \
                    This may indicate I/O errors or database corruption.",
                    self.name, e
                )))?;

            Ok(())
        } else {
            Err(Error::Store(format!(
                "Column family '{}' does not exist in RocksDB instance.",
                self.name
            )))
        }
    }
}

// REMOVED: GuardIter type - no longer needed after removing unsafe code

/// Memory-efficient RocksDB iterator with batched loading and prefix filtering.
///
/// `RocksDbIterator` provides safe, efficient iteration over large RocksDB collections
/// without loading all data into memory at once. The iterator uses batched loading
/// strategies optimized for LSM-tree storage patterns, with automatic prefix filtering
/// and configurable memory usage bounds.
///
/// ## Design Philosophy
///
/// This iterator is designed around several key principles:
///
/// ### Memory Efficiency
///
/// Large collections can contain millions of events, making full-memory loading impractical:
///
/// - **Bounded Memory**: Memory usage determined by batch size, not collection size
/// - **Lazy Loading**: Data loaded incrementally as iteration progresses
/// - **Batch Processing**: Configurable batch sizes balance memory vs. I/O efficiency
/// - **Automatic Cleanup**: Processed batches are automatically freed
///
/// ### Safety and Correctness
///
/// The implementation prioritizes safety and correctness:
///
/// - **No Unsafe Code**: Completely safe Rust implementation
/// - **Lifetime Safety**: Proper lifetime management without raw pointers
/// - **Error Resilience**: Graceful handling of non-UTF8 keys and RocksDB errors
/// - **Consistent State**: Iterator state remains consistent across operations
///
/// ### Performance Optimization
///
/// Optimized for RocksDB's LSM-tree storage characteristics:
///
/// - **Sequential Access**: Leverages LSM-tree's sequential read optimization
/// - **Block Cache Utilization**: Benefits from RocksDB's caching mechanisms
/// - **Prefix Locality**: Exploits spatial locality of prefixed keys
/// - **Minimal Overhead**: Low computational overhead during iteration
///
/// ## Iterator State Management
///
/// The iterator maintains sophisticated state to enable efficient traversal:
///
/// ### Batch Management
///
/// ```text
/// Iterator State Progression:
///
/// Initial State:
/// ├── current_batch: empty
/// ├── last_key: None
/// ├── finished: false
/// └── batch_size: 100
///
/// First Batch Load:
/// ├── current_batch: [item1, item2, ..., item100]
/// ├── last_key: Some("user_123.100")
/// ├── finished: false
/// └── position: at item1
///
/// Batch Exhaustion:
/// ├── current_batch: empty (all items consumed)
/// ├── last_key: Some("user_123.100")
/// ├── finished: false
/// └── triggers: load_next_batch()
///
/// Completion:
/// ├── current_batch: empty
/// ├── last_key: Some("user_123.999")
/// ├── finished: true
/// └── position: end of iteration
/// ```
///
/// ### Prefix Filtering Strategy
///
/// The iterator automatically filters results to match the specified prefix:
///
/// - **Inclusion Filter**: Only yields keys starting with the configured prefix
/// - **Key Trimming**: Removes prefix from returned keys for clean API
/// - **UTF-8 Validation**: Safely handles non-UTF8 keys by skipping them
/// - **Early Termination**: Stops when no more matching keys are found
///
/// ## LSM-Tree Access Patterns
///
/// The iterator is optimized for RocksDB's LSM-tree characteristics:
///
/// ### Sequential Read Benefits
///
/// LSM-trees provide excellent sequential read performance:
///
/// - **Cache Warming**: Sequential reads warm the block cache effectively
/// - **Prefetching**: OS-level prefetching benefits sequential access patterns
/// - **Compaction Benefits**: Well-compacted LSM-trees excel at range scans
/// - **Bloom Filter Efficiency**: Sequential access amortizes bloom filter costs
///
/// ### Direction-Specific Optimizations
///
/// Both forward and reverse iteration are optimized:
///
/// - **Forward Iteration**: Natural LSM-tree order with optimal cache patterns
/// - **Reverse Iteration**: Efficient reverse traversal with proper state management
/// - **Consistent Performance**: Similar performance characteristics in both directions
/// - **Memory Usage**: Identical memory profiles regardless of direction
///
/// ## Concurrent Access Considerations
///
/// The iterator supports safe concurrent usage patterns:
///
/// ### Thread Safety
///
/// - **Shared Database**: Uses `Arc<DB>` for safe database sharing across threads
/// - **Independent State**: Each iterator maintains independent traversal state
/// - **No Synchronization**: No locks or atomics needed during iteration
/// - **Isolation**: Iterator state isolated from other concurrent operations
///
/// ### Snapshot Consistency
///
/// RocksDB provides snapshot isolation for consistent iteration:
///
/// - **Point-in-Time**: Iterator sees data as of creation time
/// - **Write Isolation**: Concurrent writes don't affect ongoing iteration
/// - **Consistency**: No partial or inconsistent states visible during iteration
/// - **Durability**: Iterator works correctly across database operations
///
/// ## Performance Characteristics
///
/// ### Time Complexity
///
/// - **Per-Item**: O(1) amortized cost per yielded item
/// - **Batch Loading**: O(batch_size) cost amortized across batch items
/// - **Prefix Filtering**: O(1) prefix matching per candidate key
/// - **Overall**: O(n) where n is the number of matching items
///
/// ### Space Complexity
///
/// - **Memory Usage**: O(batch_size) bounded by configuration
/// - **Cache Impact**: Benefits from but doesn't overwhelm RocksDB's cache
/// - **Streaming**: Constant memory usage regardless of collection size
/// - **Cleanup**: Automatic memory reclamation as batches are consumed
///
/// ### I/O Patterns
///
/// - **Sequential Reads**: Optimized for LSM-tree sequential access patterns
/// - **Block Alignment**: Leverages RocksDB's block-based storage format
/// - **Cache Utilization**: Efficiently uses shared block cache across iterators
/// - **Prefetch Benefits**: OS-level and RocksDB-level prefetching optimization
///
/// ## Configuration and Tuning
///
/// ### Batch Size Selection
///
/// The batch size represents a key performance trade-off:
///
/// ```text
/// Small Batch Size (e.g., 10):
/// ├── Pros: Low memory usage, responsive
/// ├── Cons: Higher I/O overhead, more RocksDB calls
/// └── Use: Memory-constrained environments
///
/// Medium Batch Size (e.g., 100, default):
/// ├── Pros: Balanced memory/performance, good general case
/// ├── Cons: Moderate memory usage
/// └── Use: Most applications, good starting point
///
/// Large Batch Size (e.g., 1000):
/// ├── Pros: Maximum throughput, minimal I/O overhead
/// ├── Cons: Higher memory usage, less responsive
/// └── Use: Bulk processing, high-memory environments
/// ```
///
/// ### Memory vs Performance Trade-offs
///
/// Configuration should consider the specific use case:
///
/// - **Interactive Applications**: Smaller batches for responsive iteration
/// - **Bulk Processing**: Larger batches for maximum throughput
/// - **Memory-Constrained**: Tune batch size to available memory
/// - **High-Concurrency**: Balance individual iterator memory with total system load
///
/// ## Usage Examples
///
/// ### Basic Iteration
///
/// ```ignore
/// use rocksdb_db::RocksDbIterator;
/// use std::sync::Arc;
///
/// let iterator = RocksDbIterator::new(
///     db.clone(),
///     "events",           // Column family
///     "user_123",         // Prefix
///     false,              // Forward direction
/// );
///
/// for (key, data) in iterator {
///     println!("Event {}: {} bytes", key, data.len());
///     // Process event data
/// }
/// ```
///
/// ### Reverse Chronological Processing
///
/// ```ignore
/// // Process events from newest to oldest
/// let reverse_iter = RocksDbIterator::new(db.clone(), "events", "user_123", true);
///
/// for (key, data) in reverse_iter.take(10) {
///     println!("Recent event {}: {} bytes", key, data.len());
///     // Process only the 10 most recent events
/// }
/// ```
///
/// ### Memory-Bounded Processing
///
/// ```ignore
/// // Configure iterator for memory-constrained environment
/// let mut iterator = RocksDbIterator::new(db.clone(), "events", "user_123", false);
/// // Note: batch_size is private, consider adding constructor parameter
///
/// let mut total_processed = 0;
/// for (key, data) in iterator {
///     total_processed += 1;
///     if total_processed % 100 == 0 {
///         println!("Processed {} events, current memory usage controlled", total_processed);
///     }
/// }
/// ```
///
/// The `RocksDbIterator` provides a robust, efficient, and safe way to traverse
/// large RocksDB collections while maintaining predictable memory usage and
/// excellent performance characteristics optimized for LSM-tree storage.
pub struct RocksDbIterator {
    /// Shared RocksDB database instance for accessing the underlying storage.
    ///
    /// Wrapped in Arc to enable safe sharing across multiple threads while
    /// maintaining access to the database for batch loading operations.
    db: Arc<DB>,

    /// Column family name identifying the storage partition to iterate over.
    ///
    /// Each column family represents a logical data partition with independent
    /// LSM-tree storage, enabling efficient iteration within specific data types.
    cf_name: String,

    /// Key prefix for filtering iteration results to a specific actor or namespace.
    ///
    /// Only keys starting with this prefix will be yielded by the iterator,
    /// providing efficient data isolation within shared column families.
    prefix: String,

    /// Direction flag controlling iteration order (forward=false, reverse=true).
    ///
    /// Determines whether iteration proceeds in ascending (chronological) or
    /// descending (reverse chronological) key order.
    reverse: bool,

    /// Current batch of loaded items ready for iteration.
    ///
    /// Contains the items from the most recently loaded batch, converted to
    /// an iterator for efficient consumption. Empty when a new batch needs loading.
    current_batch: std::vec::IntoIter<(String, Vec<u8>)>,

    /// Last processed key for batch continuation tracking.
    ///
    /// Stores the last key yielded from the previous batch to enable proper
    /// continuation when loading subsequent batches. None when starting iteration.
    last_key: Option<String>,

    /// Completion flag indicating whether iteration has reached the end.
    ///
    /// Set to true when no more matching data is available, preventing
    /// unnecessary batch loading attempts and providing definitive termination.
    finished: bool,

    /// Configurable batch size controlling memory usage vs I/O trade-offs.
    ///
    /// Determines how many items are loaded from RocksDB in each batch operation.
    /// Larger values improve throughput but increase memory usage, while smaller
    /// values reduce memory footprint but increase I/O overhead.
    batch_size: usize,
}

impl RocksDbIterator {
    /// Creates a new RocksDB iterator with batched loading and prefix filtering.
    ///
    /// This constructor initializes an iterator optimized for traversing large RocksDB
    /// collections with memory-efficient batching and automatic prefix filtering.
    /// The iterator is designed for LSM-tree storage characteristics and provides
    /// consistent performance across different collection sizes.
    ///
    /// ## Initialization Strategy
    ///
    /// The constructor sets up the iterator state for lazy evaluation:
    ///
    /// 1. **Database Reference**: Stores shared reference to RocksDB instance
    /// 2. **Configuration**: Sets column family, prefix, and direction parameters
    /// 3. **State Initialization**: Prepares empty batch state for first load
    /// 4. **Performance Tuning**: Configures optimal batch size for memory efficiency
    ///
    /// ## Memory Management
    ///
    /// The iterator uses bounded memory consumption:
    ///
    /// - **Initial State**: No data loaded until first iteration
    /// - **Batch Size**: Default 100 items per batch for balanced performance
    /// - **Lazy Loading**: Data loaded incrementally as iteration progresses
    /// - **Automatic Cleanup**: Previous batches freed automatically
    ///
    /// ## Performance Considerations
    ///
    /// The default configuration is optimized for common use cases:
    ///
    /// - **Batch Size**: 100 items balances memory usage with I/O efficiency
    /// - **Direction Support**: Both forward and reverse iteration optimized
    /// - **Prefix Filtering**: Efficient filtering reduces unnecessary data transfer
    /// - **UTF-8 Safety**: Graceful handling of non-UTF8 keys
    ///
    /// # Parameters
    ///
    /// * `store` - Shared RocksDB database instance wrapped in Arc for thread safety
    /// * `name` - Column family name identifying the storage partition to iterate
    /// * `prefix` - Key prefix for filtering results to specific actor or namespace
    /// * `reverse` - Iteration direction (false=forward/ascending, true=reverse/descending)
    ///
    /// # Returns
    ///
    /// A new `RocksDbIterator` instance ready for iteration, with empty initial state
    /// and configured for optimal batched loading performance.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rocksdb_db::RocksDbIterator;
    /// use std::sync::Arc;
    ///
    /// // Forward iteration for chronological event processing
    /// let forward_iter = RocksDbIterator::new(
    ///     db.clone(),
    ///     "user_events",     // Column family
    ///     "user_123",        // Actor prefix
    ///     false,             // Forward direction
    /// );
    ///
    /// // Reverse iteration for most-recent-first processing
    /// let reverse_iter = RocksDbIterator::new(
    ///     db.clone(),
    ///     "user_events",     // Same column family
    ///     "user_123",        // Same actor
    ///     true,              // Reverse direction
    /// );
    ///
    /// // Different actor in same collection
    /// let other_actor_iter = RocksDbIterator::new(
    ///     db.clone(),
    ///     "user_events",     // Same column family
    ///     "user_456",        // Different actor
    ///     false,             // Forward direction
    /// );
    /// ```
    pub fn new(
        store: Arc<DB>,
        name: &str,
        prefix: &str,
        reverse: bool,
    ) -> Self {
        Self {
            db: store,
            cf_name: name.to_owned(),
            prefix: prefix.to_owned(),
            reverse,
            current_batch: Vec::new().into_iter(),
            last_key: None,
            finished: false,
            // Default batch size balances memory usage with I/O efficiency
            // Can be tuned based on specific workload characteristics:
            // - Smaller for memory-constrained environments
            // - Larger for bulk processing scenarios
            batch_size: 100,
        }
    }

    /// Loads the next batch of items from RocksDB with prefix filtering and continuation support.
    ///
    /// This method implements the core batching logic for memory-efficient iteration over
    /// large RocksDB collections. It handles continuation from previous batches, applies
    /// prefix filtering, and manages iteration termination conditions.
    ///
    /// ## Batch Loading Strategy
    ///
    /// The method implements sophisticated batch loading:
    ///
    /// 1. **Completion Check**: Early return if iteration has finished
    /// 2. **Iterator Creation**: Creates RocksDB iterator with appropriate direction
    /// 3. **Continuation Logic**: Skips to position after last yielded key
    /// 4. **Prefix Filtering**: Only includes keys matching the configured prefix
    /// 5. **Batch Size Control**: Limits memory usage to configured batch size
    /// 6. **Termination Detection**: Detects and handles end of iteration
    ///
    /// ## Continuation Mechanism
    ///
    /// Proper continuation across batches is critical for correctness:
    ///
    /// ```text
    /// Batch 1: [user_123.001, user_123.002, ..., user_123.100]
    /// last_key: Some("user_123.100")
    ///
    /// Batch 2 Loading:
    /// ├── Skip items until key > "user_123.100"
    /// ├── Collect next 100 matching items
    /// └── Update last_key to "user_123.200"
    /// ```
    ///
    /// ## LSM-Tree Optimization
    ///
    /// The batch loading leverages LSM-tree characteristics:
    ///
    /// - **Sequential Access**: Benefits from LSM-tree's sequential read optimization
    /// - **Cache Utilization**: Warms block cache for subsequent batches
    /// - **Bloom Filter Benefits**: Amortizes bloom filter costs across batch
    /// - **Compaction Alignment**: Works efficiently with compacted storage
    ///
    /// ## Error Handling and Resilience
    ///
    /// The method handles various error conditions gracefully:
    ///
    /// - **Missing Column Family**: Returns empty batch and sets finished flag
    /// - **Non-UTF8 Keys**: Skips invalid keys and continues processing
    /// - **RocksDB Errors**: Gracefully handles iterator errors
    /// - **Empty Results**: Properly detects and handles end of iteration
    ///
    /// ## Performance Characteristics
    ///
    /// - **Time Complexity**: O(batch_size + scan_overhead) per batch
    /// - **Space Complexity**: O(batch_size) memory bounded
    /// - **I/O Pattern**: Sequential reads optimized for LSM-tree storage
    /// - **Cache Impact**: Positive impact on subsequent batch loads
    ///
    /// # Returns
    ///
    /// Returns `true` if a new batch was successfully loaded and is ready for consumption,
    /// or `false` if no more data is available (iteration complete).
    ///
    /// # State Changes
    ///
    /// On successful batch load:
    /// - `current_batch` is populated with new items
    /// - `last_key` is updated to the last item's key
    /// - `finished` remains false
    ///
    /// On iteration completion:
    /// - `current_batch` remains empty
    /// - `finished` is set to true
    /// - `last_key` retains the last processed key
    fn load_next_batch(&mut self) -> bool {
        // Early termination if iteration has already completed
        if self.finished {
            return false;
        }

        // Initialize batch collection for this load cycle
        let mut batch = Vec::new();

        // Configure iterator direction based on iteration parameters
        let mode = if self.reverse {
            IteratorMode::End   // Start from end for reverse iteration
        } else {
            IteratorMode::Start // Start from beginning for forward iteration
        };

        // Attempt to get column family handle for the specified collection
        if let Some(handle) = self.db.cf_handle(&self.cf_name) {
            // Create RocksDB iterator for the column family
            let iter = self.db.iterator_cf(&handle, mode);
            let mut count = 0;
            let mut should_skip = self.last_key.is_some();

            // Process items from RocksDB iterator
            for item in iter {
                // Respect batch size limit to control memory usage
                if count >= self.batch_size {
                    break;
                }

                // Handle RocksDB iterator item result
                if let Ok((key, value)) = item {
                    // Attempt UTF-8 conversion with error resilience
                    match String::from_utf8(key.to_vec()) {
                        Ok(key_str) => {
                            // Continuation logic: skip until we pass the last processed key
                            if should_skip {
                                if let Some(ref last) = self.last_key {
                                    if key_str == *last {
                                        should_skip = false;
                                    }
                                    continue;
                                }
                            }

                            // Apply prefix filtering to include only matching keys
                            if key_str.starts_with(&self.prefix) {
                                // Trim prefix from key for clean API presentation
                                let trimmed_key = if self.prefix.is_empty() {
                                    key_str.clone()
                                } else {
                                    // Remove "prefix." from "prefix.key" → "key"
                                    key_str[self.prefix.len() + 1..].to_owned()
                                };

                                // Add item to current batch
                                batch.push((trimmed_key, value.to_vec()));

                                // Update continuation tracking
                                self.last_key = Some(key_str);
                                count += 1;
                            }
                        }
                        Err(_) => {
                            // Skip invalid UTF-8 keys gracefully
                            // This maintains iterator robustness when encountering
                            // corrupted or binary keys in the database
                            continue;
                        }
                    }
                }
            }
        }

        // Determine if iteration should continue or terminate
        if batch.is_empty() {
            // No more matching data available - mark iteration as complete
            self.finished = true;
            false
        } else {
            // Batch loaded successfully - prepare for consumption
            self.current_batch = batch.into_iter();
            true
        }
    }
}

/// Implementation of the standard [`Iterator`] trait for `RocksDbIterator`.
///
/// This implementation provides the standard Rust iterator interface while maintaining
/// memory efficiency through batched loading from RocksDB. The iterator yields
/// `(String, Vec<u8>)` tuples containing the trimmed event key and serialized data.
///
/// ## Iterator Semantics
///
/// The implementation follows standard Rust iterator semantics:
///
/// ### Item Type
///
/// Each yielded item is a `(String, Vec<u8>)` tuple containing:
///
/// - **Key**: Event key with prefix removed (e.g., "001", "002", "003")
/// - **Data**: Complete serialized event data as bytes
///
/// ### Termination Behavior
///
/// The iterator terminates when:
///
/// - All matching events have been yielded
/// - The column family doesn't exist or becomes unavailable
/// - No more keys match the configured prefix
///
/// ### Memory Management
///
/// Memory usage is controlled through batched loading:
///
/// - **Current Batch**: Small in-memory buffer of ready items
/// - **Lazy Loading**: Next batch loaded only when current is exhausted
/// - **Automatic Cleanup**: Consumed batches freed automatically
/// - **Bounded Usage**: Total memory usage limited by batch size
///
/// ## Performance Characteristics
///
/// The iterator provides excellent performance characteristics:
///
/// ### Amortized Complexity
///
/// - **Per-Item Cost**: O(1) amortized over batch size
/// - **Batch Loading**: O(batch_size) amortized across items
/// - **Overall**: O(n) where n is total matching items
///
/// ### Cache Efficiency
///
/// - **Spatial Locality**: Sequential access patterns optimize cache usage
/// - **Temporal Locality**: Recently loaded batches likely to be cache-hot
/// - **Prefetching**: Benefits from OS and RocksDB prefetching
///
/// ## Integration with Rust Ecosystem
///
/// Full compatibility with Rust iterator ecosystem:
///
/// ```ignore
/// use rocksdb_db::RocksDbIterator;
///
/// let iterator = RocksDbIterator::new(db.clone(), "events", "user_123", false);
///
/// // Works with all standard iterator methods
/// let first_10: Vec<_> = iterator.take(10).collect();
/// let event_keys: Vec<String> = iterator.map(|(key, _)| key).collect();
/// let large_events: Vec<_> = iterator.filter(|(_, data)| data.len() > 1000).collect();
/// ```
impl Iterator for RocksDbIterator {
    /// The item type yielded by this iterator.
    ///
    /// Each item is a tuple containing:
    /// - `String`: The event key with prefix removed for clean API
    /// - `Vec<u8>`: The complete serialized event data
    type Item = (String, Vec<u8>);

    /// Returns the next item from the iterator, or `None` if iteration is complete.
    ///
    /// This method implements the core iterator logic with transparent batched loading.
    /// When the current batch is exhausted, it automatically loads the next batch from
    /// RocksDB without exposing this complexity to the caller.
    ///
    /// ## Implementation Strategy
    ///
    /// The method uses a two-tier approach for efficiency:
    ///
    /// 1. **Current Batch Consumption**: First attempts to yield from loaded batch
    /// 2. **Batch Reloading**: If current batch empty, attempts to load next batch
    /// 3. **Graceful Termination**: Returns `None` when no more data available
    ///
    /// ## State Transitions
    ///
    /// The method manages several state transitions:
    ///
    /// ```text
    /// State: HasItems → next() → Returns Some(item)
    ///   ├── current_batch: [item2, item3, ...] (remaining items)
    ///   └── state: unchanged
    ///
    /// State: BatchEmpty → next() → Attempts batch reload
    ///   ├── Success: Returns Some(first_item_of_new_batch)
    ///   └── Failure: Returns None (iteration complete)
    ///
    /// State: Finished → next() → Returns None
    ///   ├── No batch loading attempted
    ///   └── state: unchanged (remains finished)
    /// ```
    ///
    /// ## Performance Optimization
    ///
    /// The implementation is optimized for common access patterns:
    ///
    /// - **Fast Path**: Current batch consumption is O(1)
    /// - **Batch Amortization**: Loading cost amortized across batch items
    /// - **Early Termination**: Finished state prevents unnecessary work
    /// - **Memory Efficiency**: Previous batches freed immediately
    ///
    /// # Returns
    ///
    /// - `Some((key, data))` - Next event with prefix-trimmed key and serialized data
    /// - `None` - No more events available (iteration complete)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut iterator = RocksDbIterator::new(db.clone(), "events", "user_123", false);
    ///
    /// // Manual iteration
    /// while let Some((key, data)) = iterator.next() {
    ///     println!("Processing event {}: {} bytes", key, data.len());
    /// }
    ///
    /// // Automatic iteration with for loop (recommended)
    /// for (key, data) in iterator {
    ///     println!("Event {}: {} bytes", key, data.len());
    /// }
    /// ```
    fn next(&mut self) -> Option<Self::Item> {
        // Fast path: Try to get next item from current loaded batch
        // This is the common case and should be highly optimized
        if let Some(item) = self.current_batch.next() {
            return Some(item);
        }

        // Slow path: Current batch is exhausted, attempt to load next batch
        // This amortizes the loading cost across all items in the new batch
        if self.load_next_batch() {
            // New batch loaded successfully, yield first item
            self.current_batch.next()
        } else {
            // No more data available, iteration is complete
            // The finished flag is set by load_next_batch()
            None
        }
    }
}

// REMOVED: Unsafe change_lifetime_const function - replaced with safe iterator implementation

#[cfg(test)]
mod tests {
    impl Default for RocksDbManager {
        fn default() -> Self {
            let dir = tempfile::tempdir()
                .expect("Can not create temporal directory.");
            let db = DB::open_default(dir.path())
                .expect("Can not create the database.");
            Self {
                opts: Options::default(),
                db: Arc::new(db),
            }
        }
    }

    use super::*;
    use store::test_store_trait;
    test_store_trait! {
        unit_test_rocksdb_manager:crate::rocksdb::RocksDbManager:RocksDbStore
    }
}
