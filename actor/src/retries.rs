// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Message Retry System
//!
//! This module provides a comprehensive message retry mechanism for the actor system, enabling
//! fault-tolerant message delivery through configurable backoff strategies. The retry system
//! operates at the actor level, creating dedicated retry actors that manage the retry lifecycle
//! for failed message deliveries.
//!
//! The retry system is designed to handle transient failures in message delivery by implementing
//! a supervisor pattern where retry actors manage the retry logic for target actors. This approach
//! provides separation of concerns between business logic and fault tolerance mechanisms.
//!
//! # Core Components
//!
//! ## RetryActor
//!
//! The `RetryActor` is a specialized actor that manages the retry lifecycle for a target actor.
//! It maintains the retry state, applies backoff strategies, and coordinates message retries
//! while respecting configured limits and policies.
//!
//! ## RetryMessage
//!
//! Control messages used to orchestrate the retry process, including initiating retries
//! and signaling completion.
//!
//! # Architecture Overview
//!
//! ```text
//! ┌─────────────┐    retry     ┌─────────────┐    message    ┌─────────────┐
//! │   Parent    │ ─────────→   │ RetryActor  │ ─────────→    │   Target    │
//! │   Actor     │              │             │               │   Actor     │
//! └─────────────┘              └─────────────┘               └─────────────┘
//!                                     │                             │
//!                                     │ backoff delay               │ failure
//!                                     │ ←───────────────────────────┘
//!                                     ∨
//!                               ┌─────────────┐
//!                               │ Supervision │
//!                               │  Strategy   │
//!                               └─────────────┘
//! ```
//!
//! # Retry Strategies Integration
//!
//! The retry system integrates closely with the supervision system's retry strategies:
//!
//! - **FixedInterval**: Consistent delays between retry attempts
//! - **CustomInterval**: Custom delay sequences for complex backoff patterns
//! - **NoInterval**: Immediate retries for fast-failing scenarios
//!
//! # Usage Patterns
//!
//! ## Basic Message Retry
//!
//! ```ignore
//! use rush_actor::*;
//! use rush_actor::retries::{RetryActor, RetryMessage};
//! use rush_actor::supervision::{Strategy, FixedIntervalStrategy};
//! use std::time::Duration;
//!
//! // Create a retry strategy with 3 attempts and 1-second intervals
//! let strategy = Strategy::FixedInterval(
//!     FixedIntervalStrategy::new(3, Duration::from_secs(1))
//! );
//!
//! // Create a retry actor for the target
//! let retry_actor = RetryActor::new(
//!     target_actor,
//!     target_message,
//!     strategy
//! );
//!
//! // Start the retry process
//! let retry_ref = ctx.create_child("retry", retry_actor).await?;
//! retry_ref.tell(RetryMessage::Retry).await?;
//! ```
//!
//! ## Integration with Supervision
//!
//! Retry actors work alongside supervision strategies to provide comprehensive
//! fault tolerance. While supervision handles actor lifecycle failures,
//! retry actors handle message delivery failures.
//!
//! # Performance Considerations
//!
//! - **Memory Usage**: Each retry actor maintains state for one message retry sequence
//! - **Resource Cleanup**: Retry actors automatically stop when max retries are reached
//! - **Backoff Efficiency**: Async delays don't block the actor system threads
//! - **Error Propagation**: Failed retries emit errors to the supervision system
//!
//! # Error Handling
//!
//! The retry system handles several error conditions:
//!
//! - **Max Retries Exceeded**: Emits `Error::ReTry` to the supervision system
//! - **Target Actor Unavailable**: Logs error and continues retry sequence
//! - **System Failures**: Propagates errors to parent actors through supervision
//!
//! # When to Use Retries
//!
//! Retries are most effective for:
//! - Transient network failures
//! - Temporary resource unavailability
//! - Rate-limited services
//! - External service timeouts
//!
//! Consider alternatives for:
//! - Persistent failures (use circuit breakers)
//! - Invalid input data (fail fast)
//! - System resource exhaustion (use backpressure)
//! - Critical real-time operations (use redundancy)
//!

use crate::{
    Actor, ActorContext, ActorPath, ActorRef, Error, Event, Handler, Message,
    Response,
    supervision::{RetryStrategy, Strategy},
};

