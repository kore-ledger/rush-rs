// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Store System Error Types
//!
//! This module provides comprehensive error handling for the storage system, defining all possible
//! error conditions that can occur during storage operations, data serialization, encryption,
//! compression, and database interactions. The error types are designed to provide detailed
//! context for debugging while enabling appropriate error handling strategies throughout the system.
//!
//! ## Error Categories
//!
//! The store system categorizes errors into several logical groups:
//!
//! ### Storage Infrastructure Errors
//! - **CreateStore**: Storage backend initialization failures
//! - **Store**: General storage operation failures including writes, transactions, and I/O
//!
//! ### Data Access Errors
//! - **Get**: Data retrieval failures from storage backends
//! - **EntryNotFound**: Missing data entries (normal condition, not necessarily an error)
//!
//! ## Error Handling Philosophy
//!
//! The store system follows a philosophy of explicit error handling with rich context:
//!
//! 1. **Fail Fast**: Detect and report errors as early as possible
//! 2. **Rich Context**: Provide detailed error messages for debugging
//! 3. **Recovery Guidance**: Include information about potential recovery strategies
//! 4. **Categorization**: Group errors by type to enable appropriate handling
//! 5. **Serialization**: All errors are serializable for distributed system support
//!
//! ## Integration with Actor System
//!
//! Store errors are designed to integrate seamlessly with the actor system's error handling:
//!
//! ```rust
//! use rush_store::Error as StoreError;
//! use actor::Error as ActorError;
//!
//! // Store errors can be converted to actor errors for propagation
//! let store_result: Result<(), StoreError> = storage.get("key");
//! let actor_result: Result<(), ActorError> = store_result
//!     .map_err(|e| ActorError::Store(e.to_string()));
//! ```
//!
//! ## Error Recovery Patterns
//!
//! Different error types suggest different recovery strategies:
//!
//! ### Transient Errors (Retryable)
//! - Network timeouts in distributed storage
//! - Temporary resource exhaustion
//! - Lock contention in concurrent access
//!
//! ### Permanent Errors (Non-Retryable)
//! - Invalid data format or corruption
//! - Authentication/authorization failures
//! - Configuration errors
//!
//! ### Missing Data (Expected Condition)
//! - `EntryNotFound` often represents normal application flow
//! - Should be handled as a valid business case, not an error
//!

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Comprehensive error handling for the storage system.
///
/// This enum defines all possible error conditions that can occur within the storage system,
/// providing detailed context for debugging and enabling appropriate error handling strategies.
/// All errors are serializable to support distributed system architectures and remote
/// error propagation.
///
/// # Error Categories
///
/// ## Infrastructure Errors
/// - **CreateStore**: Storage backend creation and initialization failures
/// - **Store**: General storage operation failures including I/O and transactions
///
/// ## Data Access Errors
/// - **Get**: Data retrieval failures from storage backends
/// - **EntryNotFound**: Missing data entries (often a normal condition)
///
/// # Thread Safety
///
/// This error type implements `Clone` and is safe to share across threads, enabling
/// error propagation throughout the distributed storage system.
///
/// # Examples
///
/// ```rust
/// use rush_store::Error;
///
/// // Handle storage creation failure with fallback
/// match create_primary_storage(&config).await {
///     Err(Error::CreateStore(reason)) => {
///         log::warn!("Primary storage failed: {}, trying backup", reason);
///         create_backup_storage(&config).await?
///     }
///     Ok(storage) => storage,
///     Err(e) => return Err(e),
/// };
///
/// // Handle missing data as normal application flow
/// match storage.get(&key).await {
///     Ok(data) => process_existing_data(data),
///     Err(Error::EntryNotFound(_)) => {
///         log::info!("Creating default entry for key: {}", key);
///         create_default_entry(&key).await?
///     }
///     Err(Error::Get(reason)) => {
///         log::error!("Storage retrieval failed: {}, retrying", reason);
///         retry_with_backoff(|| storage.get(&key)).await?
///     }
///     Err(e) => return Err(e),
/// };
/// ```
#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq)]
pub enum Error {
    /// Storage backend creation or initialization failure.
    ///
    /// This error occurs when the storage system fails to create or initialize a storage
    /// backend. Common causes include database connection failures, insufficient permissions,
    /// storage capacity issues, configuration errors, or incompatible storage versions.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the storage creation failure cause
    ///
    /// # Recovery Strategies
    ///
    /// - Validate storage configuration parameters and credentials
    /// - Implement storage backend fallbacks (memory, file, database alternatives)
    /// - Check storage system health, connectivity, and resource availability
    /// - Use connection pooling and retry logic for network-based storage
    /// - Consider degraded mode operation with read-only or in-memory storage
    /// - Verify storage version compatibility and perform migrations if needed
    ///
    /// # Common Causes
    ///
    /// - Database server unavailable or network connectivity issues
    /// - Insufficient disk space or memory for storage initialization
    /// - Invalid connection strings, credentials, or configuration parameters
    /// - Permission issues accessing storage directories or database resources
    /// - Version incompatibilities between storage schema and application
    /// - Resource exhaustion (connection limits, file handles, memory)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_store::Error;
    ///
    /// // Implement storage fallback strategy
    /// let storage = match create_database_storage(&config).await {
    ///     Ok(store) => store,
    ///     Err(Error::CreateStore(reason)) => {
    ///         log::warn!("Database storage failed: {}, falling back to memory", reason);
    ///         match create_memory_storage().await {
    ///             Ok(memory_store) => {
    ///                 log::info!("Using in-memory storage as fallback");
    ///                 memory_store
    ///             }
    ///             Err(e) => {
    ///                 log::error!("All storage backends failed");
    ///                 return Err(e);
    ///             }
    ///         }
    ///     }
    ///     Err(e) => return Err(e),
    /// };
    /// ```
    ///
    /// # Monitoring and Alerting
    ///
    /// This error should trigger immediate alerts as it indicates critical infrastructure
    /// problems that prevent the application from functioning properly. Monitor for:
    ///
    /// - Frequency of storage creation failures
    /// - Correlation with infrastructure changes or deployments
    /// - Resource utilization trends that might indicate capacity issues
    /// - Network connectivity patterns in distributed deployments
    #[error("Can't create store: {0}")]
    CreateStore(String),

