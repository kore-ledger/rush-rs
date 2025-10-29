

//! Supervision strategies
//!

use std::{collections::VecDeque, fmt::Debug, time::Duration};

/// Trait to define a retry strategy for actor startup failures.
/// Implementations of this trait control how many times an actor should
/// be retried and how long to wait between attempts.
///
/// # Usage
///
/// You can implement this trait to define custom retry logic with
/// backoff strategies like exponential backoff, jittered delays, etc.
///
pub trait RetryStrategy: Debug + Send + Sync {
    /// Returns the maximum number of retry attempts before permanently failing the actor.
    ///
    /// # Returns
    ///
    /// The maximum number of retries allowed.
    ///
    fn max_retries(&self) -> usize;

    /// Returns the wait duration before the next retry attempt.
    ///
    /// # Returns
    ///
    /// Some(Duration) if there should be a delay, None for immediate retry.
    /// This method may be called multiple times and can return different
    /// durations for progressive backoff strategies.
    ///
    fn next_backoff(&mut self) -> Option<Duration>;
}

/// A SupervisionStrategy defined what to do when an actor fails at startup.
/// Currently there are two choices: Stop the actor and do nothing, or Retry
/// the startup. For Retry you can set a RetryStrategy.
#[derive(Debug, Clone)]
pub enum SupervisionStrategy {
    /// Stop the actor if an error occurs at startup
    Stop,
    /// Retry start the actor if an error occurs at startup
    Retry(Strategy),
}

/// Concrete retry strategy implementations.
/// This enum wraps different retry strategy types, providing a unified
/// interface for retry logic with different timing behaviors.
#[derive(Debug, Clone)]
pub enum Strategy {
    /// Retry immediately with no delay between attempts.
    NoInterval(NoIntervalStrategy),
    /// Retry with a fixed delay between attempts.
    FixedInterval(FixedIntervalStrategy),
    /// Retry with custom-defined delays for each attempt.
    CustomIntervalStrategy(CustomIntervalStrategy),
}

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

impl Default for Strategy {
    fn default() -> Self {
        Strategy::NoInterval(NoIntervalStrategy::default())
    }
}

/// A retry strategy that immediately retries an actor with no delay.
/// This strategy attempts to restart the actor as quickly as possible
/// after each failure, up to the maximum number of retries.
///
/// # Use Cases
///
/// Best for transient errors that are expected to resolve immediately,
/// such as temporary resource contention or race conditions.
///
#[derive(Debug, Default, Clone)]
pub struct NoIntervalStrategy {
    /// Maximum number of retry attempts.
    max_retries: usize,
}

impl NoIntervalStrategy {
    /// Creates a new NoIntervalStrategy with the specified maximum retries.
    ///
    /// # Arguments
    ///
    /// * `max_retries` - Maximum number of retry attempts before giving up.
    ///
    /// # Returns
    ///
    /// Returns a new NoIntervalStrategy instance.
    ///
    pub fn new(max_retries: usize) -> Self {
        NoIntervalStrategy { max_retries }
    }
}

impl RetryStrategy for NoIntervalStrategy {
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    fn next_backoff(&mut self) -> Option<Duration> {
        None
    }
}

/// A retry strategy that waits a fixed duration between retry attempts.
/// This strategy adds a consistent delay between each restart attempt,
/// which can help avoid overwhelming resources or rapid failure loops.
///
/// # Use Cases
///
/// Suitable for errors that may require a brief cooldown period, such as
/// waiting for external services to recover or rate limits to reset.
///
#[derive(Debug, Default, Clone)]
pub struct FixedIntervalStrategy {
    /// Maximum number of retries before permanently failing an actor.
    max_retries: usize,
    /// Fixed wait duration before each retry attempt.
    duration: Duration,
}

/// Implementation of fixed interval strategy.
impl FixedIntervalStrategy {
    /// Creates a new FixedIntervalStrategy with specified retries and delay.
    ///
    /// # Arguments
    ///
    /// * `max_retries` - Maximum number of retry attempts before giving up.
    /// * `duration` - Fixed delay duration between retry attempts.
    ///
    /// # Returns
    ///
    /// Returns a new FixedIntervalStrategy instance.
    ///
    pub fn new(max_retries: usize, duration: Duration) -> Self {
        FixedIntervalStrategy {
            max_retries,
            duration,
        }
    }
}

/// Implementation of `RetryStrategy` for `FixedIntervalStrategy`.
impl RetryStrategy for FixedIntervalStrategy {
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    fn next_backoff(&mut self) -> Option<Duration> {
        Some(self.duration)
    }
}

/// A retry strategy with custom-defined delays for each retry attempt.
/// This strategy allows you to specify a different delay for each retry,
/// enabling complex backoff patterns like exponential backoff, fibonacci
/// sequences, or any custom progression.
///
/// # Use Cases
///
/// Ideal for sophisticated error handling scenarios where you want fine-grained
/// control over retry timing, such as exponential backoff with jitter or
/// adapting delays based on the type of failure.
///
/// # Usage
///
/// Allows implementing exponential backoff, fibonacci sequences, or any
/// custom progression by providing a VecDeque of Duration values.
///
#[derive(Debug, Default, Clone)]
pub struct CustomIntervalStrategy {
    /// Queue of delay durations for each retry attempt.
    /// Each call to next_backoff() pops one duration from the front.
    durations: VecDeque<Duration>,
    /// Maximum number of retries (equal to the number of durations provided).
    max_retries: usize,
}

impl CustomIntervalStrategy {
    /// Creates a new CustomIntervalStrategy with the specified delay sequence.
    ///
    /// # Arguments
    ///
    /// * `durations` - A queue of durations defining the delay before each retry.
    ///                 The number of durations determines the maximum number of retries.
    ///
    /// # Returns
    ///
    /// Returns a new CustomIntervalStrategy instance.
    ///
    pub fn new(durations: VecDeque<Duration>) -> Self {
        Self {
            durations: durations.clone(),
            max_retries: durations.len(),
        }
    }
}

impl RetryStrategy for CustomIntervalStrategy {
    fn max_retries(&self) -> usize {
        self.max_retries
    }

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