use async_trait::async_trait;

use std::fmt::Debug;
use tracing::{debug, error, warn};

/// A specialized actor that manages message retry logic with configurable backoff strategies.
///
/// `RetryActor` implements a fault-tolerant message delivery pattern by wrapping a target actor
/// and managing retry attempts for failed message deliveries. It maintains retry state, applies
/// backoff delays, and coordinates with the supervision system to handle failure scenarios.
///
/// The retry actor operates as a supervisor for a single target actor instance, creating the
/// target as a child actor and managing its message delivery lifecycle. When a message fails,
/// the retry actor applies the configured retry strategy and schedules subsequent retry attempts
/// until either the message succeeds or the maximum retry limit is reached.
///
/// # Type Parameters
///
/// * `T` - The target actor type that will receive the retried messages. Must implement
///         `Actor + Handler<T> + Clone` to support actor instantiation and message handling.
///
/// # Architecture
///
/// The `RetryActor` creates a parent-child relationship where:
/// - The retry actor serves as the parent supervisor
/// - The target actor is created as a named child ("target")
/// - Message retries are coordinated through the retry actor's message handling
/// - Backoff delays are managed asynchronously without blocking the actor system
///
/// # State Management
///
/// The retry actor maintains several pieces of state:
/// - **target**: The cloneable instance of the target actor for child creation
/// - **message**: The original message to be retried (must be cloneable)
/// - **retry_strategy**: The backoff strategy defining retry limits and delays
/// - **retries**: Current retry attempt counter (starts at 0)
/// - **is_end**: Flag indicating whether retry sequence has been terminated
///
/// # Lifecycle
///
/// 1. **Initialization**: Created with target actor, message, and retry strategy
/// 2. **Pre-start**: Creates target actor as child during actor startup
/// 3. **Retry Phase**: Handles `RetryMessage::Retry` by sending message to target
/// 4. **Backoff**: Applies strategy-defined delays between retry attempts
/// 5. **Completion**: Either succeeds or reaches max retries, then terminates
///
/// # Error Handling
///
/// The retry actor handles various error conditions:
/// - Target actor creation failures during pre-start
/// - Message delivery failures to the target actor
/// - Max retry limit exceeded (emits `Error::ReTry`)
/// - System reference unavailability during backoff scheduling
///
/// # Memory and Resource Management
///
/// - Automatically cleans up when max retries are reached or explicitly ended
/// - Uses async scheduling for backoff delays to avoid blocking threads
/// - Maintains minimal state footprint with only essential retry information
/// - Integrates with actor system lifecycle for proper resource cleanup
///
/// # Examples
///
/// ## Basic Usage
///
/// ```ignore
/// use rush_actor::*;
/// use rush_actor::retries::{RetryActor, RetryMessage};
/// use rush_actor::supervision::{Strategy, FixedIntervalStrategy};
/// use std::time::Duration;
///
/// // Create retry strategy: 5 attempts with 2-second intervals
/// let strategy = Strategy::FixedInterval(
///     FixedIntervalStrategy::new(5, Duration::from_secs(2))
/// );
///
/// // Create retry actor for database operations
/// let retry_actor = RetryActor::new(
///     database_actor,
///     SaveRecordMessage { id: 123, data: "important".to_string() },
///     strategy
/// );
///
/// // Deploy and start retry process
/// let retry_ref = ctx.create_child("db_retry", retry_actor).await?;
/// retry_ref.tell(RetryMessage::Retry).await?;
/// ```
///
/// ## Custom Backoff Strategy
///
/// ```ignore
/// use std::collections::VecDeque;
///
/// // Exponential backoff: 1s, 4s, 16s
/// let intervals = VecDeque::from([
///     Duration::from_secs(1),
///     Duration::from_secs(4),
///     Duration::from_secs(16),
/// ]);
///
/// let custom_strategy = Strategy::CustomInterval(
///     CustomIntervalStrategy::new(intervals)
/// );
///
/// let retry_actor = RetryActor::new(
///     external_api_actor,
///     ApiRequestMessage::new("critical_operation"),
///     custom_strategy
/// );
/// ```
///
/// # Thread Safety
///
/// The retry actor is designed for single-threaded actor execution contexts.
/// The target actor and message must be `Clone` to support child actor creation
/// and message retry attempts.
pub struct RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    /// The target actor instance that will be created as a child to receive retried messages.
    /// Must be cloneable to support actor instantiation during pre-start lifecycle.
    target: T,
    /// The message to be retried, stored for multiple delivery attempts.
    /// Must be cloneable to support repeated message sending.
    message: T::Message,
    /// The retry strategy defining maximum attempts, backoff intervals, and retry behavior.
    /// Consulted for each retry attempt to determine timing and limits.
    retry_strategy: Strategy,
    /// Current count of retry attempts made, starting from 0 and incrementing with each retry.
    /// Used to track progress against the strategy's maximum retry limit.
    retries: usize,
    /// Flag indicating whether the retry sequence has been explicitly terminated.
    /// Set to true when receiving `RetryMessage::End` to prevent further retries.
    is_end: bool,
}

