// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor Supervision System
//!
//! This module provides a comprehensive supervision strategy framework for managing actor failures
//! and recovery within the actor system. Supervision strategies define how the system responds
//! to actor startup failures, enabling fault-tolerant operation through configurable retry
//! mechanisms and backoff policies.
//!
//! The supervision system follows the "let it crash" philosophy common in actor systems,
//! where failures are expected and handled through well-defined recovery strategies rather
//! than defensive programming. When an actor fails during startup, the supervisor can either
//! stop the failed actor permanently or attempt to restart it according to a specified
//! retry strategy.
//!
//! # Core Concepts
//!
//! ## Supervision Strategies
//!
//! - **Stop**: Immediately terminate a failed actor without retry attempts
//! - **Retry**: Attempt to restart failed actors using configurable retry strategies
//!
//! ## Retry Strategies
//!
//! - **NoInterval**: Immediate retry without delay (for fast-failing scenarios)
//! - **FixedInterval**: Retry with consistent delay between attempts
//! - **CustomInterval**: Retry with custom delay sequences (e.g., exponential backoff)
//!
//! # Usage Examples
//!
//! ## Basic Supervision Setup
//!
//! ```ignore
//! use actor::supervision::{SupervisionStrategy, Strategy, FixedIntervalStrategy};
//! use std::time::Duration;
//!
//! // Stop immediately on failure
//! let stop_strategy = SupervisionStrategy::Stop;
//!
//! // Retry with fixed 1-second intervals, maximum 3 attempts
//! let retry_strategy = SupervisionStrategy::Retry(
//!     Strategy::FixedInterval(
//!         FixedIntervalStrategy::new(3, Duration::from_secs(1))
//!     )
//! );
//! ```
//!
//! ## Custom Backoff Strategies
//!
//! ```ignore
//! use actor::supervision::{Strategy, CustomIntervalStrategy};
//! use std::collections::VecDeque;
//! use std::time::Duration;
//!
//! // Exponential backoff: 1s, 2s, 4s, 8s
//! let backoff_intervals = VecDeque::from([
//!     Duration::from_secs(1),
//!     Duration::from_secs(2),
//!     Duration::from_secs(4),
//!     Duration::from_secs(8),
//! ]);
//!
//! let exponential_strategy = Strategy::CustomIntervalStrategy(
//!     CustomIntervalStrategy::new(backoff_intervals)
//! );
//! ```
//!
//! # Error Handling Philosophy
//!
//! The supervision system is designed around several key principles:
//!
//! - **Isolation**: Actor failures are contained and don't cascade to siblings
//! - **Transparency**: Failed actors can be restarted without affecting their public interface
//! - **Configurability**: Different failure scenarios require different recovery strategies
//! - **Observability**: Supervision decisions can be monitored and adjusted based on system behavior
//!

use std::{collections::VecDeque, fmt::Debug, time::Duration};

/// Defines the behavior of retry strategies for actor supervision.
///
/// This trait provides the interface for implementing custom retry strategies that determine
/// how many times an actor should be retried and what delays should be applied between
/// retry attempts. Implementors can create sophisticated backoff policies, circuit breaker
/// patterns, or adaptive retry mechanisms based on failure patterns.
///
/// # Implementation Requirements
///
/// - Must be thread-safe (`Send + Sync`) for use across actor system threads
/// - Must be debuggable for monitoring and troubleshooting
/// - Should track internal state to provide varying backoff intervals
///
/// # Design Patterns
///
/// Common retry strategy patterns include:
/// - **Fixed Interval**: Constant delay between retries
/// - **Exponential Backoff**: Increasing delays to reduce system load under failure
/// - **Linear Backoff**: Gradually increasing delays for predictable recovery
/// - **Jittered Backoff**: Random variation to prevent thundering herd problems
/// - **Adaptive**: Backoff adjustment based on historical success rates
///
/// # Examples
///
/// ## Implementing a Custom Exponential Backoff Strategy
///
/// ```ignore
/// use actor::supervision::RetryStrategy;
/// use std::time::Duration;
///
/// #[derive(Debug)]
/// struct ExponentialBackoff {
///     max_retries: usize,
///     base_delay: Duration,
///     current_attempt: usize,
/// }
///
/// impl ExponentialBackoff {
///     fn new(max_retries: usize, base_delay: Duration) -> Self {
///         Self {
///             max_retries,
///             base_delay,
///             current_attempt: 0,
///         }
///     }
/// }
///
/// impl RetryStrategy for ExponentialBackoff {
///     fn max_retries(&self) -> usize {
///         self.max_retries
///     }
///
///     fn next_backoff(&mut self) -> Option<Duration> {
///         if self.current_attempt >= self.max_retries {
///             return None;
///         }
///
///         let delay = self.base_delay * 2_u32.pow(self.current_attempt as u32);
///         self.current_attempt += 1;
///         Some(delay)
///     }
/// }
/// ```
pub trait RetryStrategy: Debug + Send + Sync {
    /// Returns the maximum number of retry attempts before permanently failing an actor.
    ///
    /// This method defines the upper bound on retry attempts for a failed actor. Once this
    /// limit is reached, the supervision system will stop attempting to restart the actor
    /// and mark it as permanently failed. The value should be chosen based on:
    ///
    /// - **System criticality**: Critical actors may warrant more retry attempts
    /// - **Failure patterns**: Transient failures may benefit from more retries
    /// - **Resource constraints**: Each retry consumes system resources
    /// - **Recovery time**: Total recovery time is attempts × average backoff duration
    ///
    /// # Returns
    ///
    /// The maximum number of retry attempts as a `usize`. A value of 0 means no retries
    /// will be attempted (equivalent to `SupervisionStrategy::Stop`).
    fn max_retries(&self) -> usize;

