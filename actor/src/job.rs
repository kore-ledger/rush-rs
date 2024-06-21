// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Jobs module.
//! 
//! This module provides the necessary components for executing planned jobs during an actor's life
//! cycle, from simple cleaning and checking tasks to tasks to be executed in relation to the 
//! actor's state.
//!

use crate::{Error, Actor, ActorContext, supervision::RetryStrategy};
use futures::future::BoxFuture;

/// Boxed job.
pub type BoxedJob = Box<dyn Job + Send + Sync>;

/// Result of a job execution.
type JobResult = BoxFuture<'static, Result<(), Error>>;

/// A job to be executed. This trait must be implemented in any job that must be executed in a 
/// planned manner.
pub trait Job {

    /// Name of the job.
    /// 
    /// # Returns
    /// 
    /// The name of the job.
    /// 
    fn name(&self) -> &str;
    
    /// Execute the job.
    /// 
    /// # Arguments
    /// 
    /// * `ctx` - Actor context.
    /// 
    /// # Returns
    /// 
    /// Result of the job execution.
    /// 
    fn run(&self) -> JobResult;
    
    /// Retry strategy for the job.
    /// 
    /// # Returns
    /// 
    /// The optional retry strategy for the job. 
    /// 
    fn retry_strategy(&self) -> Option<Box<dyn RetryStrategy>>;

}

pub struct JobContainer {
    job: BoxedJob,
    last_execution: u64,
    retry_strategy: Option<Box<dyn RetryStrategy>>,
}

/// Job runner.
pub struct JobRunner{
    jobs: Vec<BoxedJob>,
  
}

impl JobRunner {

    fn new() -> Self {
        Self {
            jobs: Vec::new(),
        }
    }

    fn add_job(&mut self, job: BoxedJob) {
        self.jobs.push(job);
    }

}