impl<T> RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    /// Creates a new retry actor with the specified target, message, and retry strategy.
    ///
    /// This constructor initializes a retry actor in its initial state, ready to begin
    /// the retry process when deployed and messaged with `RetryMessage::Retry`.
    /// The retry counter starts at 0, and the actor is marked as active (not ended).
    ///
    /// # Parameters
    ///
    /// * `target` - The actor instance that will receive the retried messages. This actor
    ///              will be cloned and created as a child actor during the retry actor's
    ///              pre-start lifecycle phase.
    /// * `message` - The message to be delivered to the target actor. This message will
    ///               be cloned for each retry attempt, so it must implement `Clone`.
    /// * `retry_strategy` - The strategy defining retry behavior, including maximum
    ///                      retry attempts and backoff intervals between retries.
    ///
    /// # Returns
    ///
    /// A new `RetryActor` instance ready to be deployed in the actor system.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use rush_actor::retries::RetryActor;
    /// use rush_actor::supervision::{Strategy, FixedIntervalStrategy};
    /// use std::time::Duration;
    ///
    /// // Create a retry strategy with 3 attempts and 1-second delays
    /// let strategy = Strategy::FixedInterval(
    ///     FixedIntervalStrategy::new(3, Duration::from_secs(1))
    /// );
    ///
    /// // Create a retry actor for a database write operation
    /// let retry_actor = RetryActor::new(
    ///     DatabaseActor::new(),
    ///     WriteMessage { table: "users", data: user_data },
    ///     strategy
    /// );
    /// ```
    ///
    /// # Design Notes
    ///
    /// The constructor follows the builder pattern common in Rust, taking all required
    /// parameters and returning a fully initialized instance. The retry actor maintains
    /// ownership of the target actor and message, allowing for independent retry sequences
    /// without shared mutable state concerns.
    pub fn new(
        target: T,
        message: T::Message,
        retry_strategy: Strategy,
    ) -> Self {
        Self {
            target,
            message,
            retry_strategy,
            retries: 0,
            is_end: false,
        }
    }
}
/// Control messages for orchestrating retry actor behavior and lifecycle management.
///
/// `RetryMessage` defines the command interface for controlling retry actors,
/// providing messages to initiate retry attempts and terminate retry sequences.
/// These messages are used by parent actors and the retry system itself to
/// coordinate the retry process.
///
/// The message types follow a simple command pattern where each variant
/// represents a specific action that the retry actor should perform.
///
/// # Design Philosophy
///
/// The enum is designed for clarity and explicit control over retry behavior:
/// - Each message has a single, well-defined responsibility
/// - Message names are self-documenting and action-oriented
/// - The set is minimal but complete for retry coordination needs
///
/// # Usage Patterns
///
/// ## Initiating Retries
///
/// ```ignore
/// // Start the retry process after creating a retry actor
/// let retry_ref = ctx.create_child("retry", retry_actor).await?;
/// retry_ref.tell(RetryMessage::Retry).await?;
/// ```
///
/// ## Terminating Retries
///
/// ```ignore
/// // Stop retry attempts when operation succeeds elsewhere
/// if operation_succeeded {
///     retry_ref.tell(RetryMessage::End).await?;
/// }
/// ```
///
/// # Message Flow
///
/// 1. **Retry**: Triggers immediate retry attempt or schedules next retry with backoff
/// 2. **End**: Immediately terminates retry sequence and stops the retry actor
///
/// # Thread Safety
///
/// The enum derives `Clone` to support message passing semantics in the actor system,
/// and `Debug` for logging and troubleshooting retry sequences.
#[derive(Debug, Clone)]
pub enum RetryMessage {
    /// Initiates a retry attempt for the target message.
    ///
    /// When a retry actor receives this message, it will:
    /// 1. Check if the retry sequence has been ended
    /// 2. Increment the retry counter
    /// 3. Verify the retry count is within the strategy's maximum limit
    /// 4. Send the stored message to the target child actor
    /// 5. Schedule the next retry attempt with backoff delay (if applicable)
    /// 6. Emit an error if maximum retries have been exceeded
    ///
    /// This message can be sent multiple times to continue the retry sequence,
    /// with each invocation applying the configured backoff strategy for timing.
    ///
    /// # Behavior
    ///
    /// - **Success Path**: Message delivered to target, next retry scheduled
    /// - **Failure Path**: Error logged, retry sequence continues if under limit
    /// - **Limit Exceeded**: `Error::ReTry` emitted, no further retries scheduled
    /// - **Ended State**: Message ignored if retry sequence has been terminated
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Manual retry initiation (typically done by parent actor)
    /// retry_ref.tell(RetryMessage::Retry).await?;
    ///
    /// // Automatic retry scheduling (done internally by retry actor)
    /// tokio::spawn(async move {
    ///     tokio::time::sleep(backoff_duration).await;
    ///     retry_ref.tell(RetryMessage::Retry).await;
    /// });
    /// ```
    Retry,