    /// Returns the next backoff duration before the next retry attempt.
    ///
    /// This method is called before each retry attempt to determine how long the system
    /// should wait before attempting to restart the failed actor. The implementation
    /// can return different durations for each call to implement various backoff strategies.
    ///
    /// Returning `None` indicates that no more retries should be attempted, effectively
    /// ending the retry sequence early. This can be useful for adaptive strategies that
    /// give up based on failure patterns or external conditions.
    ///
    /// # State Management
    ///
    /// Implementations should track internal state to provide appropriate backoff intervals:
    /// - **Attempt counter**: Track current retry attempt for exponential backoff
    /// - **Previous durations**: Remember past intervals for adaptive strategies
    /// - **Failure context**: Consider failure types for strategy adjustment
    ///
    /// # Returns
    ///
    /// - `Some(Duration)`: Wait for the specified duration before retrying
    /// - `None`: Stop retrying and mark the actor as permanently failed
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Fixed 100ms delay
    /// fn next_backoff(&mut self) -> Option<Duration> {
    ///     Some(Duration::from_millis(100))
    /// }
    ///
    /// // Exponential backoff with jitter
    /// fn next_backoff(&mut self) -> Option<Duration> {
    ///     if self.attempts >= self.max_retries {
    ///         return None;
    ///     }
    ///
    ///     let base_delay = Duration::from_millis(100);
    ///     let exponential_delay = base_delay * 2_u32.pow(self.attempts);
    ///     let jitter = Duration::from_millis(rand::random::<u64>() % 50);
    ///
    ///     self.attempts += 1;
    ///     Some(exponential_delay + jitter)
    /// }
    /// ```
    fn next_backoff(&mut self) -> Option<Duration>;
}

