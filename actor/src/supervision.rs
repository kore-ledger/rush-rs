// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! Supervision strategies
//!

use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
    time::Duration,
};

use backoff::backoff::Backoff as InnerBackoff;

/// Trait to define a RetryStrategy. You can use this trait to define your
/// custom retry strategy.
pub trait RetryStrategy: Debug + Send + Sync {
    /// Maximum number of tries before permanently failing an actor
    fn max_retries(&self) -> usize;
    /// Wait duration before retrying
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

/// A Retry strategy that immediately retries an actor that failed to start
#[derive(Debug, Clone)]
pub enum Strategy {
    NoInterval(NoIntervalStrategy),
    FixedInterval(FixedIntervalStrategy),
    ExponentialBackoff(ExponentialBackoffStrategy),
}

impl RetryStrategy for Strategy {
    fn max_retries(&self) -> usize {
        match self {
            Strategy::NoInterval(strategy) => strategy.max_retries(),
            Strategy::FixedInterval(strategy) => strategy.max_retries(),
            Strategy::ExponentialBackoff(strategy) => strategy.max_retries(),
        }
    }

    fn next_backoff(&mut self) -> Option<Duration> {
        match self {
            Strategy::NoInterval(strategy) => strategy.next_backoff(),
            Strategy::FixedInterval(strategy) => strategy.next_backoff(),
            Strategy::ExponentialBackoff(strategy) => strategy.next_backoff(),
        }
    }
}

impl Default for Strategy {
    fn default() -> Self {
        Strategy::NoInterval(NoIntervalStrategy::default())
    }
}

/// A Retry strategy that immediately retries an actor that failed to start
#[derive(Debug, Default, Clone)]
pub struct NoIntervalStrategy {
    max_retries: usize,
}

impl NoIntervalStrategy {
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

/// A retry strategy that retries an actor with a fixed wait period before
/// retrying.
#[derive(Debug, Default, Clone)]
pub struct FixedIntervalStrategy {
    /// Maximum number of retries before permanently failing an actor.
    max_retries: usize,
    /// Wait duration before retrying.
    duration: Duration,
}

/// Implementation of fixed interval strategy.
impl FixedIntervalStrategy {
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

/// A retry strategy that retries an actor with an exponential backoff wait
/// period before retrying.
#[derive(Debug, Default, Clone)]
pub struct ExponentialBackoffStrategy {
    /// Maximum number of retries before permanently failing an actor.
    max_retries: usize,
    /// Inner exponential backoff strategy.
    inner: Arc<Mutex<backoff::ExponentialBackoff>>,
}

/// Implementation of exponential backoff strategy.
impl ExponentialBackoffStrategy {
    pub fn new(max_retries: usize) -> Self {
        ExponentialBackoffStrategy {
            max_retries,
            inner: Arc::new(Mutex::new(backoff::ExponentialBackoff::default())),
        }
    }
}

/// Implementation of `RetryStrategy` for `ExponentialBackoffStrategy`.
impl RetryStrategy for ExponentialBackoffStrategy {
    fn max_retries(&self) -> usize {
        self.max_retries
    }

    fn next_backoff(&mut self) -> Option<Duration> {
        self.inner.lock().ok().and_then(|mut eb| eb.next_backoff())
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
    fn test_exponential_backoff_strategy() {
        let mut strategy = ExponentialBackoffStrategy::new(3);
        assert_eq!(strategy.max_retries(), 3);
        assert!(strategy.next_backoff().is_some());
    }
}