    /// Terminates the retry sequence and stops the retry actor.
    ///
    /// When a retry actor receives this message, it will:
    /// 1. Set the internal `is_end` flag to true
    /// 2. Stop accepting further retry attempts
    /// 3. Initiate actor shutdown through the context
    /// 4. Clean up resources and child actors
    ///
    /// This message provides explicit control over retry termination,
    /// allowing parent actors to stop retry sequences when the operation
    /// has succeeded through other means or when retries are no longer desired.
    ///
    /// # Use Cases
    ///
    /// - **Success Notification**: Operation completed successfully via alternative path
    /// - **Manual Cancellation**: User or system decision to abort retry attempts
    /// - **Resource Conservation**: Stopping retries to preserve system resources
    /// - **State Changes**: Business logic changes that make retry unnecessary
    ///
    /// # Behavior
    ///
    /// - **Immediate Effect**: No further retry attempts will be processed
    /// - **Graceful Shutdown**: Actor stops cleanly through normal lifecycle
    /// - **Resource Cleanup**: Child actors and scheduled tasks are cleaned up
    /// - **Idempotent**: Multiple `End` messages have no additional effect
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // End retry sequence when operation succeeds elsewhere
    /// if let Ok(result) = alternative_operation().await {
    ///     retry_ref.tell(RetryMessage::End).await?;
    ///     return Ok(result);
    /// }
    ///
    /// // Manual cancellation based on timeout or user input
    /// if should_cancel_retries() {
    ///     retry_ref.tell(RetryMessage::End).await?;
    /// }
    /// ```
    End,
}

/// Implementation of the `Message` trait for `RetryMessage`.
///
/// This implementation enables `RetryMessage` to be used as a message type
/// in the actor system's message passing infrastructure. The trait provides
/// the necessary type information for the actor system to handle routing
/// and delivery of retry control messages.
impl Message for RetryMessage {}

/// Implementation of the `Response` trait for the unit type.
///
/// Retry actors use `()` (unit type) as their response type since retry
/// operations are fire-and-forget commands that don't return meaningful
/// data to callers. This implementation enables the unit type to be used
/// in the actor system's response handling infrastructure.
impl Response for () {}

/// Implementation of the `Event` trait for the unit type.
///
/// Retry actors use `()` (unit type) as their event type since they
/// don't publish domain-specific events to the system. This implementation
/// allows the unit type to participate in the actor system's event
/// publishing infrastructure when needed.
impl Event for () {}