/// Defines the high-level supervision strategy for handling actor startup failures.
///
/// This enum represents the fundamental decision point in the supervision system: whether
/// to give up immediately when an actor fails to start, or to attempt recovery through
/// a retry mechanism. The choice between strategies depends on the nature of the application,
/// the criticality of the failing actor, and the expected failure patterns.
///
/// # Strategy Selection Guidelines
///
/// ## Use `Stop` when:
/// - Actor failures indicate permanent configuration or code issues
/// - System resources are constrained and retries would be wasteful
/// - Fast failure detection is more important than fault tolerance
/// - The actor is non-critical and its absence won't impact system operation
/// - Manual intervention is required to fix the underlying issue
///
/// ## Use `Retry` when:
/// - Failures are likely transient (network issues, resource contention)
/// - The actor provides critical system functionality
/// - External dependencies may be temporarily unavailable
/// - System load balancing benefits from automatic recovery
/// - Recovery time is acceptable for the use case
///
/// # Failure Scope
///
/// Currently, supervision strategies apply only to **startup failures**. Runtime failures
/// during message processing follow different error handling patterns within the actor's
/// message handlers. This design separates initialization concerns from operational concerns.
///
/// # Examples
///
/// ## Immediate Termination Strategy
///
/// ```ignore
/// use actor::supervision::SupervisionStrategy;
///
/// // Stop immediately on any startup failure
/// let strategy = SupervisionStrategy::Stop;
/// ```
///
/// ## Retry with Exponential Backoff
///
/// ```ignore
/// use actor::supervision::{SupervisionStrategy, Strategy, FixedIntervalStrategy};
/// use std::time::Duration;
///
/// // Retry up to 5 times with 2-second intervals
/// let strategy = SupervisionStrategy::Retry(
///     Strategy::FixedInterval(
///         FixedIntervalStrategy::new(5, Duration::from_secs(2))
///     )
/// );
/// ```
///
/// # Performance Considerations
///
/// - **Stop**: Zero overhead, immediate failure detection
/// - **Retry**: Overhead proportional to retry attempts and backoff durations
/// - Choose retry strategies that balance fault tolerance with system responsiveness
#[derive(Debug, Clone)]
pub enum SupervisionStrategy {
    /// Immediately terminate the actor on startup failure without retry attempts.
    ///
    /// This strategy provides the fastest failure detection and recovery by immediately
    /// giving up on failed actors. It's appropriate for scenarios where failures indicate
    /// permanent issues that won't be resolved by retrying, such as configuration errors,
    /// missing resources, or programming bugs.
    ///
    /// Use this strategy when:
    /// - Fast failure detection is critical
    /// - Manual intervention is required to fix failures
    /// - System resources are limited
    /// - The actor is non-critical to system operation
    Stop,
    /// Attempt to restart the actor using the specified retry strategy.
    ///
    /// This strategy enables fault-tolerant operation by automatically attempting to
    /// recover from startup failures. The retry behavior is determined by the wrapped
    /// `Strategy` instance, which defines the number of attempts, backoff intervals,
    /// and termination conditions.
    ///
    /// Use this strategy when:
    /// - Failures are likely transient
    /// - The actor provides critical functionality
    /// - Automatic recovery is preferred over manual intervention
    /// - System availability is prioritized over immediate failure detection
    Retry(Strategy),
}

/// Concrete retry strategy implementations for actor supervision.
///
/// This enum provides three built-in retry strategies that cover common backoff patterns
/// used in distributed systems. Each strategy implements the `RetryStrategy` trait and
/// can be used within a `SupervisionStrategy::Retry` to define specific retry behavior.
///
/// # Available Strategies
///
/// - **NoInterval**: Immediate retries without delay, suitable for fast-failing scenarios
/// - **FixedInterval**: Consistent delay between retries, predictable recovery timing
/// - **CustomInterval**: Flexible delay sequences, enables complex backoff patterns
///
/// # Strategy Selection
///
/// Choose retry strategies based on failure characteristics and system requirements:
///
/// ## NoInterval Strategy
/// - **Use for**: Resource initialization failures, lock contention, race conditions
/// - **Avoid for**: Network failures, external service dependencies
/// - **Trade-offs**: Fast recovery vs. potential system overload
///
/// ## FixedInterval Strategy
/// - **Use for**: Service dependencies with predictable recovery times
/// - **Avoid for**: Cascading failures, thundering herd scenarios
/// - **Trade-offs**: Predictable timing vs. lack of adaptability
///
/// ## CustomInterval Strategy
/// - **Use for**: Complex failure patterns, exponential backoff, jittered delays
/// - **Avoid for**: Simple scenarios where fixed intervals suffice
/// - **Trade-offs**: Maximum flexibility vs. configuration complexity
///
/// # Examples
///
/// ```ignore
/// use actor::supervision::Strategy;
/// use std::collections::VecDeque;
/// use std::time::Duration;
///
/// // Immediate retries for fast recovery
/// let immediate = Strategy::NoInterval(
///     NoIntervalStrategy::new(3)
/// );
///
/// // Fixed 500ms intervals for consistent timing
/// let fixed = Strategy::FixedInterval(
///     FixedIntervalStrategy::new(5, Duration::from_millis(500))
/// );
///
/// // Exponential backoff: 100ms, 200ms, 400ms, 800ms
/// let exponential = Strategy::CustomIntervalStrategy(
///     CustomIntervalStrategy::new(VecDeque::from([
///         Duration::from_millis(100),
///         Duration::from_millis(200),
///         Duration::from_millis(400),
///         Duration::from_millis(800),
///     ]))
/// );
/// ```
#[derive(Debug, Clone)]
pub enum Strategy {
    NoInterval(NoIntervalStrategy),
    FixedInterval(FixedIntervalStrategy),
    CustomIntervalStrategy(CustomIntervalStrategy),
}