    /// Data retrieval failure from storage backend.
    ///
    /// This error occurs when the storage system fails to retrieve data from the underlying
    /// storage backend. Common causes include network connectivity issues, storage system
    /// failures, data corruption, serialization errors, encryption/decryption failures,
    /// or transient resource contention.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the data retrieval failure cause
    ///
    /// # Recovery Strategies
    ///
    /// - Implement cache layers to reduce storage dependencies and improve resilience
    /// - Use retry logic with exponential backoff for transient failures
    /// - Provide default values or fallback data for non-critical information
    /// - Monitor storage system health and automatically switch to backup systems
    /// - Implement data validation and corruption detection with repair mechanisms
    /// - Use circuit breaker patterns to prevent cascading failures
    /// - Consider read replicas for high-availability scenarios
    ///
    /// # Common Causes
    ///
    /// - Network timeouts or connectivity issues in distributed storage systems
    /// - Storage backend failures (disk errors, database crashes, service unavailability)
    /// - Data corruption or invalid data formats preventing deserialization
    /// - Encryption/decryption failures due to key issues or data tampering
    /// - Resource contention (locks, connection limits, memory pressure)
    /// - Serialization format changes or version incompatibilities
    /// - Storage system overload or performance degradation
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_store::Error;
    /// use std::time::Duration;
    ///
    /// // Implement retry with exponential backoff
    /// async fn get_with_retry(storage: &Storage, key: &str) -> Result<Vec<u8>, Error> {
    ///     let mut attempt = 0;
    ///     let max_attempts = 3;
    ///
    ///     loop {
    ///         match storage.get(key).await {
    ///             Ok(data) => return Ok(data),
    ///             Err(Error::Get(reason)) if attempt < max_attempts => {
    ///                 let delay = Duration::from_millis(100 * (2_u64.pow(attempt)));
    ///                 log::warn!("Get attempt {} failed: {}, retrying in {:?}",
    ///                           attempt + 1, reason, delay);
    ///                 tokio::time::sleep(delay).await;
    ///                 attempt += 1;
    ///             }
    ///             Err(Error::Get(reason)) => {
    ///                 log::error!("Get failed after {} attempts: {}", max_attempts, reason);
    ///                 return Err(Error::Get(reason));
    ///             }
    ///             Err(e) => return Err(e),
    ///         }
    ///     }
    /// }
    ///
    /// // Implement fallback with cached data
    /// match storage.get(&key).await {
    ///     Ok(data) => {
    ///         cache.put(&key, &data).await; // Update cache
    ///         Ok(data)
    ///     }
    ///     Err(Error::Get(reason)) => {
    ///         log::warn!("Storage get failed: {}, trying cache", reason);
    ///         match cache.get(&key).await {
    ///             Ok(cached_data) => {
    ///                 log::info!("Serving stale data from cache");
    ///                 Ok(cached_data)
    ///             }
    ///             Err(_) => {
    ///                 log::error!("Both storage and cache failed");
    ///                 Err(Error::Get(reason))
    ///             }
    ///         }
    ///     }
    ///     Err(e) => Err(e),
    /// }
    /// ```
    ///
    /// # Performance Considerations
    ///
    /// Get errors can indicate performance issues that should be monitored:
    ///
    /// - High frequency may indicate storage system overload
    /// - Correlation with specific keys might indicate hot spot issues
    /// - Network-related failures suggest infrastructure problems
    /// - Pattern analysis can help identify systemic vs transient issues
    #[error("Get error: {0}")]
    Get(String),