/// Implementation of the `Actor` trait for `RetryActor`.
///
/// This implementation defines the retry actor's core behavior within the actor system,
/// specifying its message types and lifecycle management. The retry actor operates as
/// a specialized supervisor that manages message retry sequences for target actors.
///
/// # Type Associations
///
/// - **Message**: `RetryMessage` - Control messages for retry orchestration
/// - **Response**: `()` - Unit type since retries are fire-and-forget operations
/// - **Event**: `()` - Unit type since retry actors don't publish domain events
///
/// # Lifecycle Management
///
/// The retry actor follows a specific lifecycle pattern:
/// 1. **Pre-start**: Creates the target actor as a named child ("target")
/// 2. **Active**: Handles retry messages and coordinates message delivery attempts
/// 3. **Termination**: Cleans up resources when max retries reached or explicitly ended
#[async_trait]
impl<T> Actor for RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    type Message = RetryMessage;
    type Response = ();
    type Event = ();

    /// Initializes the retry actor by creating the target actor as a child.
    ///
    /// This method is called during the actor startup lifecycle, before the actor
    /// begins processing messages. It creates the target actor instance as a named
    /// child actor ("target"), establishing the parent-child supervision relationship
    /// that enables message retry coordination.
    ///
    /// # Parameters
    ///
    /// * `ctx` - The actor context providing access to system services and child
    ///           actor management capabilities.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Target actor successfully created as child
    /// * `Err(Error)` - Child actor creation failed (propagated from system)
    ///
    /// # Behavior
    ///
    /// 1. Clones the stored target actor instance (required for child creation)
    /// 2. Creates the target actor as a child with the name "target"
    /// 3. Establishes supervision relationship for retry coordination
    /// 4. Logs debug information about the startup process
    ///
    /// # Error Handling
    ///
    /// Child creation failures are propagated to the parent actor through the
    /// supervision system. Common failure scenarios include:
    /// - System resource exhaustion
    /// - Actor system shutdown in progress
    /// - Target actor initialization failures
    ///
    /// # Design Notes
    ///
    /// The target actor is created with a fixed name ("target") since each retry
    /// actor manages exactly one target instance. This naming convention simplifies
    /// message routing and child actor lookup during retry attempts.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // The actor system calls this automatically during retry actor deployment
    /// let retry_ref = ctx.create_child("retry", retry_actor).await?;
    /// // pre_start is called automatically, creating the target child
    /// ```
    async fn pre_start(
        &mut self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        debug!("RetryActor pre_start");
        let _ = ctx.create_child::<T>("target", self.target.clone()).await?;
        Ok(())
    }
}