/// Implements the `RetryStrategy` trait for the `Strategy` enum.
///
/// This implementation delegates retry strategy behavior to the appropriate concrete
/// strategy variant, enabling polymorphic use of different retry strategies through
/// a common interface. The delegation pattern ensures that each strategy type maintains
/// its specific behavior while providing a unified API.
///
/// # Method Delegation
///
/// All `RetryStrategy` methods are forwarded to the wrapped strategy implementation:
/// - `max_retries()` returns the retry limit for the specific strategy
/// - `next_backoff()` returns the next delay interval from the specific strategy
///
/// This design allows strategy instances to maintain their internal state (such as
/// attempt counters or remaining intervals) while being used polymorphically.
impl RetryStrategy for Strategy {
    fn max_retries(&self) -> usize {
        match self {
            Strategy::NoInterval(strategy) => strategy.max_retries(),
            Strategy::FixedInterval(strategy) => strategy.max_retries(),
            Strategy::CustomIntervalStrategy(strategy) => {
                strategy.max_retries()
            }
        }
    }

    fn next_backoff(&mut self) -> Option<Duration> {
        match self {
            Strategy::NoInterval(strategy) => strategy.next_backoff(),
            Strategy::FixedInterval(strategy) => strategy.next_backoff(),
            Strategy::CustomIntervalStrategy(strategy) => {
                strategy.next_backoff()
            }
        }
    }
}

/// Provides a sensible default retry strategy for actor supervision.
///
/// The default strategy uses `NoIntervalStrategy` with zero maximum retries, effectively
/// providing no retry behavior. This conservative default ensures that actors fail fast
/// by default, requiring explicit configuration for retry behavior.
///
/// This design follows the principle of "secure by default" where potentially resource-
/// intensive operations (like retries) must be explicitly enabled rather than being
/// automatically applied.
///
/// # Default Behavior
///
/// ```ignore
/// use actor::supervision::Strategy;
///
/// let default_strategy = Strategy::default();
/// // Equivalent to:
/// // Strategy::NoInterval(NoIntervalStrategy::new(0))
/// ```
impl Default for Strategy {
    fn default() -> Self {
        Strategy::NoInterval(NoIntervalStrategy::default())
    }
}

/// A retry strategy that immediately retries failed actors without any delay.
///
/// This strategy is designed for scenarios where failures are expected to be quickly
/// resolved, such as resource contention, initialization race conditions, or temporary
/// lock conflicts. By eliminating delay between retry attempts, it enables the fastest
/// possible recovery when the underlying issue resolves quickly.
///
/// # Use Cases
///
/// ## Ideal Scenarios
/// - **Resource contention**: Multiple actors competing for shared resources
/// - **Initialization races**: Dependent services starting in uncertain order
/// - **Lock conflicts**: Brief contention for exclusive resources
/// - **Memory pressure**: Temporary allocation failures
///
/// ## Avoid When
/// - **Network failures**: Immediate retries can overwhelm failing services
/// - **External dependencies**: Services that need time to recover
/// - **Resource exhaustion**: System-wide resource shortages
/// - **Cascading failures**: Failures that propagate through system components
///
/// # Performance Characteristics
///
/// - **Recovery Time**: Minimal - limited only by actor startup time
/// - **Resource Usage**: High during retry bursts, minimal backoff overhead
/// - **System Load**: Can create retry storms under widespread failures
/// - **Failure Detection**: Fastest possible detection of permanent failures
///
/// # Examples
///
/// ```ignore
/// use actor::supervision::NoIntervalStrategy;
///
/// // Retry up to 3 times immediately
/// let strategy = NoIntervalStrategy::new(3);
///
/// // Default: no retries (fail fast)
/// let default_strategy = NoIntervalStrategy::default();
/// ```
///
/// # Implementation Notes
///
/// The strategy tracks only the maximum retry count and always returns `None` for
/// backoff duration, indicating that no delay should be applied between attempts.
/// The actual retry counting is handled by the supervision system.
#[derive(Debug, Default, Clone)]
pub struct NoIntervalStrategy {
    max_retries: usize,
}

