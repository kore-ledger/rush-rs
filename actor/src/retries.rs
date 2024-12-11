// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Retries module.
//!
//! This module provides the necessary components for retrying messages through a backoff strategy.
//!

use crate::{
    supervision::{RetryStrategy, Strategy},
    Actor, ActorContext, ActorPath, ActorRef, Error, Event, Handler, Message,
    Response,
};

use async_trait::async_trait;

use std::fmt::Debug;
use tracing::{debug, error, warn};

/// Retry actor.
pub struct RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    target: T,
    message: T::Message,
    retry_strategy: Strategy,
    retries: usize,
    is_end: bool,
}

impl<T> RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    /// Create a new RetryActor.
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
#[derive(Debug, Clone)]
pub enum RetryMessage {
    Retry,
    End,
}

impl Message for RetryMessage {}

impl Response for () {}

impl Event for () {}

#[async_trait]
impl<T> Actor for RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    type Message = RetryMessage;
    type Response = ();
    type Event = ();

    async fn pre_start(
        &mut self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        debug!("RetryActor pre_start");
        let _ = ctx.create_child::<T>("target", self.target.clone()).await?;
        Ok(())
    }
}

#[async_trait]
impl<T> Handler<RetryActor<T>> for RetryActor<T>
where
    T: Actor + Handler<T> + Clone,
{
    async fn handle_message(
        &mut self,
        _path: ActorPath,
        message: RetryMessage,
        ctx: &mut ActorContext<RetryActor<T>>,
    ) -> Result<(), Error> {
        match message {
            RetryMessage::Retry => {
                if !self.is_end {
                    self.retries += 1;
                    if self.retries <= self.retry_strategy.max_retries() {
                        debug!(
                            "Retry {}/{}.",
                            self.retries,
                            self.retry_strategy.max_retries()
                        );

                        // Send retry to parent.
                        if let Some(child) = ctx.get_child::<T>("target").await
                        {
                            if child.tell(self.message.clone()).await.is_err() {
                                error!("Cannot initiate retry to send message");
                            }
                        }

                        if let Some(actor) =
                            ctx.reference().await
                        {
                            if let Some(duration) =
                            self.retry_strategy.next_backoff()
                        {
                            let actor: ActorRef<RetryActor<T>> = actor;
                            tokio::spawn(async move {
                                debug!("Backoff for {:?}", &duration);
                                tokio::time::sleep(duration).await;
                                if actor.tell(RetryMessage::Retry).await.is_err() {
                                    error!("Cannot initiate retry to send message");
                                }
                            }); 
                        }
                            
                            
                        } else {
                            let _ = ctx
                                .emit_error(Error::Send(
                                    "Cannot get retry actor in".to_owned(),
                                ))
                                .await;
                        };
                        // Next retry
                    } else {
                        warn!("Max retries reached.");
                        if let Err(e) = ctx
                            .emit_error(Error::ReTry)
                            .await {
                                error!("Error in emit_error {}", e);
                            };
                    }
                }
            }
            RetryMessage::End => {
                self.is_end = true;
                debug!("RetryActor end");
                ctx.stop().await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::{ActorSystem, Error, FixedIntervalStrategy};

    use std::time::Duration;

    pub struct SourceActor;

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

    #[derive(Clone)]
    pub struct TargetActor {
        counter: usize,
    }

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

    #[tokio::test]
    async fn test_retry_actor() {
        let (system, mut runner) = ActorSystem::create(None);

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