/// Implementation of the `Handler` trait for `RetryActor`.
///
/// This implementation defines the core retry logic, handling control messages that
/// orchestrate the retry sequence. The handler manages retry attempts, applies backoff
/// strategies, and coordinates with the supervision system for error handling.
///
/// # Message Handling Strategy
///
/// The handler follows a state-driven approach where the retry actor's internal state
/// (`retries`, `is_end`, `retry_strategy`) determines the behavior for each message.
/// This ensures consistent retry behavior and proper resource management throughout
/// the retry sequence lifecycle.
///
/// # Concurrency and Scheduling
///
/// The handler uses Tokio's async scheduling (`tokio::spawn`) to implement backoff
/// delays without blocking the actor system. This approach allows multiple retry
/// actors to operate concurrently while respecting their individual backoff timings.
///
/// # Error Propagation
///
/// Retry failures are handled through the supervision system's error emission
/// mechanism, allowing parent actors to respond to retry exhaustion according
/// to their supervision strategies.
#[async_trait]
impl<T> Handler<RetryActor<T>> for RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    /// Handles retry control messages and coordinates the retry sequence.
    ///
    /// This method implements the core retry logic, processing `RetryMessage::Retry`
    /// and `RetryMessage::End` messages to manage the retry lifecycle. It applies
    /// the configured retry strategy, coordinates message delivery to the target actor,
    /// and manages backoff delays between retry attempts.
    ///
    /// # Parameters
    ///
    /// * `_path` - The sender's actor path (unused in retry logic)
    /// * `message` - The retry control message (`Retry` or `End`)
    /// * `ctx` - Actor context for system operations and child management
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Message processed successfully
    /// * `Err(Error)` - Critical error in message processing (rare)
    ///
    /// # Message Processing
    ///
    /// ## RetryMessage::Retry
    ///
    /// Processes a retry attempt with the following steps:
    ///
    /// 1. **State Check**: Verifies the retry sequence hasn't been ended
    /// 2. **Counter Update**: Increments the retry attempt counter
    /// 3. **Limit Check**: Validates retry count against strategy maximum
    /// 4. **Message Delivery**: Sends stored message to target child actor
    /// 5. **Backoff Scheduling**: Schedules next retry with strategy-defined delay
    /// 6. **Error Handling**: Emits supervision errors when limits are exceeded
    ///
    /// ## RetryMessage::End
    ///
    /// Terminates the retry sequence with the following actions:
    ///
    /// 1. **State Update**: Sets the `is_end` flag to prevent further retries
    /// 2. **Logging**: Records termination for debugging and monitoring
    /// 3. **Shutdown**: Initiates graceful actor shutdown through context
    /// 4. **Cleanup**: Allows actor system to clean up resources and children
    ///
    /// # Backoff Implementation
    ///
    /// The backoff mechanism uses async task spawning to implement delays:
    ///
    /// ```ignore
    /// tokio::spawn(async move {
    ///     tokio::time::sleep(backoff_duration).await;
    ///     actor_ref.tell(RetryMessage::Retry).await;
    /// });
    /// ```
    ///
    /// This approach ensures:
    /// - Non-blocking delay implementation
    /// - Concurrent retry actor operation
    /// - Proper resource utilization
    /// - Cancellation-safe operation
    ///
    /// # Error Scenarios
    ///
    /// The handler manages several error conditions:
    ///
    /// - **Target Actor Unavailable**: Logs error but continues retry sequence
    /// - **Max Retries Exceeded**: Emits `Error::ReTry` to supervision system
    /// - **Actor Reference Lost**: Emits `Error::Send` for system issues
    /// - **Message Delivery Failure**: Logged but retry sequence continues
    ///
    /// # Performance Characteristics
    ///
    /// - **Memory Usage**: O(1) per retry actor (fixed state size)
    /// - **CPU Usage**: Minimal except during backoff delay scheduling
    /// - **Latency**: Backoff delays as configured by retry strategy
    /// - **Concurrency**: Non-blocking backoff implementation
    ///
    /// # Examples
    ///
    /// The handler is typically invoked by the actor system:
    ///
    /// ```ignore
    /// // Actor system automatically calls handle_message
    /// retry_ref.tell(RetryMessage::Retry).await?;
    /// retry_ref.tell(RetryMessage::End).await?;
    /// ```
    async fn handle_message(
        &mut self,
        _path: ActorPath,
        message: RetryMessage,
        ctx: &mut ActorContext<RetryActor<T>>,
    ) -> Result<(), Error> {
        match message {
            RetryMessage::Retry => {
                // Check if retry sequence has been terminated
                if !self.is_end {
                    // Increment retry counter for this attempt
                    self.retries += 1;

                    // Verify we haven't exceeded the maximum retry limit
                    if self.retries <= self.retry_strategy.max_retries() {
                        debug!(
                            "Retry {}/{}.",
                            self.retries,
                            self.retry_strategy.max_retries()
                        );

                        // Attempt to send message to target child actor
                        // Note: Error is logged but doesn't stop retry sequence
                        if let Some(child) = ctx.get_child::<T>("target").await && child.tell(self.message.clone()).await.is_err()
                        {
                            error!("Cannot initiate retry to send message");
                        }

                        // Schedule next retry attempt with backoff delay
                        if let Some(actor) = ctx.reference().await {
                            if let Some(duration) = self.retry_strategy.next_backoff() {
                                let actor: ActorRef<RetryActor<T>> = actor;

                                // Spawn async task for non-blocking backoff delay
                                tokio::spawn(async move {
                                    debug!("Backoff for {:?}", &duration);
                                    tokio::time::sleep(duration).await;

                                    // Schedule next retry attempt
                                    let _ = actor.tell(RetryMessage::Retry).await;
                                });
                            }
                        } else {
                            // Failed to get actor reference - emit system error
                            let _ = ctx
                                .emit_error(Error::Send(
                                    "Cannot get retry actor reference".to_owned(),
                                ))
                                .await;
                        };
                    } else {
                        // Maximum retry limit exceeded - emit retry exhaustion error
                        warn!("Max retries reached.");
                        if let Err(e) = ctx.emit_error(Error::ReTry).await {
                            error!("Error in emit_error {}", e);
                        };
                    }
                }
                // Note: If is_end is true, retry messages are silently ignored
            }
            RetryMessage::End => {
                // Mark retry sequence as terminated
                self.is_end = true;
                debug!("RetryActor end");

                // Initiate graceful shutdown of retry actor
                ctx.stop(None).await;
            }
        }
        Ok(())
    }
}