impl NoIntervalStrategy {
    /// Creates a new no-interval retry strategy with the specified maximum retry count.
    ///
    /// # Parameters
    ///
    /// * `max_retries` - The maximum number of retry attempts before giving up.
    ///   Setting this to 0 effectively disables retries.
    ///
    /// # Returns
    ///
    /// A new `NoIntervalStrategy` instance configured with the specified retry limit.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use actor::supervision::NoIntervalStrategy;
    ///
    /// // Allow up to 5 immediate retry attempts
    /// let strategy = NoIntervalStrategy::new(5);
    ///
    /// // No retries - fail immediately
    /// let no_retry = NoIntervalStrategy::new(0);
    /// ```
    pub fn new(max_retries: usize) -> Self {
        NoIntervalStrategy { max_retries }
    }
}

/// Implementation of `RetryStrategy` for immediate retry behavior.
///
/// This implementation provides the core behavior for no-interval retries:
/// - Returns the configured maximum retry count
/// - Always returns `None` for backoff duration to indicate no delay
///
/// The `None` return from `next_backoff()` is the key characteristic that makes
/// this an "immediate" retry strategy - the supervision system interprets this
/// as "proceed with the next retry attempt without delay".
impl RetryStrategy for NoIntervalStrategy {
    /// Returns the maximum number of retry attempts configured for this strategy.
    ///
    /// # Returns
    ///
    /// The maximum retry count as specified during strategy creation.
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    /// Returns no backoff duration, indicating immediate retry attempts.
    ///
    /// This method always returns `None` to signal that no delay should be
    /// applied between retry attempts. The supervision system interprets this
    /// as a directive to immediately attempt the next retry.
    ///
    /// # Returns
    ///
    /// Always returns `None` to indicate no delay between retry attempts.
    fn next_backoff(&mut self) -> Option<Duration> {
        None
    }
}

/// A retry strategy that applies a consistent delay between retry attempts.
///
/// This strategy provides predictable retry timing by using the same delay duration
/// for all retry attempts. It's ideal for scenarios where the underlying failure
/// cause has a known or predictable recovery time, such as waiting for external
/// services to restart or for temporary resource shortages to resolve.
///
/// # Characteristics
///
/// - **Predictable timing**: Each retry occurs at exactly the same interval
/// - **Simple configuration**: Only requires retry count and delay duration
/// - **Consistent load**: Generates predictable load patterns on the system
/// - **No adaptation**: Doesn't adjust based on failure patterns or success rates
///
/// # Use Cases
///
/// ## Ideal Scenarios
/// - **Service dependencies**: Waiting for external services with known restart times
/// - **Resource cleanup**: Allowing time for resource cleanup between attempts
/// - **Rate limiting**: Respecting API rate limits with consistent spacing
/// - **Batch processing**: Regular retry intervals for batch job failures
///
/// ## Consider Alternatives When
/// - **Cascading failures**: Fixed intervals can create thundering herd problems
/// - **Variable recovery**: Some failures need more/less time than others
/// - **High contention**: Multiple actors retrying simultaneously
/// - **Exponential failure**: Failures that get progressively worse
///
/// # Performance Trade-offs
///
/// **Advantages:**
/// - Predictable resource usage patterns
/// - Simple to reason about and configure
/// - Consistent system load distribution
/// - No complex backoff calculations
///
/// **Disadvantages:**
/// - No adaptation to failure patterns
/// - May retry too quickly for slow-recovering failures
/// - Can contribute to thundering herd problems
/// - Fixed cost regardless of failure severity
///
/// # Examples
///
/// ```ignore
/// use actor::supervision::FixedIntervalStrategy;
/// use std::time::Duration;
///
/// // Retry every 1 second, up to 5 times
/// let strategy = FixedIntervalStrategy::new(5, Duration::from_secs(1));
///
/// // Quick retries for fast-recovering failures
/// let quick = FixedIntervalStrategy::new(3, Duration::from_millis(100));
///
/// // Longer intervals for external service dependencies
/// let patient = FixedIntervalStrategy::new(10, Duration::from_secs(30));
/// ```
#[derive(Debug, Default, Clone)]
pub struct FixedIntervalStrategy {
    /// Maximum number of retries before permanently failing an actor.
    ///
    /// This field defines the upper bound on retry attempts. Once this limit is reached,
    /// the supervision system will stop attempting to restart the actor and mark it as
    /// permanently failed.
    max_retries: usize,

