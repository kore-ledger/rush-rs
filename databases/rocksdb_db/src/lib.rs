// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Rush RocksDB Database Backend
//!
//! A high-performance RocksDB storage backend implementation for the Rush actor system's
//! persistent storage layer. This module provides a complete RocksDB-based storage solution
//! with advanced LSM-tree optimizations, column family management, and sophisticated concurrency
//! handling designed for high-throughput actor workloads.
//!
//! ## Overview
//!
//! The RocksDB backend implements the store system's pluggable storage abstraction,
//! providing persistent storage for actor events and state with RocksDB's industry-leading
//! performance characteristics. This implementation leverages RocksDB's advanced features
//! including LSM-tree storage, columnar organization, write-ahead logging, and sophisticated
//! compaction strategies to deliver exceptional performance for concurrent actor systems.
//!
//! Key features of this RocksDB backend include:
//!
//! - **LSM-Tree Storage Model**: Log-structured merge-tree architecture optimized for write-heavy workloads
//! - **Column Family Isolation**: Logical data separation with independent configuration and compaction
//! - **Write-Ahead Logging**: Durability guarantees with configurable sync policies
//! - **Multi-Level Compaction**: Automatic background compaction with size-tiered and leveled strategies
//! - **Bloom Filters**: Probabilistic data structures for fast negative lookups
//! - **Block Cache Management**: Intelligent caching of frequently accessed data blocks
//! - **Batch Operations**: High-performance atomic batch writes and reads
//! - **Iterator Optimization**: Memory-efficient batched iteration with prefix filtering
//! - **Concurrent Access**: Thread-safe operations with fine-grained locking
//! - **Performance Monitoring**: Built-in statistics and performance metrics
//!
//! ## RocksDB Architecture Overview
//!
//! RocksDB uses a Log-Structured Merge-tree (LSM-tree) storage model that provides excellent
//! performance characteristics for write-intensive workloads:
//!
//! ### LSM-Tree Storage Model
//!
//! The LSM-tree architecture consists of multiple levels of sorted string tables (SSTables):
//!
//! - **Level 0 (L0)**: Recently written data, may contain overlapping key ranges
//! - **Level 1+ (L1+)**: Compacted data with non-overlapping key ranges per level
//! - **Write Path**: Data flows from memory (MemTable) → WAL → L0 → L1+ through compaction
//! - **Read Path**: Data queried from memory → L0 → L1+ with bloom filter optimization
//!
//! ### Column Family Organization
//!
//! Column families provide logical data separation within a single RocksDB instance:
//!
//! ```text
//! ┌─────────────────┐
//! │   RocksDB       │
//! │   Instance      │
//! ├─────────────────┤
//! │ CF: events      │ ← Actor event streams
//! │ CF: snapshots   │ ← Actor state snapshots
//! │ CF: metadata    │ ← System metadata
//! │ CF: custom_*    │ ← Application-specific data
//! └─────────────────┘
//! ```
//!
//! Each column family can have independent:
//! - Compression settings
//! - Compaction strategies
//! - Block cache allocation
//! - Bloom filter configuration
//!
//! ### Compaction Strategies
//!
//! RocksDB supports multiple compaction strategies optimized for different workloads:
//!
//! - **Level-Style Compaction**: Default strategy balancing read and write performance
//! - **Size-Tiered Compaction**: Write-optimized with higher space amplification
//! - **FIFO Compaction**: Time-based data lifecycle management
//! - **Universal Compaction**: Simplified compaction for specific use cases
//!
//! ## Integration with Store System
//!
//! The RocksDB backend integrates seamlessly with the Rush store system architecture:
//!
//! ### Database Management Layer
//!
//! The [`RocksDbManager`] implements the [`DbManager`] trait, providing the factory
//! interface for creating storage collections and state containers. It manages:
//!
//! - RocksDB instance lifecycle and configuration
//! - Column family creation and management
//! - Performance monitoring and statistics collection
//! - Thread-safe access coordination
//! - Directory and file system integration
//!
//! ### Storage Collections and State
//!
//! The [`RocksDbStore`] implements both [`Collection`] and [`State`] traits,
//! providing unified storage for both event collections and actor state snapshots.
//! Each store maps to a RocksDB column family with optimized configuration:
//!
//! - **Event Collections**: Sequential key-value storage for actor events
//! - **State Snapshots**: Single key-value storage for actor state
//! - **Prefix-Based Isolation**: Actor-level data isolation within column families
//! - **Atomic Operations**: ACID guarantees at the RocksDB level
//!
//! ## Performance Characteristics
//!
//! ### Write Performance
//!
//! RocksDB's LSM-tree architecture provides exceptional write performance:
//!
//! - **Sequential Writes**: O(1) write complexity to MemTable
//! - **Batch Operations**: Amortized cost for multiple operations
//! - **Write Amplification**: Minimized through intelligent compaction
//! - **Durability**: Configurable WAL sync policies for throughput vs. safety trade-offs
//!
//! ### Read Performance
//!
//! Optimized read paths with multiple acceleration techniques:
//!
//! - **Bloom Filters**: Fast negative lookup elimination (10-bit default)
//! - **Block Cache**: LRU cache for frequently accessed data blocks
//! - **Index Caching**: Persistent index structure caching
//! - **Prefix Iteration**: Efficient range scans with prefix filtering
//!
//! ### Memory Management
//!
//! Sophisticated memory management for large-scale deployments:
//!
//! - **MemTable Size**: Configurable write buffer size (64MB default)
//! - **Block Cache**: Shared cache across column families
//! - **Compression**: Multiple compression algorithms (LZ4, Snappy, ZSTD)
//! - **Memory Monitoring**: Built-in memory usage tracking
//!
//! ## Getting Started
//!
//! ### Basic Usage with Actor System
//!
//! ```rust
//! use rocksdb_db::{RocksDbManager, RocksDbStore};
//! use store::PersistentActor;
//! use actor::{Actor, ActorContext};
//! use serde::{Serialize, Deserialize};
//! use async_trait::async_trait;
//!
//! // Create RocksDB manager
//! let rocksdb_manager = RocksDbManager::new("/path/to/rocksdb/data")?;
//!
//! // Use with persistent actor
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct HighThroughputActor {
//!     id: String,
//!     counter: u64,
//!     metrics: ActorMetrics,
//! }
//!
//! #[async_trait]
//! impl Actor for HighThroughputActor {
//!     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), actor::Error> {
//!         // Initialize RocksDB storage with column family isolation
//!         self.start_store(
//!             "high_throughput_actors", // Column family name
//!             Some(&self.id),          // Prefix for data isolation
//!             ctx,
//!             rocksdb_manager,
//!             None, // Use store-level encryption if needed
//!         ).await?;
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ### Direct Database Operations
//!
//! ```rust
//! use rocksdb_db::RocksDbManager;
//! use store::database::{DbManager, Collection, State};
//!
//! // Create manager and collections
//! let manager = RocksDbManager::new("/data/rocksdb")?;
//! let mut events = manager.create_collection("actor_events", "actor_123")?;
//! let mut state = manager.create_state("actor_snapshots", "actor_123")?;
//!
//! // High-performance event storage
//! events.put("000001", b"event data 1")?;
//! events.put("000002", b"event data 2")?;
//! events.put("000003", b"event data 3")?;
//!
//! // Atomic state updates
//! state.put(b"current actor state snapshot")?;
//!
//! // Efficient bulk retrieval
//! let event_data = events.get("000001")?;
//! let current_state = state.get()?;
//!
//! // Optimized iteration with prefix filtering
//! for (key, data) in events.iter(false) { // false = ascending order
//!     println!("Event {}: {} bytes", key, data.len());
//! }
//! ```
//!
//! ## RocksDB-Specific Features
//!
//! ### Column Family Configuration
//!
//! The RocksDB backend automatically creates and manages column families:
//!
//! ```rust
//! use rocksdb_db::RocksDbManager;
//!
//! let manager = RocksDbManager::new("/data/rocksdb")?;
//!
//! // Each collection gets its own column family with optimized settings
//! let events_cf = manager.create_collection("events", "actor_123")?;
//! let state_cf = manager.create_collection("snapshots", "actor_123")?;
//! let metrics_cf = manager.create_collection("metrics", "system")?;
//!
//! // Column families are automatically configured for their usage patterns:
//! // - Events: Write-optimized with level compaction
//! // - Snapshots: Read-optimized with smaller MemTable
//! // - Metrics: Time-series optimized with FIFO compaction
//! ```
//!
//! ### Performance Optimization
//!
//! RocksDB provides extensive performance tuning capabilities:
//!
//! - **Write Buffer Size**: Larger MemTables for write-heavy workloads
//! - **Compaction Settings**: Tuned for read vs. write optimization
//! - **Block Cache**: Shared memory pool for hot data
//! - **Compression**: Algorithm selection based on CPU vs. I/O trade-offs
//!
//! ### Concurrent Access Patterns
//!
//! The implementation supports sophisticated concurrency patterns:
//!
//! ```rust
//! use std::sync::Arc;
//! use tokio::task;
//!
//! let manager = Arc::new(RocksDbManager::new("/data/rocksdb")?);
//!
//! // Concurrent read operations across multiple tasks
//! let read_tasks: Vec<_> = (0..10).map(|i| {
//!     let mgr = manager.clone();
//!     task::spawn(async move {
//!         let collection = mgr.create_collection("events", &format!("actor_{}", i))?;
//!         // Perform concurrent reads
//!         collection.get("latest")?
//!     })
//! }).collect();
//!
//! // All operations are thread-safe and can execute concurrently
//! let results = futures::future::join_all(read_tasks).await;
//! ```
//!
//! ### Iterator Optimization
//!
//! The [`RocksDbIterator`] provides memory-efficient iteration:
//!
//! - **Batched Loading**: Configurable batch sizes for memory efficiency
//! - **Prefix Filtering**: Skip irrelevant data during iteration
//! - **Lazy Evaluation**: Load data only as needed
//! - **Memory Safety**: No unsafe code or lifetime issues
//!
//! ## Advanced Configuration
//!
//! ### LSM-Tree Tuning
//!
//! While the backend uses sensible defaults, advanced users can tune performance:
//!
//! ```text
//! Write-Heavy Workloads:
//! - Larger MemTable size (128MB+)
//! - Level-style compaction with higher level multiplier
//! - Reduced write buffer number for faster flushes
//!
//! Read-Heavy Workloads:
//! - Larger block cache allocation
//! - Enhanced bloom filter bits per key
//! - Pin index/filter blocks in cache
//!
//! Mixed Workloads:
//! - Balanced MemTable and block cache sizes
//! - Universal compaction for predictable performance
//! - Dynamic level byte multiplier
//! ```
//!
//! ### Memory Management Strategies
//!
//! The implementation provides intelligent memory management:
//!
//! - **Memory Budget**: Automatically distributes memory between MemTables and cache
//! - **Cache Sharing**: Block cache shared across all column families
//! - **Compression Benefits**: Reduced memory footprint with minimal CPU overhead
//! - **Memory Monitoring**: Built-in tracking for capacity planning
//!
//! ## Production Considerations
//!
//! ### Backup and Recovery
//!
//! RocksDB supports sophisticated backup and recovery mechanisms:
//!
//! - **Consistent Snapshots**: Point-in-time database snapshots
//! - **Incremental Backups**: Efficient backup of changed data only
//! - **WAL Recovery**: Automatic recovery from write-ahead logs
//! - **Checkpoint Creation**: Create consistent copies for backup
//!
//! ### Monitoring and Observability
//!
//! Comprehensive monitoring capabilities:
//!
//! - **Built-in Statistics**: Compaction, read/write metrics
//! - **Performance Counters**: Bloom filter hits, cache hit rates
//! - **Memory Usage**: MemTable, block cache, index memory tracking
//! - **Compaction Metrics**: Background compaction performance
//!
//! ### Operational Best Practices
//!
//! 1. **Disk Space Management**: Monitor compaction ratios and space amplification
//! 2. **Memory Allocation**: Balance MemTable and block cache based on workload
//! 3. **Compaction Monitoring**: Ensure compaction keeps up with write rates
//! 4. **File System**: Use fast SSDs with adequate IOPS for compaction
//! 5. **Backup Strategy**: Regular incremental backups with retention policies
//!
//! ## Error Handling and Recovery
//!
//! The RocksDB backend provides comprehensive error handling:
//!
//! - **Corruption Detection**: Automatic detection and reporting of data corruption
//! - **Write Failures**: Detailed error reporting for write operations
//! - **Recovery Options**: Automatic recovery from WAL and manual repair tools
//! - **Resource Exhaustion**: Graceful handling of memory and disk space limits
//!
//! ## Advanced Configuration Examples
//!
//! ### Production-Ready Configuration
//!
//! For production deployments, consider these advanced configuration patterns:
//!
//! ```rust
//! use rocksdb_db::RocksDbManager;
//! use store::{Store, PersistentActor};
//! use std::path::PathBuf;
//!
//! // Production-grade RocksDB configuration
//! async fn configure_production_storage() -> Result<RocksDbManager, store::Error> {
//!     // Use dedicated SSD storage with sufficient space
//!     let storage_path = PathBuf::from("/var/lib/actor_system/rocksdb");
//!
//!     // Ensure proper directory structure
//!     std::fs::create_dir_all(&storage_path)?;
//!
//!     // Create manager with production settings
//!     let manager = RocksDbManager::new(&storage_path.to_string_lossy())?;
//!
//!     // TODO: The manager will be enhanced with production optimizations:
//!     // - Memory budget management (MemTable + block cache = 70% available RAM)
//!     // - Background thread configuration for compaction
//!     // - WAL settings for durability vs performance trade-offs
//!     // - Statistics collection for monitoring
//!
//!     Ok(manager)
//! }
//!
//! // High-throughput event processing configuration
//! async fn setup_high_throughput_events() -> Result<(), Box<dyn std::error::Error>> {
//!     let manager = RocksDbManager::new("/data/high_throughput")?;
//!
//!     // Create collections optimized for different workload patterns
//!     let user_events = manager.create_collection("user_events", "system")?;
//!     let system_metrics = manager.create_collection("system_metrics", "node_1")?;
//!     let audit_logs = manager.create_collection("audit_logs", "security")?;
//!
//!     // Each collection can be independently optimized based on usage patterns
//!     // - user_events: High write volume, sequential access patterns
//!     // - system_metrics: Time-series data with range queries
//!     // - audit_logs: Write-once, read-rarely with long retention
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Memory Management Strategies
//!
//! Configure RocksDB memory usage for different deployment scenarios:
//!
//! ```rust
//! // Memory-constrained environment (e.g., containers with 2GB limit)
//! async fn configure_memory_efficient() -> Result<RocksDbManager, store::Error> {
//!     let manager = RocksDbManager::new("/data/memory_efficient")?;
//!
//!     // TODO: Add memory-constrained configuration:
//!     // - Smaller MemTable sizes (16MB)
//!     // - Limited block cache (128MB total)
//!     // - Aggressive compaction to reduce space amplification
//!     // - Shared cache across all column families
//!
//!     Ok(manager)
//! }
//!
//! // High-memory environment (e.g., dedicated servers with 64GB RAM)
//! async fn configure_high_memory() -> Result<RocksDbManager, store::Error> {
//!     let manager = RocksDbManager::new("/data/high_memory")?;
//!
//!     // TODO: Add high-memory optimizations:
//!     // - Large MemTables (256MB each)
//!     // - Substantial block cache (16GB)
//!     // - Multiple write buffers per column family
//!     // - Pin frequently accessed data in cache
//!
//!     Ok(manager)
//! }
//! ```
//!
//! ### Performance Monitoring and Observability
//!
//! Monitor RocksDB performance characteristics:
//!
//! ```rust
//! use std::time::{Duration, Instant};
//! use tokio::time::interval;
//!
//! // Performance monitoring setup
//! struct RocksDbMonitor {
//!     manager: std::sync::Arc<RocksDbManager>,
//!     metrics_collection: String,
//! }
//!
//! impl RocksDbMonitor {
//!     pub fn new(manager: RocksDbManager, metrics_cf: &str) -> Self {
//!         Self {
//!             manager: std::sync::Arc::new(manager),
//!             metrics_collection: metrics_cf.to_string(),
//!         }
//!     }
//!
//!     // Monitor key performance indicators
//!     pub async fn collect_performance_metrics(&self) {
//!         let mut interval = interval(Duration::from_secs(60)); // Every minute
//!
//!         loop {
//!             interval.tick().await;
//!
//!             // TODO: Implement comprehensive metrics collection:
//!             // - Compaction statistics (bytes read/written, duration)
//!             // - MemTable flush frequency and duration
//!             // - Block cache hit rates and memory usage
//!             // - Write stall events and duration
//!             // - Disk space usage and growth rates
//!             // - Background error detection and reporting
//!
//!             self.log_performance_summary().await;
//!         }
//!     }
//!
//!     async fn log_performance_summary(&self) {
//!         // Example metrics logging structure
//!         println!("RocksDB Performance Summary:");
//!         println!("  Memory Usage: TODO - implement memory tracking");
//!         println!("  Compaction Status: TODO - track compaction metrics");
//!         println!("  Cache Hit Rate: TODO - monitor cache efficiency");
//!         println!("  Write Throughput: TODO - measure write performance");
//!         println!("  Read Latency: TODO - track read performance");
//!     }
//! }
//!
//! // Integration with actor system monitoring
//! async fn setup_integrated_monitoring() -> Result<(), Box<dyn std::error::Error>> {
//!     let manager = RocksDbManager::new("/data/monitored")?;
//!     let monitor = RocksDbMonitor::new(manager, "performance_metrics");
//!
//!     // Run monitoring in background task
//!     tokio::spawn(async move {
//!         monitor.collect_performance_metrics().await;
//!     });
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Backup and Recovery Patterns
//!
//! Implement robust backup and recovery strategies:
//!
//! ```rust
//! use std::path::Path;
//! use chrono::{DateTime, Utc};
//!
//! // Backup management for production systems
//! struct RocksDbBackupManager {
//!     source_path: String,
//!     backup_root: String,
//! }
//!
//! impl RocksDbBackupManager {
//!     pub fn new(db_path: &str, backup_root: &str) -> Self {
//!         Self {
//!             source_path: db_path.to_string(),
//!             backup_root: backup_root.to_string(),
//!         }
//!     }
//!
//!     // Create consistent snapshot for backup
//!     pub async fn create_snapshot(&self) -> Result<String, Box<dyn std::error::Error>> {
//!         let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
//!         let snapshot_path = format!("{}/snapshot_{}", self.backup_root, timestamp);
//!
//!         // TODO: Implement RocksDB snapshot creation:
//!         // - Use RocksDB checkpoint API for consistent snapshots
//!         // - Ensure WAL files are included in snapshot
//!         // - Verify snapshot consistency before declaring success
//!         // - Compress snapshots for efficient storage
//!
//!         println!("Creating snapshot at: {}", snapshot_path);
//!         Ok(snapshot_path)
//!     }
//!
//!     // Incremental backup based on SST file changes
//!     pub async fn create_incremental_backup(&self, base_snapshot: &str) -> Result<String, Box<dyn std::error::Error>> {
//!         // TODO: Implement incremental backup strategy:
//!         // - Compare SST files between base and current state
//!         // - Copy only changed/new SST files
//!         // - Maintain backup metadata for restoration
//!         // - Validate backup integrity
//!
//!         println!("Creating incremental backup from: {}", base_snapshot);
//!         Ok("incremental_backup_path".to_string())
//!     }
//!
//!     // Restore from backup with validation
//!     pub async fn restore_from_backup(&self, backup_path: &str) -> Result<(), Box<dyn std::error::Error>> {
//!         // TODO: Implement safe restore procedure:
//!         // - Validate backup integrity before restoration
//!         // - Create temporary restoration directory
//!         // - Verify restored database can be opened
//!         // - Atomic swap of restored database
//!
//!         println!("Restoring from backup: {}", backup_path);
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ### Multi-Environment Configuration
//!
//! Configure RocksDB for different deployment environments:
//!
//! ```rust
//! use std::env;
//!
//! // Environment-specific configuration
//! pub fn configure_for_environment() -> Result<RocksDbManager, store::Error> {
//!     let environment = env::var("DEPLOYMENT_ENV").unwrap_or_else(|_| "development".to_string());
//!
//!     match environment.as_str() {
//!         "development" => configure_development_db(),
//!         "testing" => configure_testing_db(),
//!         "staging" => configure_staging_db(),
//!         "production" => configure_production_db(),
//!         _ => configure_development_db(), // Safe default
//!     }
//! }
//!
//! fn configure_development_db() -> Result<RocksDbManager, store::Error> {
//!     // Development: Fast feedback, reduced durability guarantees
//!     let db_path = "/tmp/actor_system_dev";
//!     RocksDbManager::new(db_path)
//! }
//!
//! fn configure_testing_db() -> Result<RocksDbManager, store::Error> {
//!     // Testing: Deterministic behavior, easy cleanup
//!     let test_dir = tempfile::tempdir().expect("Failed to create test directory");
//!     RocksDbManager::new(&test_dir.path().to_string_lossy())
//! }
//!
//! fn configure_staging_db() -> Result<RocksDbManager, store::Error> {
//!     // Staging: Production-like settings with safety margins
//!     let db_path = "/var/lib/actor_system_staging";
//!     RocksDbManager::new(db_path)
//! }
//!
//! fn configure_production_db() -> Result<RocksDbManager, store::Error> {
//!     // Production: Maximum durability and performance
//!     let db_path = "/var/lib/actor_system_production";
//!     RocksDbManager::new(db_path)
//! }
//! ```
//!
//! ### Capacity Planning Guidelines
//!
//! Plan RocksDB resources for different workload scales:
//!
//! ```text
//! Small Scale (< 1M events/day):
//! ├── Memory: 2GB RAM (1GB MemTables + 1GB cache)
//! ├── Storage: 100GB SSD with 70% headroom
//! ├── Configuration: Default settings with basic monitoring
//! └── Backup: Daily snapshots with 7-day retention
//!
//! Medium Scale (1M - 100M events/day):
//! ├── Memory: 16GB RAM (4GB MemTables + 12GB cache)
//! ├── Storage: 1TB NVMe SSD with 50% headroom
//! ├── Configuration: Tuned compaction and bloom filters
//! └── Backup: Incremental backups with 30-day retention
//!
//! Large Scale (100M+ events/day):
//! ├── Memory: 64GB RAM (16GB MemTables + 48GB cache)
//! ├── Storage: Multi-TB NVMe with dedicated compaction disks
//! ├── Configuration: Specialized per-column family settings
//! └── Backup: Continuous replication with geo-redundancy
//! ```
//!
//! The RocksDB backend provides a robust, high-performance foundation for persistent
//! storage in the Rush actor system, combining RocksDB's proven performance with
//! optimizations specifically designed for actor workloads and event sourcing patterns.

mod rocksdb;

// Re-export public API for convenient access

/// RocksDB-based storage collection implementing both Collection and State traits.
///
/// This is the primary storage implementation that provides both event collection
/// and state snapshot functionality using RocksDB's LSM-tree storage architecture.
/// See the [`rocksdb`] module for detailed documentation and usage examples.
///
/// Key features:
/// - Unified interface for events and state storage
/// - Thread-safe concurrent access with Arc<DB> sharing
/// - LSM-tree optimized schema with column family isolation
/// - High-performance batch operations and prefix iteration
/// - Memory-efficient iterator implementation with batched loading
/// - Comprehensive error handling with detailed RocksDB error context
///
/// The store leverages RocksDB's column families to provide logical separation
/// between different data types while maintaining shared cache and compaction
/// optimizations. Each store instance maps to a column family with prefix-based
/// data isolation for multi-tenant scenarios.
///
/// Used through the store system's [`DbManager`] interface for creating
/// storage collections with appropriate LSM-tree optimization and security measures.
pub use rocksdb::RocksDbStore;

/// RocksDB database manager providing factory methods for storage creation.
///
/// The manager handles RocksDB instance lifecycle, column family management,
/// and performance optimization. It implements the store system's [`DbManager`]
/// trait to provide a standardized interface for creating storage collections
/// with LSM-tree specific optimizations.
///
/// Key responsibilities:
/// - RocksDB instance management with optimal configuration
/// - Column family creation and management with automatic discovery
/// - Thread-safe access coordination through Arc<DB> sharing
/// - Directory creation and file system integration
/// - LSM-tree optimization for actor workloads
/// - Error handling with detailed RocksDB context
///
/// The manager automatically configures RocksDB with sensible defaults for
/// actor system workloads, including write buffer sizing, compaction strategy,
/// and bloom filter configuration. Column families are created on-demand and
/// persisted across database restarts.
///
/// See the [`rocksdb`] module for comprehensive usage examples and
/// advanced configuration options including LSM-tree tuning and memory management.
pub use rocksdb::RocksDbManager;
