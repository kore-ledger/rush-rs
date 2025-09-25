// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor System Error Types
//!
//! This module provides comprehensive error handling for the actor system, defining all possible
//! error conditions that can occur during actor lifecycle management, message passing, system
//! operations, and data storage interactions. The error types are designed to provide detailed
//! context for debugging while maintaining performance and enabling appropriate error handling
//! strategies throughout the system.
//!

use crate::ActorPath;

use thiserror::Error;
// GRCOV-START
/// Comprehensive error handling for the actor system.
///
/// This enum defines all possible error conditions that can occur within the actor system,
/// providing detailed context for debugging and enabling appropriate error handling strategies.
/// Errors are categorized by their severity and impact on system operation, allowing for
/// differentiated response strategies ranging from simple retries to system-wide recovery.
///
/// # Error Categories
///
/// ## Communication Errors
/// - **Send**: Message delivery failures between actors
/// - **UnexpectedResponse**: Protocol violations in actor communication
/// - **SendEvent**: Event bus publishing failures
///
/// ## Actor Lifecycle Errors
/// - **Create**: Actor instantiation and initialization failures
/// - **Exists**: Duplicate actor creation attempts
/// - **NotFound**: References to non-existent actors
/// - **Stop**: Actor termination issues
/// - **Start**: Actor system startup failures
///
/// ## Data Access Errors
/// - **CreateStore**: Storage system initialization failures
/// - **Get**: Data retrieval failures from storage
/// - **EntryNotFound**: Missing data entries in storage
/// - **Store**: General storage operation failures
/// - **State**: Actor state corruption or inconsistency
///
/// ## System Errors
/// - **Functional**: Non-critical operational errors (recoverable)
/// - **FunctionalFail**: Critical operational errors (may require restart)
/// - **ReTry**: Retry limit exceeded for operations
/// - **NotHelper**: Helper service access failures
///
/// # Error Handling Strategies
///
/// Errors should be handled based on their category and severity:
///
/// 1. **Recoverable Errors**: Can be retried or handled gracefully without stopping the actor
/// 2. **Non-Recoverable Errors**: Require actor restart or parent intervention
/// 3. **System-Critical Errors**: May require system-wide recovery or shutdown
///
/// # Thread Safety
///
/// This error type implements `Clone` and is safe to share across threads, enabling
/// error propagation throughout the distributed actor system.
///
/// # Examples
///
/// ```rust
/// use rush_actor::{Error, ActorPath};
///
/// // Handle communication error with retry
/// match actor_ref.tell(message).await {
///     Err(Error::Send(_)) => {
///         // Retry message send with exponential backoff
///         tokio::time::sleep(Duration::from_millis(100)).await;
///         actor_ref.tell(message).await?;
///     }
///     Err(Error::NotFound(path)) => {
///         // Actor doesn't exist, create it first
///         let actor_ref = system.create_actor("new-actor", Actor::new()).await?;
///         actor_ref.tell(message).await?;
///     }
///     Ok(()) => println!("Message sent successfully"),
///     Err(e) => return Err(e), // Propagate other errors
/// }
/// ```
#[derive(Clone, Debug, Error, PartialEq)]
pub enum Error {
    /// Message delivery failure to target actor.
    ///
    /// This error occurs when the actor system fails to deliver a message to its intended
    /// recipient. Common causes include actor mailbox overflow, network issues in distributed
    /// setups, or the target actor being in an unresponsive state.
    ///
    /// # Context
    ///
    /// * Contains the actor identifier or error description for debugging
    ///
    /// # Recovery Strategies
    ///
    /// - Implement exponential backoff retry logic
    /// - Check actor health and restart if necessary
    /// - Consider message priority queuing for critical messages
    /// - Monitor system load and implement flow control
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Retry message send with backoff
    /// for attempt in 0..3 {
    ///     match actor_ref.tell(message.clone()).await {
    ///         Ok(()) => break,
    ///         Err(Error::Send(_)) if attempt < 2 => {
    ///             tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await;
    ///         }
    ///         Err(e) => return Err(e),
    ///     }
    /// }
    /// ```
    #[error("An error occurred while sending a message to actor: {0}.")]
    Send(String),