    /// Requested data entry not found in storage.
    ///
    /// This error occurs when attempting to retrieve data that doesn't exist in the storage
    /// system. Unlike `Get` errors, this specifically indicates that the storage operation
    /// succeeded but the requested entry is not present, which is often a normal condition
    /// in application logic rather than an error requiring recovery.
    ///
    /// # Context
    ///
    /// * Contains the identifier or description of the missing entry
    ///
    /// # Usage Patterns
    ///
    /// This error is frequently used in normal application flow and should often be
    /// handled as a valid business case rather than an exceptional condition:
    ///
    /// - First-time access to new entities that don't exist yet
    /// - Optional data that may or may not be present
    /// - Cache misses in caching scenarios
    /// - Lazy initialization patterns where data is created on first access
    ///
    /// # Recovery Strategies
    ///
    /// - Check if the missing entry represents expected application behavior
    /// - Create default entries for missing required data with appropriate defaults
    /// - Implement lazy loading patterns for optional data that's created on demand
    /// - Use optional types (`Option<T>`) to handle missing data gracefully
    /// - Cache frequently accessed entries to reduce lookup failures
    /// - Implement data migration or initialization routines for missing system data
    ///
    /// # Common Scenarios
    ///
    /// - User profiles that haven't been created yet
    /// - Configuration entries that use system defaults when not present
    /// - Cache entries that have expired or were never cached
    /// - Optional metadata or extended properties
    /// - Historical data that predates certain features
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_store::Error;
    ///
    /// // Handle missing user profile by creating default
    /// async fn get_user_profile(storage: &Storage, user_id: &str) -> Result<UserProfile, Error> {
    ///     match storage.get_user_profile(user_id).await {
    ///         Ok(profile) => Ok(profile),
    ///         Err(Error::EntryNotFound(_)) => {
    ///             log::info!("User {} profile not found, creating default", user_id);
    ///             let default_profile = UserProfile::default_for_user(user_id);
    ///             storage.save_user_profile(user_id, &default_profile).await?;
    ///             Ok(default_profile)
    ///         }
    ///         Err(e) => Err(e),
    ///     }
    /// }
    ///
    /// // Handle optional configuration with fallback
    /// let config_value = match storage.get_config("advanced_feature_x").await {
    ///     Ok(value) => value,
    ///     Err(Error::EntryNotFound(_)) => {
    ///         log::debug!("Advanced feature config not set, using default");
    ///         AdvancedConfig::default()
    ///     }
    ///     Err(e) => return Err(e),
    /// };
    ///
    /// // Use as Option type for natural handling
    /// fn try_get_optional_data(storage: &Storage, key: &str) -> Option<Data> {
    ///     match storage.get(key) {
    ///         Ok(data) => Some(data),
    ///         Err(Error::EntryNotFound(_)) => None,
    ///         Err(e) => {
    ///             log::error!("Storage error retrieving optional data: {}", e);
    ///             None
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Design Considerations
    ///
    /// When designing storage-based applications, consider:
    ///
    /// - Whether missing data is an error or normal condition
    /// - Default value strategies for missing configuration or settings
    /// - Lazy vs eager initialization patterns
    /// - Cache warming strategies to prevent frequent misses
    /// - Data lifecycle management (creation, migration, cleanup)
    ///
    /// # Monitoring
    ///
    /// While not typically an error, monitoring EntryNotFound patterns can provide insights:
    ///
    /// - High frequency might indicate missing data initialization
    /// - Specific keys with frequent misses might need preloading
    /// - Patterns can inform caching and optimization strategies
    #[error("Entry not found: {0}")]
    EntryNotFound(String),

