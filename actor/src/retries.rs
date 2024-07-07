// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Retries module.
//!
//! This module provides the necessary components for retrying messages through a backoff strategy.
//!

use crate::{
    supervision::{RetryStrategy, Strategy},
    Actor, ActorContext, ActorPath, Error, Event, Handler, Message, Response,
};

use serde::{Deserialize, Serialize};

use async_trait::async_trait;
use std::{fmt::Debug, marker::PhantomData};
use tracing::{debug, error};

/// Retry trait.
pub trait Retry: Actor + Handler<Self> + Debug + Clone {}

/// Retries.
#[derive(Debug, Clone)]
pub struct Retries<A>
where
    A: Retry,
{
    /// Retry strategy.
    retry_strategy: Strategy,

    /// The actor path.
    actor_path: ActorPath,

    /// Phantom data.
    _phantom: PhantomData<A>,
}

impl<A> Retries<A>
where
    A: Retry,
{
    /// Create a new retries.
    ///
    /// # Arguments
    ///
    /// * `retry_strategy` - Retry strategy.
    ///
    /// # Returns
    ///
    /// A new retries.
    ///
    pub fn new(retry_strategy: Strategy, actor_path: ActorPath) -> Self {
        Retries {
            retry_strategy,
            actor_path,
            _phantom: PhantomData,
        }
    }

    /// Apply retries.
    ///
    /// # Arguments
    ///
    /// * `msg` - Message to retry.
    /// * `ctx` - Actor context.
    ///
    /// # Returns
    ///
    /// The response of the retries.
    ///
    /// # Errors
    ///
    /// An error is returned if the retries fail.
    ///
    async fn apply_retries(
        &mut self,
        msg: A::Message,
        ctx: &mut ActorContext<Retries<A>>,
    ) {
        if let Some(child) = ctx.system().get_actor::<A>(&self.actor_path).await
        {
            let mut retries = 0;
            while retries < self.retry_strategy.max_retries() {
                debug!(
                    "Retry {}/{}.",
                    retries + 1,
                    self.retry_strategy.max_retries()
                );
                if let Ok(_) = child.ask(msg.clone()).await {
                    debug!("Response from retry");
                    return;
                } else {
                    if let Some(duration) = self.retry_strategy.next_backoff() {
                        debug!("Backoff for {:?}", &duration);
                        tokio::time::sleep(duration).await;
                    }
                    retries += 1;
                }
            }
        }
        error!("Retries with actor {} failed. Unknown actor.", self.actor_path);
        let _ = ctx
            .emit_error(Error::Functional(format!(
                "Retries with actor {} failed. Unknown actor.",
                self.actor_path
            )))
            .await;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryMessage<A: Retry>(A::Message);

impl<A: Retry> Message for RetryMessage<A> {}

/// Retry response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RetryResponse {
    /// Done
    Done,
    /// Fail
    Fail,
}

impl Response for RetryResponse {}

/// Retry event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryEvent<E>(E);

impl<E> Event for RetryEvent<E> where E: Event {}

/// 'Actor' implementation for 'Retries'.
#[async_trait]
impl<A> Actor for Retries<A>
where
    A: Retry,
{
    type Message = RetryMessage<A>;
    type Response = RetryResponse;
    type Event = RetryEvent<A::Event>;
}

#[async_trait]
impl<A> Handler<Retries<A>> for Retries<A>
where
    A: Retry,
{
    async fn handle_message(
        &mut self,
        msg: RetryMessage<A>,
        ctx: &mut ActorContext<Retries<A>>,
    ) -> Result<RetryResponse, Error> {
        match ctx.message(msg).await {
            Ok(_) => Ok(RetryResponse::Done),
            Err(_) => Ok(RetryResponse::Fail),
        }
    }

    async fn on_message(
        &mut self,
        msg: RetryMessage<A>,
        ctx: &mut ActorContext<Retries<A>>,
    ) {
        self.apply_retries(msg.0, ctx).await;
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{ActorSystem, NoIntervalStrategy};

    #[derive(Debug, Clone)]
    pub struct TestActor {
        pub counter: usize,
    }

    #[derive(Debug, Clone)]
    pub struct TestCommand(String);

    impl Message for TestCommand {}

    #[derive(Debug, Clone)]
    pub enum TestResponse {
        Done(usize),
        Fail,
    }

    impl Response for TestResponse {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestEvent(usize);

    impl Event for TestEvent {}

    impl Actor for TestActor {
        type Message = TestCommand;
        type Response = TestResponse;
        type Event = TestEvent;
    }

    impl Retry for TestActor {}

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            msg: TestCommand,
            ctx: &mut ActorContext<TestActor>,
        ) -> Result<TestResponse, Error> {
            if self.counter < 3 {
                self.counter += 1;
                Err(Error::Send("Fail".to_owned()))
            } else {
                Ok(TestResponse::Done(self.counter))
            }
        }
    }

    #[tokio::test]
    async fn test_retries() {
        let (system, mut runner) = ActorSystem::create();

        tokio::spawn(async move {
            runner.run().await;
        });

        let actor = TestActor { counter: 0 };
        let actor_ref = system
            .create_root_actor("test", TestActor { counter: 0 })
            .await
            .unwrap();
        let retry_strategy = Strategy::NoInterval(NoIntervalStrategy::new(3));
        let mut retries: Retries<TestActor> =
            Retries::new(retry_strategy, actor_ref.path().clone());
    }
}