    /// Protocol violation in actor communication.
    ///
    /// This error indicates that an actor returned a response that doesn't match the expected
    /// response type or format according to the established communication protocol. This typically
    /// indicates a programming error, version mismatch, or corruption in the message handling logic.
    ///
    /// # Context
    ///
    /// * First parameter: Path of the actor that sent the invalid response
    /// * Second parameter: Description of the expected response type
    ///
    /// # Recovery Strategies
    ///
    /// - Validate actor implementation matches the expected protocol
    /// - Check for version compatibility issues between actors
    /// - Implement response validation and type checking
    /// - Consider graceful degradation for protocol mismatches
    ///
    /// # Examples
    ///
    /// ```rust
    /// match actor_ref.ask(query).await {
    ///     Err(Error::UnexpectedResponse(path, expected)) => {
    ///         log::error!("Actor {} sent invalid response, expected {}", path, expected);
    ///         // Fallback to default response or alternative actor
    ///         handle_protocol_error(path, expected).await?;
    ///     }
    ///     Ok(response) => process_response(response),
    ///     Err(e) => return Err(e),
    /// }
    /// ```
    #[error(
        "Actor {0} returned a response that was not expected, expected response: {1}"
    )]
    UnexpectedResponse(ActorPath, String),

    /// Actor creation or initialization failure.
    ///
    /// This error occurs when the actor system fails to create a new actor instance. Common
    /// causes include resource exhaustion, invalid actor configuration, initialization errors
    /// in the actor's constructor, or system-level restrictions on actor creation.
    ///
    /// # Context
    ///
    /// * First parameter: The intended path for the new actor
    /// * Second parameter: Detailed error description explaining the failure cause
    ///
    /// # Recovery Strategies
    ///
    /// - Verify resource availability (memory, handles, etc.)
    /// - Check actor configuration parameters for validity
    /// - Implement actor factory patterns for complex initialization
    /// - Use supervision strategies to handle creation failures gracefully
    /// - Consider lazy initialization for resource-heavy actors
    ///
    /// # Examples
    ///
    /// ```rust
    /// match system.create_actor("worker", WorkerActor::new()).await {
    ///     Err(Error::Create(path, reason)) => {
    ///         log::warn!("Failed to create actor at {}: {}", path, reason);
    ///         // Try alternative actor or fallback strategy
    ///         let backup_actor = create_backup_worker().await?;
    ///         Ok(backup_actor)
    ///     }
    ///     Ok(actor_ref) => Ok(actor_ref),
    ///     Err(e) => Err(e),
    /// }
    /// ```
    #[error("An error occurred while creating an actor: {0}/{1}.")]
    Create(ActorPath, String),

    /// Attempt to create actor at path that already exists.
    ///
    /// This error occurs when trying to create an actor at a path where another actor already
    /// exists. The actor system enforces unique paths to maintain clear actor identity and
    /// prevent naming conflicts that could lead to message delivery confusion.
    ///
    /// # Context
    ///
    /// * Contains the actor path that already exists in the system
    ///
    /// # Recovery Strategies
    ///
    /// - Generate unique actor names using timestamps or UUIDs
    /// - Check for existing actors before creation
    /// - Implement actor replacement strategies if appropriate
    /// - Use hierarchical naming to avoid conflicts
    /// - Consider using actor pools for similar functionality
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut attempt = 0;
    /// loop {
    ///     let actor_name = if attempt == 0 {
    ///         "processor".to_string()
    ///     } else {
    ///         format!("processor-{}", attempt)
    ///     };
    ///
    ///     match system.create_actor(&actor_name, ProcessorActor::new()).await {
    ///         Ok(actor_ref) => break Ok(actor_ref),
    ///         Err(Error::Exists(_)) => {
    ///             attempt += 1;
    ///             if attempt > 10 {
    ///                 return Err(Error::Create(ActorPath::from(actor_name),
    ///                     "Too many naming conflicts".to_string()));
    ///             }
    ///         }
    ///         Err(e) => return Err(e),
    ///     }
    /// }
    /// ```
    #[error("Actor {0} exist.")]
    Exists(ActorPath),

    /// Reference to non-existent actor.
    ///
    /// This error occurs when attempting to send a message to, query, or interact with an
    /// actor that doesn't exist in the system. This can happen due to actor termination,
    /// incorrect path references, or timing issues where an actor is removed between
    /// reference acquisition and usage.
    ///
    /// # Context
    ///
    /// * Contains the path of the actor that could not be found
    ///
    /// # Recovery Strategies
    ///
    /// - Implement actor existence checking before message sending
    /// - Use weak references for optional actor communication
    /// - Implement actor registry with health monitoring
    /// - Create actors on-demand when they don't exist
    /// - Handle actor lifecycle events to update references
    ///
    /// # Examples
    ///
    /// ```rust
    /// match system.get_actor(&actor_path) {
    ///     Err(Error::NotFound(path)) => {
    ///         log::info!("Actor {} not found, creating new instance", path);
    ///         // Create actor on-demand
    ///         let new_actor = system.create_actor_from_path(&path,
    ///             DefaultActor::new()).await?;
    ///         new_actor.tell(message).await?;
    ///     }
    ///     Ok(actor_ref) => {
    ///         actor_ref.tell(message).await?;
    ///     }
    ///     Err(e) => return Err(e),
    /// }
    /// ```
    #[error("Actor {0} not found.")]
    NotFound(ActorPath),

    /// Actor termination failure.
    ///
    /// This error occurs when the actor system fails to properly stop an actor. This can
    /// happen due to the actor being in an unresponsive state, having blocked message
    /// handlers, resource cleanup failures, or supervision strategy conflicts during shutdown.
    ///
    /// # Recovery Strategies
    ///
    /// - Implement graceful shutdown with timeouts
    /// - Use force termination for unresponsive actors
    /// - Clean up actor resources manually if needed
    /// - Log termination failures for debugging
    /// - Check for deadlocks in actor message handlers
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Attempt graceful shutdown with timeout
    /// match tokio::time::timeout(
    ///     Duration::from_secs(30),
    ///     actor_ref.stop()
    /// ).await {
    ///     Ok(Ok(())) => log::info!("Actor stopped gracefully"),
    ///     Ok(Err(Error::Stop)) | Err(_) => {
    ///         log::warn!("Graceful shutdown failed, forcing termination");
    ///         force_terminate_actor(actor_ref).await?;
    ///     }
    ///     Ok(Err(e)) => return Err(e),
    /// }
    /// ```
    #[error("An error occurred while stopping an actor.")]
    Stop,

    /// Actor system startup failure.
    ///
    /// This error occurs during actor system initialization when critical components fail
    /// to start properly. Common causes include resource allocation failures, configuration
    /// errors, network binding issues, or dependency initialization problems.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the startup failure cause
    ///
    /// # Recovery Strategies
    ///
    /// - Validate system configuration before startup
    /// - Check resource availability (ports, memory, file handles)
    /// - Implement retry logic with exponential backoff
    /// - Use health checks for system dependencies
    /// - Provide fallback configurations for common failures
    ///
    /// # Examples
    ///
    /// ```rust
    /// for attempt in 0..3 {
    ///     match ActorSystem::start(config.clone()).await {
    ///         Ok(system) => return Ok(system),
    ///         Err(Error::Start(reason)) => {
    ///             log::warn!("System start failed (attempt {}): {}", attempt + 1, reason);
    ///             if attempt < 2 {
    ///                 tokio::time::sleep(Duration::from_secs(5)).await;
    ///             }
    ///         }
    ///         Err(e) => return Err(e),
    ///     }
    /// }
    /// Err(Error::Start("Maximum startup attempts exceeded".to_string()))
    /// ```
    #[error("An error occurred while starting the actor system: {0}")]
    Start(String),

    /// Event bus message publishing failure.
    ///
    /// This error occurs when the system fails to publish an event to the event bus.
    /// Common causes include event bus overflow, serialization failures, network issues
    /// in distributed systems, or subscriber processing bottlenecks causing backpressure.
    ///
    /// # Context
    ///
    /// * Contains description of the event publishing failure
    ///
    /// # Recovery Strategies
    ///
    /// - Implement event queuing with persistence for critical events
    /// - Use circuit breaker pattern for event publishing
    /// - Monitor event bus health and capacity
    /// - Implement event priority levels and selective dropping
    /// - Consider async event publishing to prevent blocking
    ///
    /// # Examples
    ///
    /// ```rust
    /// match event_bus.publish(critical_event).await {
    ///     Err(Error::SendEvent(reason)) => {
    ///         log::error!("Failed to publish critical event: {}", reason);
    ///         // Store event for later retry
    ///         event_store.persist_for_retry(critical_event).await?;
    ///         // Try alternative notification method
    ///         fallback_notifier.notify(critical_event).await?;
    ///     }
    ///     Ok(()) => log::debug!("Event published successfully"),
    ///     Err(e) => return Err(e),
    /// }
    /// ```
    #[error("An error occurred while sending an event to event bus: {0}")]
    SendEvent(String),

    /// Storage system initialization failure.
    ///
    /// This error occurs when the actor system fails to create or initialize a storage
    /// backend. Common causes include database connection failures, insufficient permissions,
    /// storage capacity issues, or configuration errors in the storage layer.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the storage creation failure
    ///
    /// # Recovery Strategies
    ///
    /// - Validate storage configuration and credentials
    /// - Implement storage backend fallbacks (memory, file, database)
    /// - Check storage system health and connectivity
    /// - Use connection pooling and retry logic for network storage
    /// - Consider read-only mode for degraded storage scenarios
    ///
    /// # Examples
    ///
    /// ```rust
    /// let storage = match create_primary_storage(&config).await {
    ///     Ok(store) => store,
    ///     Err(Error::CreateStore(reason)) => {
    ///         log::warn!("Primary storage failed: {}, trying backup", reason);
    ///         match create_backup_storage(&config).await {
    ///             Ok(backup_store) => backup_store,
    ///             Err(_) => {
    ///                 log::error!("All storage backends failed, using in-memory");
    ///                 create_memory_storage()
    ///             }
    ///         }
    ///     }
    ///     Err(e) => return Err(e),
    /// };
    /// ```
    #[error("Can't create store: {0}")]
    CreateStore(String),

    /// Data retrieval failure from storage.
    ///
    /// This error occurs when the actor system fails to retrieve data from the storage layer.
    /// Common causes include network connectivity issues, storage system failures, permission
    /// problems, or corruption in the stored data that prevents successful deserialization.
    ///
    /// # Context
    ///
    /// * Contains description of the data retrieval failure
    ///
    /// # Recovery Strategies
    ///
    /// - Implement cache layers to reduce storage dependencies
    /// - Use retry logic with exponential backoff for transient failures
    /// - Provide default values for non-critical data
    /// - Monitor storage system health and switch to backup systems
    /// - Implement data validation and corruption detection
    ///
    /// # Examples
    ///
    /// ```rust
    /// let data = match storage.get(&key).await {
    ///     Ok(value) => value,
    ///     Err(Error::Get(reason)) => {
    ///         log::warn!("Storage get failed: {}, trying cache", reason);
    ///         match cache.get(&key).await {
    ///             Ok(cached_value) => cached_value,
    ///             Err(_) => {
    ///                 log::info!("Cache miss, using default value");
    ///                 DataType::default()
    ///             }
    ///         }
    ///     }
    ///     Err(e) => return Err(e),
    /// };
    /// ```
    #[error("Get error: {0}")]
    Get(String),

    /// Requested data entry not found in storage.
    ///
    /// This error occurs when attempting to retrieve data that doesn't exist in the storage
    /// system. Unlike `Get` errors, this specifically indicates that the storage operation
    /// succeeded but the requested entry is not present, which is a normal condition in
    /// many scenarios.
    ///
    /// # Context
    ///
    /// * Contains the identifier or description of the missing entry
    ///
    /// # Recovery Strategies
    ///
    /// - Check if the entry should exist or if this is expected behavior
    /// - Create default entries for missing required data
    /// - Implement lazy loading for optional data
    /// - Use optional types to handle missing data gracefully
    /// - Cache frequently accessed entries to reduce lookups
    ///
    /// # Examples
    ///
    /// ```rust
    /// let user_data = match storage.get_user(&user_id).await {
    ///     Ok(data) => data,
    ///     Err(Error::EntryNotFound(_)) => {
    ///         log::info!("User {} not found, creating default profile", user_id);
    ///         let default_profile = UserProfile::default_for_user(&user_id);
    ///         storage.save_user(&user_id, &default_profile).await?;
    ///         default_profile
    ///     }
    ///     Err(e) => return Err(e),
    /// };
    /// ```
    #[error("Entry not found: {0}")]
    EntryNotFound(String),

    /// General storage operation failure.
    ///
    /// This error encompasses various storage-related failures including write operations,
    /// transaction failures, index corruption, storage capacity issues, or other storage
    /// backend problems that don't fit into more specific error categories.
    ///
    /// # Context
    ///
    /// * Contains detailed description of the storage operation failure
    ///
    /// # Recovery Strategies
    ///
    /// - Implement transactional rollback for failed operations
    /// - Use write-ahead logging for critical operations
    /// - Monitor storage health and implement alerting
    /// - Implement data replication for high availability
    /// - Use storage-specific error handling based on the backend type
    ///
    /// # Examples
    ///
    /// ```rust
    /// let transaction = storage.begin_transaction().await?;
    /// match transaction.save_multiple(&data_batch).await {
    ///     Ok(()) => transaction.commit().await?,
    ///     Err(Error::Store(reason)) => {
    ///         log::error!("Batch save failed: {}, rolling back", reason);
    ///         transaction.rollback().await?;
    ///         // Try saving individually to identify problematic entries
    ///         for item in data_batch {
    ///             if let Err(e) = storage.save_single(&item).await {
    ///                 log::warn!("Failed to save item {:?}: {}", item, e);
    ///             }
    ///         }
    ///     }
    ///     Err(e) => {
    ///         transaction.rollback().await?;
    ///         return Err(e);
    ///     }
    /// }
    /// ```
    #[error("Store error: {0}")]
    Store(String),

    /// Non-critical operational error that doesn't compromise system stability.
    ///
    /// This error represents recoverable conditions that don't affect the core functionality
    /// of the actor system. Examples include optional feature failures, non-essential service
    /// unavailability, or degraded performance conditions that the system can continue to
    /// operate through.
    ///
    /// # Context
    ///
    /// * Contains description of the functional error condition
    ///
    /// # Recovery Strategies
    ///
    /// - Log the error for monitoring but continue operation
    /// - Implement graceful degradation of non-critical features
    /// - Use circuit breaker patterns to prevent cascading failures
    /// - Provide alternative implementations or fallback behaviors
    /// - Monitor error frequency to detect systemic issues
    ///
    /// # Examples
    ///
    /// ```rust
    /// match optional_service.perform_enhancement(&data).await {
    ///     Ok(enhanced_data) => process_data(enhanced_data),
    ///     Err(Error::Functional(reason)) => {
    ///         log::info!("Enhancement service failed: {}, using original data", reason);
    ///         // Continue with unenhanced data
    ///         process_data(data)
    ///     }
    ///     Err(e) => return Err(e), // Propagate critical errors
    /// }
    /// ```
    #[error("Error: {0}")]
    Functional(String),

    /// Critical operational error that compromises system stability.
    ///
    /// This error represents serious conditions that significantly impact the actor system's
    /// ability to function correctly. Unlike `Functional` errors, these require immediate
    /// attention and may necessitate actor restart, system recovery, or escalation to
    /// supervision strategies.
    ///
    /// # Context
    ///
    /// * Contains description of the critical failure condition
    ///
    /// # Recovery Strategies
    ///
    /// - Trigger supervision strategies for actor restart
    /// - Implement system-wide recovery procedures
    /// - Alert monitoring systems for immediate attention
    /// - Isolate affected components to prevent failure spread
    /// - Consider graceful system shutdown if recovery is not possible
    ///
    /// # Examples
    ///
    /// ```rust
    /// match critical_operation().await {
    ///     Ok(result) => Ok(result),
    ///     Err(Error::FunctionalFail(reason)) => {
    ///         log::error!("Critical failure in {}: {}", actor_path, reason);
    ///         // Trigger actor restart through supervision
    ///         supervision_strategy.handle_failure(actor_ref, reason).await?;
    ///         // May not return if actor is restarted
    ///         Err(Error::FunctionalFail(reason))
    ///     }
    ///     Err(e) => Err(e),
    /// }
    /// ```
    #[error("Error: {0}")]
    FunctionalFail(String),

    /// Actor state corruption or inconsistency detected.
    ///
    /// This error occurs when the actor system detects that an actor's internal state
    /// has become corrupted, inconsistent, or invalid. This can happen due to concurrent
    /// modification bugs, serialization/deserialization errors, or logical errors in
    /// state transition handling.
    ///
    /// # Context
    ///
    /// * Contains description of the state error and potentially the last known valid state
    ///
    /// # Recovery Strategies
    ///
    /// - Implement state validation and checksum verification
    /// - Use immutable state patterns to prevent corruption
    /// - Implement state rollback to the last known good state
    /// - Trigger actor restart to restore clean state
    /// - Use event sourcing for state reconstruction
    ///
    /// # Examples
    ///
    /// ```rust
    /// impl Actor for StatefulActor {
    ///     async fn handle_message(&mut self, message: Message, ctx: &mut ActorContext<Self>)
    ///         -> Result<Response, Error> {
    ///         // Validate state before processing
    ///         if !self.validate_state() {
    ///             let backup_state = self.get_backup_state();
    ///             log::error!("State corruption detected, restoring from backup");
    ///             self.restore_state(backup_state);
    ///             return Err(Error::State("State restored from backup".to_string()));
    ///         }
    ///
    ///         // Process message and validate state after
    ///         let result = self.process_message(message).await?;
    ///         if !self.validate_state() {
    ///             return Err(Error::State("State corrupted during processing".to_string()));
    ///         }
    ///
    ///         Ok(result)
    ///     }
    /// }
    /// ```
    #[error("State error: {0}")]
    State(String),

    /// Maximum retry limit exceeded for operation.
    ///
    /// This error occurs when an operation has been retried the maximum number of times
    /// without success. It indicates that a transient error condition has persisted long
    /// enough to be considered a permanent failure, and further retry attempts should be
    /// avoided to prevent resource waste.
    ///
    /// # Recovery Strategies
    ///
    /// - Escalate to manual intervention or alternative strategies
    /// - Implement exponential backoff with jitter to reduce system load
    /// - Log detailed retry history for debugging
    /// - Consider circuit breaker patterns to prevent retry storms
    /// - Use different retry strategies for different error types
    ///
    /// # Examples
    ///
    /// ```rust
    /// async fn retry_operation<F, T, E>(
    ///     mut operation: F,
    ///     max_retries: u32,
    ///     base_delay: Duration,
    /// ) -> Result<T, Error>
    /// where
    ///     F: FnMut() -> Result<T, E>,
    /// {
    ///     for attempt in 0..max_retries {
    ///         match operation() {
    ///             Ok(result) => return Ok(result),
    ///             Err(_) if attempt < max_retries - 1 => {
    ///                 let delay = base_delay * 2_u32.pow(attempt);
    ///                 tokio::time::sleep(delay).await;
    ///             }
    ///             Err(_) => {
    ///                 log::error!("Operation failed after {} attempts", max_retries);
    ///                 return Err(Error::ReTry);
    ///             }
    ///         }
    ///     }
    ///     unreachable!()
    /// }
    /// ```
    #[error("The maximum number of retries has been reached.")]
    ReTry,

    /// Helper service or utility unavailable.
    ///
    /// This error occurs when an actor or system component cannot access a required helper
    /// service, utility function, or dependency. Helper services provide supporting functionality
    /// like configuration management, logging, metrics collection, or external integrations
    /// that actors depend on for proper operation.
    ///
    /// # Context
    ///
    /// * Contains the name or identifier of the inaccessible helper service
    ///
    /// # Recovery Strategies
    ///
    /// - Implement service discovery and health checking for helpers
    /// - Use dependency injection with fallback implementations
    /// - Cache helper service references and refresh on failure
    /// - Implement helper service retry logic with circuit breakers
    /// - Provide minimal functionality when helpers are unavailable
    ///
    /// # Examples
    ///
    /// ```rust
    /// impl Actor for ServiceActor {
    ///     async fn handle_message(&mut self, msg: Message, ctx: &mut ActorContext<Self>)
    ///         -> Result<Response, Error> {
    ///         // Try to get required helper service
    ///         let config_helper = match ctx.get_helper::<ConfigHelper>("config").await {
    ///             Ok(helper) => helper,
    ///             Err(Error::NotHelper(name)) => {
    ///                 log::warn!("Config helper '{}' unavailable, using defaults", name);
    ///                 // Use default configuration or cached values
    ///                 return self.handle_with_defaults(msg).await;
    ///             }
    ///             Err(e) => return Err(e),
    ///         };
    ///
    ///         // Use helper service normally
    ///         let config = config_helper.get_config(&msg.config_key).await?;
    ///         self.process_with_config(msg, config).await
    ///     }
    /// }
    /// ```
    #[error("Helper {0} could not be accessed.")]
    NotHelper(String),
}
// GRCOV-END