    /// General storage operation failure.
    ///
    /// This error encompasses various storage-related failures including write operations,
    /// transaction failures, data serialization errors, compression/decompression issues,
    /// encryption/decryption problems, index corruption, storage capacity issues, or other
    /// storage backend problems that don't fit into more specific error categories.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the storage operation failure
    ///
    /// # Recovery Strategies
    ///
    /// - Implement transactional rollback mechanisms for failed multi-step operations
    /// - Use write-ahead logging (WAL) for critical operations to enable recovery
    /// - Monitor storage health metrics and implement automated alerting
    /// - Implement data replication and backup strategies for high availability
    /// - Use storage-specific error handling based on the backend type and capabilities
    /// - Implement circuit breaker patterns to prevent cascading failures
    /// - Consider graceful degradation modes when storage is partially unavailable
    ///
    /// # Common Causes
    ///
    /// - **Write Failures**: Disk full, permissions issues, hardware failures
    /// - **Transaction Failures**: Deadlocks, constraint violations, concurrent access issues
    /// - **Serialization Errors**: Data format changes, type incompatibilities, size limits
    /// - **Compression Issues**: Corruption during compression/decompression operations
    /// - **Encryption Problems**: Key rotation issues, authentication failures, data tampering
    /// - **Index Corruption**: Database index inconsistencies, filesystem corruption
    /// - **Resource Exhaustion**: Memory limits, connection pools, temporary space
    /// - **Configuration Errors**: Invalid settings, incompatible options, missing dependencies
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_store::Error;
    ///
    /// // Implement transactional rollback on failure
    /// async fn save_batch_with_rollback(
    ///     storage: &mut Storage,
    ///     data_batch: &[DataItem]
    /// ) -> Result<(), Error> {
    ///     let transaction = storage.begin_transaction().await?;
    ///
    ///     match transaction.save_multiple(data_batch).await {
    ///         Ok(()) => {
    ///             transaction.commit().await?;
    ///             log::info!("Successfully saved {} items", data_batch.len());
    ///             Ok(())
    ///         }
    ///         Err(Error::Store(reason)) => {
    ///             log::error!("Batch save failed: {}, rolling back transaction", reason);
    ///             transaction.rollback().await?;
    ///
    ///             // Try saving items individually to identify problematic entries
    ///             let mut success_count = 0;
    ///             for (i, item) in data_batch.iter().enumerate() {
    ///                 match storage.save_single(item).await {
    ///                     Ok(()) => success_count += 1,
    ///                     Err(e) => log::warn!("Failed to save item {}: {}", i, e),
    ///                 }
    ///             }
    ///
    ///             if success_count > 0 {
    ///                 log::info!("Saved {}/{} items individually", success_count, data_batch.len());
    ///             }
    ///
    ///             Err(Error::Store(format!(
    ///                 "Batch operation failed, saved {}/{} items: {}",
    ///                 success_count, data_batch.len(), reason
    ///             )))
    ///         }
    ///         Err(e) => {
    ///             transaction.rollback().await?;
    ///             Err(e)
    ///         }
    ///     }
    /// }
    ///
    /// // Implement retry logic with different strategies for different error types
    /// async fn store_with_retry(storage: &Storage, key: &str, data: &[u8]) -> Result<(), Error> {
    ///     for attempt in 0..3 {
    ///         match storage.put(key, data).await {
    ///             Ok(()) => return Ok(()),
    ///             Err(Error::Store(reason)) if reason.contains("temporary") => {
    ///                 let delay = std::time::Duration::from_millis(100 * (attempt + 1));
    ///                 log::info!("Temporary storage error, retrying in {:?}: {}", delay, reason);
    ///                 tokio::time::sleep(delay).await;
    ///             }
    ///             Err(Error::Store(reason)) if reason.contains("full") => {
    ///                 log::error!("Storage full, triggering cleanup: {}", reason);
    ///                 trigger_storage_cleanup().await?;
    ///                 // Don't retry immediately, let cleanup complete
    ///                 return Err(Error::Store(format!("Storage capacity exceeded: {}", reason)));
    ///             }
    ///             Err(Error::Store(reason)) => {
    ///                 log::error!("Permanent storage error on attempt {}: {}", attempt + 1, reason);
    ///                 return Err(Error::Store(reason));
    ///             }
    ///             Err(e) => return Err(e),
    ///         }
    ///     }
    ///     Err(Error::Store("Maximum retry attempts exceeded".to_string()))
    /// }
    /// ```
    ///
    /// # Monitoring and Alerting
    ///
    /// Store errors should be actively monitored for:
    ///
    /// - **Error Frequency**: Sudden increases may indicate system problems
    /// - **Error Patterns**: Specific error types might indicate infrastructure issues
    /// - **Resource Utilization**: Correlation with disk space, memory, or connection usage
    /// - **Performance Degradation**: Storage errors often precede performance issues
    /// - **Data Integrity**: Serialization and compression errors might indicate corruption
    ///
    /// # Prevention Strategies
    ///
    /// - Regular storage health checks and maintenance
    /// - Proactive monitoring of storage capacity and performance metrics
    /// - Automated backup and replication verification
    /// - Load testing to identify capacity limits and bottlenecks
    /// - Regular data integrity checks and validation
    /// - Proper resource allocation and capacity planning
    #[error("Store error: {0}")]
    Store(String),
}