    /// Wait duration before each retry attempt.
    ///
    /// This duration is applied consistently between all retry attempts, providing
    /// predictable timing behavior. The same delay is used regardless of which
    /// retry attempt is being made or what caused the previous failure.
    duration: Duration,
}

impl FixedIntervalStrategy {
    /// Creates a new fixed interval retry strategy with specified parameters.
    ///
    /// # Parameters
    ///
    /// * `max_retries` - The maximum number of retry attempts before giving up.
    ///   Setting this to 0 effectively disables retries.
    /// * `duration` - The consistent delay to apply between each retry attempt.
    ///   This duration is used for all retry attempts.
    ///
    /// # Returns
    ///
    /// A new `FixedIntervalStrategy` instance with the specified configuration.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use actor::supervision::FixedIntervalStrategy;
    /// use std::time::Duration;
    ///
    /// // Retry up to 3 times with 2-second intervals
    /// let strategy = FixedIntervalStrategy::new(3, Duration::from_secs(2));
    ///
    /// // Quick retries for transient failures
    /// let quick = FixedIntervalStrategy::new(5, Duration::from_millis(250));
    ///
    /// // Patient retries for slow-recovering dependencies
    /// let patient = FixedIntervalStrategy::new(2, Duration::from_secs(60));
    /// ```
    ///
    /// # Configuration Guidelines
    ///
    /// **Retry Count:**
    /// - Low (1-3): For critical failures that should be resolved quickly
    /// - Medium (4-10): For transient failures with reasonable recovery time
    /// - High (>10): For non-critical actors or very reliable recovery patterns
    ///
    /// **Interval Duration:**
    /// - Short (<1s): For resource contention or fast-recovering failures
    /// - Medium (1-30s): For service dependencies or moderate recovery scenarios
    /// - Long (>30s): For external systems or batch processing scenarios
    pub fn new(max_retries: usize, duration: Duration) -> Self {
        FixedIntervalStrategy {
            max_retries,
            duration,
        }
    }
}