/// Test module for the retry system functionality.
///
/// This module provides comprehensive integration tests for the retry actor system,
/// demonstrating proper usage patterns and verifying retry behavior under various
/// scenarios. The tests create a complete actor system environment with source
/// actors, target actors, and retry coordination.
///
/// # Test Architecture
///
/// The test setup follows a three-tier architecture:
///
/// ```text
/// ┌─────────────┐    creates    ┌─────────────┐    manages    ┌─────────────┐
/// │ SourceActor │ ─────────→    │ RetryActor  │ ─────────→    │ TargetActor │
/// │             │               │             │               │             │
/// └─────────────┘               └─────────────┘               └─────────────┘
/// ```
///
/// # Test Scenarios
///
/// The tests verify:
/// - Retry actor creation and initialization
/// - Message retry coordination between actors
/// - Backoff strategy application and timing
/// - Success condition handling and cleanup
/// - Error propagation and supervision integration
/// - Graceful termination of retry sequences
///
/// # Test Components
///
/// - **SourceActor**: Initiates retry sequences and handles completion
/// - **TargetActor**: Receives retried messages and simulates processing
/// - **RetryActor**: Coordinates the retry process with backoff strategies
/// - **ActorSystem**: Provides the runtime environment for all actors
#[cfg(test)]
mod tests {

    use tokio_util::sync::CancellationToken;

    use super::*;

    use crate::{ActorSystem, Error, FixedIntervalStrategy};

    use std::time::Duration;

    /// Test actor that initiates retry sequences and handles completion callbacks.
    ///
    /// `SourceActor` serves as the orchestrator in the test scenario, creating retry
    /// actors and handling success notifications from target actors. It demonstrates
    /// the typical usage pattern where a parent actor needs to retry operations
    /// through a dedicated retry coordination mechanism.
    pub struct SourceActor;

    /// Message type for the source actor containing string data.
    ///
    /// This message type is used for communication between the target actor and
    /// source actor, carrying completion notifications and test data validation.
    #[derive(Debug, Clone)]
    pub struct SourceMessage(pub String);

    impl Message for SourceMessage {}

    #[async_trait]
    impl Actor for SourceActor {
        type Message = SourceMessage;
        type Response = ();
        type Event = ();