/// Implementation of `RetryStrategy` for fixed interval retry behavior.
///
/// This implementation provides consistent retry behavior by:
/// - Returning the configured maximum retry count
/// - Always returning the same duration for backoff delays
///
/// The consistent `Some(duration)` return from `next_backoff()` is what makes
/// this a "fixed interval" strategy - every retry attempt uses the same delay.
impl RetryStrategy for FixedIntervalStrategy {
    /// Returns the maximum number of retry attempts configured for this strategy.
    ///
    /// # Returns
    ///
    /// The maximum retry count as specified during strategy creation.
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    /// Returns the fixed backoff duration for the next retry attempt.
    ///
    /// This method always returns the same duration that was specified when the
    /// strategy was created, providing consistent timing between all retry attempts.
    /// The duration is wrapped in `Some()` to indicate that a delay should be applied.
    ///
    /// # Returns
    ///
    /// Always returns `Some(duration)` where `duration` is the fixed interval
    /// specified during strategy creation.
    fn next_backoff(&mut self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// A retry strategy that uses a custom sequence of delay intervals.
///
/// This strategy provides maximum flexibility by allowing you to specify exactly
/// what delay should be used for each retry attempt. It's ideal for implementing
/// sophisticated backoff patterns like exponential backoff, fibonacci sequences,
/// jittered delays, or any other custom timing strategy.
///
/// # Behavior
///
/// The strategy maintains a queue of duration values that are consumed in order
/// for each retry attempt. Once all durations are consumed, no further retries
/// will be attempted. This design allows for complete control over the retry
/// timing pattern while automatically limiting the total number of attempts.
///
/// # Use Cases
///
/// ## Exponential Backoff
/// ```ignore
/// use actor::supervision::CustomIntervalStrategy;
/// use std::collections::VecDeque;
/// use std::time::Duration;
///
/// // 100ms, 200ms, 400ms, 800ms, 1.6s
/// let exponential = CustomIntervalStrategy::new(VecDeque::from([
///     Duration::from_millis(100),
///     Duration::from_millis(200),
///     Duration::from_millis(400),
///     Duration::from_millis(800),
///     Duration::from_millis(1600),
/// ]));
/// ```
///
/// ## Fibonacci Backoff
/// ```ignore
/// // 1s, 1s, 2s, 3s, 5s, 8s
/// let fibonacci = CustomIntervalStrategy::new(VecDeque::from([
///     Duration::from_secs(1),
///     Duration::from_secs(1),
///     Duration::from_secs(2),
///     Duration::from_secs(3),
///     Duration::from_secs(5),
///     Duration::from_secs(8),
/// ]));
/// ```
///
/// ## Jittered Delays
/// ```ignore
/// // Random jitter applied to base intervals
/// let jittered = CustomIntervalStrategy::new(VecDeque::from([
///     Duration::from_millis(100 + rand::random::<u64>() % 50),
///     Duration::from_millis(200 + rand::random::<u64>() % 100),
///     Duration::from_millis(400 + rand::random::<u64>() % 200),
/// ]));
/// ```
///
/// ## Decreasing Intervals
/// ```ignore
/// // Start with longer delays, decrease over time
/// let decreasing = CustomIntervalStrategy::new(VecDeque::from([
///     Duration::from_secs(10),  // First retry: wait longer
///     Duration::from_secs(5),   // Second retry: medium wait
///     Duration::from_secs(1),   // Final retries: quick attempts
///     Duration::from_secs(1),
/// ]));
/// ```
///
/// # Performance Characteristics
///
/// - **Memory usage**: O(n) where n is the number of retry attempts
/// - **Time complexity**: O(1) for each backoff calculation
/// - **Flexibility**: Complete control over retry timing patterns
/// - **Predictability**: Exact behavior can be pre-calculated
///
/// # Design Patterns
///
/// ## Circuit Breaker Pattern
/// Implement increasing delays to give failing systems time to recover:
/// ```ignore
/// let circuit_breaker = CustomIntervalStrategy::new(VecDeque::from([
///     Duration::from_secs(1),    // Quick first retry
///     Duration::from_secs(5),    // Give it some time
///     Duration::from_secs(30),   // Longer pause
///     Duration::from_secs(300),  // Much longer pause
/// ]));
/// ```
///
/// ## Adaptive Strategy
/// Use external metrics to determine appropriate delays:
/// ```ignore
/// fn create_adaptive_strategy(system_load: f64) -> CustomIntervalStrategy {
///     let base_delay = Duration::from_millis((100.0 * system_load) as u64);
///     CustomIntervalStrategy::new(VecDeque::from([
///         base_delay,
///         base_delay * 2,
///         base_delay * 4,
///     ]))
/// }
/// ```
#[derive(Debug, Default, Clone)]
pub struct CustomIntervalStrategy {
    /// Queue of duration values to be used for retry attempts.
    ///
    /// Each call to `next_backoff()` removes and returns the front duration.
    /// When the queue becomes empty, no further retries will be attempted.
    /// This design provides precise control over retry timing while automatically
    /// limiting the total number of attempts.
    durations: VecDeque<Duration>,

    /// Maximum number of retries, derived from the initial duration queue length.
    ///
    /// This value is set to the length of the durations queue when the strategy
    /// is created. It represents the maximum possible number of retry attempts,
    /// though the actual number may be fewer if `next_backoff()` returns `None`
    /// before all durations are consumed.
    max_retries: usize,
}

impl CustomIntervalStrategy {
    /// Creates a new custom interval retry strategy with the specified duration sequence.
    ///
    /// # Parameters
    ///
    /// * `durations` - A queue of duration values that will be used in order for
    ///   each retry attempt. The length of this queue determines the maximum number
    ///   of retry attempts. An empty queue means no retries will be attempted.
    ///
    /// # Returns
    ///
    /// A new `CustomIntervalStrategy` instance that will use the provided durations
    /// in sequence for retry attempts.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use actor::supervision::CustomIntervalStrategy;
    /// use std::collections::VecDeque;
    /// use std::time::Duration;
    ///
    /// // Exponential backoff: 1s, 2s, 4s, 8s
    /// let exponential = CustomIntervalStrategy::new(VecDeque::from([
    ///     Duration::from_secs(1),
    ///     Duration::from_secs(2),
    ///     Duration::from_secs(4),
    ///     Duration::from_secs(8),
    /// ]));
    ///
    /// // Linear backoff: 1s, 2s, 3s, 4s, 5s
    /// let linear = CustomIntervalStrategy::new(VecDeque::from([
    ///     Duration::from_secs(1),
    ///     Duration::from_secs(2),
    ///     Duration::from_secs(3),
    ///     Duration::from_secs(4),
    ///     Duration::from_secs(5),
    /// ]));
    ///
    /// // Custom pattern: short, long, short
    /// let custom = CustomIntervalStrategy::new(VecDeque::from([
    ///     Duration::from_millis(100),  // Quick first retry
    ///     Duration::from_secs(30),     // Long wait
    ///     Duration::from_millis(500),  // Quick final retry
    /// ]));
    ///
    /// // No retries
    /// let no_retry = CustomIntervalStrategy::new(VecDeque::new());
    /// ```
    ///
    /// # Design Considerations
    ///
    /// **Duration Sequence Design:**
    /// - Start with shorter intervals for transient failures
    /// - Increase intervals for persistent failures to reduce system load
    /// - Consider jitter to avoid thundering herd problems
    /// - Balance total recovery time with retry frequency
    ///
    /// **Queue Length:**
    /// - Shorter queues (1-5 entries): For critical failures requiring quick resolution
    /// - Medium queues (5-15 entries): For typical transient failures
    /// - Longer queues (>15 entries): For non-critical actors or very reliable patterns
    pub fn new(durations: VecDeque<Duration>) -> Self {
        Self {
            durations: durations.clone(),
            max_retries: durations.len(),
        }
    }
}

/// Implementation of `RetryStrategy` for custom interval retry behavior.
///
/// This implementation provides flexible retry behavior by:
/// - Returning the total number of durations as the maximum retry count
/// - Consuming durations from the queue in order for each backoff request
/// - Automatically terminating retries when the duration queue is empty
///
/// The `pop_front()` behavior is key to this strategy's functionality - it ensures
/// that each retry attempt uses the next duration in the sequence, and naturally
/// terminates the retry sequence when all durations have been consumed.
impl RetryStrategy for CustomIntervalStrategy {
    /// Returns the maximum number of retry attempts based on the duration queue length.
    ///
    /// This value represents the number of durations provided when the strategy was
    /// created. It may not reflect the current state of the duration queue if
    /// `next_backoff()` has been called, as durations are consumed during use.
    ///
    /// # Returns
    ///
    /// The original length of the duration queue as specified during strategy creation.
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    /// Returns the next backoff duration by consuming it from the duration queue.
    ///
    /// This method removes and returns the first duration from the internal queue.
    /// Once all durations have been consumed, it returns `None` to indicate that
    /// no further retry attempts should be made. This design provides precise
    /// control over the retry sequence while automatically terminating retries.
    ///
    /// # State Mutation
    ///
    /// Each call to this method modifies the internal state by removing one duration
    /// from the queue. This means:
    /// - The method can only be called successfully a limited number of times
    /// - The behavior changes based on how many times it has been called previously
    /// - The strategy "remembers" its position in the retry sequence
    ///
    /// # Returns
    ///
    /// - `Some(Duration)`: The next duration in the sequence to wait before retrying
    /// - `None`: No more durations available, stop retrying
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use actor::supervision::{CustomIntervalStrategy, RetryStrategy};
    /// use std::collections::VecDeque;
    /// use std::time::Duration;
    ///
    /// let mut strategy = CustomIntervalStrategy::new(VecDeque::from([
    ///     Duration::from_secs(1),
    ///     Duration::from_secs(2),
    /// ]));
    ///
    /// assert_eq!(strategy.next_backoff(), Some(Duration::from_secs(1)));
    /// assert_eq!(strategy.next_backoff(), Some(Duration::from_secs(2)));
    /// assert_eq!(strategy.next_backoff(), None); // No more durations
    /// ```
    fn next_backoff(&mut self) -> Option<Duration> {
        self.durations.pop_front()
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_no_interval_strategy() {
        let mut strategy = NoIntervalStrategy::new(3);
        assert_eq!(strategy.max_retries(), 3);
        assert_eq!(strategy.next_backoff(), None);
    }

    #[test]
    fn test_fixed_interval_strategy() {
        let mut strategy =
            FixedIntervalStrategy::new(3, Duration::from_secs(1));
        assert_eq!(strategy.max_retries(), 3);
        assert_eq!(strategy.next_backoff(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn test_exponential_custom_strategy() {
        let mut strategy = CustomIntervalStrategy::new(VecDeque::from([
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3),
        ]));
        assert_eq!(strategy.max_retries(), 3);
        assert!(strategy.next_backoff().is_some());
        assert!(strategy.next_backoff().is_some());
        assert!(strategy.next_backoff().is_some());
        assert!(strategy.next_backoff().is_none());
    }
}