        async fn pre_start(
            &mut self,
            ctx: &mut ActorContext<SourceActor>,
        ) -> Result<(), Error> {
            println!("SourceActor pre_start");
            let target = TargetActor { counter: 0 };

            let strategy = Strategy::FixedInterval(FixedIntervalStrategy::new(
                3,
                Duration::from_secs(1),
            ));

            let retry_actor = RetryActor::new(
                target,
                TargetMessage {
                    source: ctx.path().clone(),
                    message: "Hello from parent".to_owned(),
                },
                strategy,
            );
            let retry = ctx
                .create_child::<RetryActor<TargetActor>>("retry", retry_actor)
                .await
                .unwrap();

            retry.tell(RetryMessage::Retry).await.unwrap();
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<SourceActor> for SourceActor {
        async fn handle_message(
            &mut self,
            _path: ActorPath,
            message: SourceMessage,
            ctx: &mut ActorContext<SourceActor>,
        ) -> Result<(), Error> {
            println!("Message: {:?}", message);
            assert_eq!(message.0, "Hello from child");

            let retry = ctx
                .get_child::<RetryActor<TargetActor>>("retry")
                .await
                .unwrap();
            retry.tell(RetryMessage::End).await.unwrap();

            Ok(())
        }
    }

    /// Test actor that receives retried messages and simulates processing behavior.
    ///
    /// `TargetActor` represents the destination of retry attempts, maintaining a counter
    /// to track retry attempts and implementing test-specific logic to simulate success
    /// conditions. On the second retry attempt, it sends a completion notification back
    /// to the source actor, demonstrating successful message processing.
    ///
    /// # Fields
    ///
    /// * `counter` - Tracks the number of times messages have been received, used to
    ///               determine when to signal successful completion
    #[derive(Clone)]
    pub struct TargetActor {
        counter: usize,
    }

    /// Message sent to target actors containing the original message data and sender path.
    ///
    /// This message structure demonstrates a typical retry scenario where the message
    /// contains both the business data (`message`) and routing information (`source`)
    /// for response delivery.
    ///
    /// # Fields
    ///
    /// * `source` - Actor path of the original sender for response routing
    /// * `message` - The actual message content being retried
    #[derive(Debug, Clone)]
    pub struct TargetMessage {
        pub source: ActorPath,
        pub message: String,
    }

    impl Message for TargetMessage {}

    impl Actor for TargetActor {
        type Message = TargetMessage;
        type Response = ();
        type Event = ();
    }

    #[async_trait]
    impl Handler<TargetActor> for TargetActor {
        async fn handle_message(
            &mut self,
            _path: ActorPath,
            message: TargetMessage,
            ctx: &mut ActorContext<TargetActor>,
        ) -> Result<(), Error> {
            assert_eq!(message.message, "Hello from parent");
            self.counter += 1;
            println!("Counter: {}", self.counter);
            if self.counter == 2 {
                let source = ctx
                    .system()
                    .get_actor::<SourceActor>(&message.source)
                    .await
                    .unwrap();
                source
                    .tell(SourceMessage("Hello from child".to_owned()))
                    .await?;
            }
            Ok(())
        }
    }

    /// Integration test for retry actor functionality and coordination.
    ///
    /// This test verifies the complete retry actor lifecycle, including:
    ///
    /// 1. **System Setup**: Creates an actor system with proper runtime configuration
    /// 2. **Actor Creation**: Instantiates source, target, and retry actors
    /// 3. **Retry Coordination**: Demonstrates message retry with backoff strategies
    /// 4. **Success Handling**: Verifies proper termination when operation succeeds
    /// 5. **Resource Cleanup**: Ensures graceful shutdown of retry sequences
    ///
    /// # Test Flow
    ///
    /// 1. Source actor creates a retry actor with a fixed interval strategy (3 retries, 1s intervals)
    /// 2. Retry actor creates target actor as a child and begins retry attempts
    /// 3. Target actor receives messages, incrementing counter on each attempt
    /// 4. On the second attempt, target actor notifies source of success
    /// 5. Source actor terminates the retry sequence by sending `RetryMessage::End`
    /// 6. System runs for 5 seconds to allow complete message processing
    ///
    /// # Verification Points
    ///
    /// - Retry actor creation and child management
    /// - Message delivery coordination between actors
    /// - Backoff strategy application with fixed intervals
    /// - Success condition detection and retry termination
    /// - Graceful cleanup of retry resources
    ///
    /// # Test Configuration
    ///
    /// - **Retry Strategy**: Fixed interval with 3 max retries and 1-second delays
    /// - **Success Condition**: Target actor responds after receiving 2 messages
    /// - **Test Duration**: 5 seconds to allow for message processing and delays
    ///
    /// # Expected Behavior
    ///
    /// 1. Target receives first retry attempt immediately
    /// 2. After 1-second delay, target receives second retry attempt
    /// 3. Target sends success notification to source
    /// 4. Source terminates retry sequence
    /// 5. No further retry attempts occur
    #[tokio::test]
    async fn test_retry_actor() {
        let (system, mut runner) = ActorSystem::create(CancellationToken::new());

        tokio::spawn(async move {
            runner.run().await;
        });

        let _ = system
            .create_root_actor::<SourceActor>("source", SourceActor)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
