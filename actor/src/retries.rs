// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Retries module.
//!
//! This module provides the necessary components for retrying messages through a backoff strategy.
//!

use crate::{
    supervision::{RetryStrategy, Strategy},
    Actor, ActorRef, ActorContext, Error, Handler,
};

use async_trait::async_trait;

use tracing::{debug, error};
use std::fmt::Debug;

/// Retry trait.
#[async_trait]
pub trait Retry: Actor + Handler<Self> + Debug + Clone {
    type Child: Actor + Handler<Self::Child> + Debug + Clone;

    /// Returns child actor.
    async fn child(
        &self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<ActorRef<Self::Child>, Error>;

    /// Retry strategy.
    fn retry_strategy(&self) -> Strategy;

    /// Apply response.
    async fn apply_response(
        &mut self,
        response: <<Self as Retry>::Child as Actor>::Response,
        ctx: &mut ActorContext<Self>,
    );

    /// Retry message.
    async fn apply_retries(
        &mut self,
        ctx: &mut ActorContext<Self>,
        message: <<Self as Retry>::Child as Actor>::Message,
    ) {
        if let Ok(child) = self.child(ctx).await {
            let mut strategy = self.retry_strategy();

            let mut retries = 0;

            while retries < strategy.max_retries() {
                debug!("Retry {}/{}.", retries + 1, strategy.max_retries());
                if let Ok(response) = child.ask(message.clone()).await {
                    debug!("Response from retry");
                    self.apply_response(response, ctx).await;
                    return;
                } else {
                    if let Some(duration) = strategy.next_backoff() {
                        debug!("Backoff for {:?}", &duration);
                        tokio::time::sleep(duration).await;
                    }
                    retries += 1;
                }
            }
            let path = child.path();
            error!("Max retries reached with actor {}.", path);
            let _ = ctx
                .emit_error(Error::Functional(format!(
                    "Max retries reached with actor {}.",
                    path
                )))
                .await;
        } else {
            error!("Retries with actor. Unknown actor.");
            let _ = ctx
                .emit_error(Error::Functional(
                    "Retries with actor {} failed. Unknown actor.".to_owned(),
                
                ))
                .await;
        }
    }
}


#[cfg(test)]
mod tests {

    use std::time::Duration;

    use crate::{
        Actor, ActorSystem, Event, FixedIntervalStrategy, Message, Response,
    };

    use super::*;

    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone)]
    struct ParentActor {
        pub child: String,
    }

    #[derive(Debug, Clone)]
    enum ParentCommand {
        Retry,
        Size(usize),
    }

    impl Message for ParentCommand {}

    #[derive(Debug, Clone)]
 enum ParentResponse {
    None,
 }

    impl Response for ParentResponse {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ParentEvent;

    impl Event for ParentEvent {}

    impl Actor for ParentActor {
        type Message = ParentCommand;
        type Response = ParentResponse;
        type Event = ParentEvent;
    }

    #[async_trait]
    impl Handler<ParentActor> for ParentActor {
        async fn handle_message(
            &mut self,
            message: ParentCommand,
            ctx: &mut ActorContext<ParentActor>,
        ) -> Result<ParentResponse, Error> {
            match message {
                ParentCommand::Retry => {
                    ctx.message(message).await.unwrap();
                }
                ParentCommand::Size(size) => {
                    assert_eq!(size, 2);
                }
            }
            Ok(ParentResponse::None)
        }

        async fn on_message(
            &mut self,
            _message: ParentCommand,
            ctx: &mut ActorContext<ParentActor>,
        ) {
            self.apply_retries(ctx, ChildCommand).await;
        }
    }

    #[derive(Debug, Clone)]
    struct ChildActor {
        pub counter: usize,
    }

    #[derive(Debug, Clone)]
    struct ChildCommand;

    impl Message for ChildCommand {}

    #[derive(Debug, Clone)]
    struct ChildResponse(usize);

    impl Response for ChildResponse {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ChildEvent;

    impl Event for ChildEvent {}

    impl Actor for ChildActor {
        type Message = ChildCommand;
        type Response = ChildResponse;
        type Event = ChildEvent;
    }

    #[async_trait]
    impl Handler<ChildActor> for ChildActor {
        async fn handle_message(
            &mut self,
            _message: ChildCommand,
            _ctx: &mut ActorContext<ChildActor>,
        ) -> Result<ChildResponse, Error> {
            if self.counter < 2 {
                self.counter += 1;
                Err(Error::Functional("Counter reached".to_string()))
            } else {
                Ok(ChildResponse(self.counter))
            }
        }
    }
    #[async_trait]
    impl Retry for ParentActor {
        type Child = ChildActor;

        async fn child(
            &self,
            ctx: &mut ActorContext<ParentActor>,
        ) -> Result<ActorRef<ChildActor>, Error> {
            ctx.create_child(&self.child, ChildActor { counter: 0 })
                .await
        }

        fn retry_strategy(&self) -> Strategy {
            Strategy::FixedInterval(FixedIntervalStrategy::new(3, Duration::from_secs(1)))
        }

        async fn apply_response(
            &mut self,
            response: ChildResponse,
            ctx: &mut ActorContext<ParentActor>,
        ) {
            assert_eq!(response.0, 2);
            self.handle_message(ParentCommand::Size(response.0), ctx).await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_apply_retries() {
        let (system, mut runner) = ActorSystem::create();

        tokio::spawn(async move {
            runner.run().await;
        });

        let actor = system
            .create_root_actor(
                "parent",
                ParentActor {
                    child: "child".to_owned(),
                },
            )
            .await
            .unwrap();

        actor.tell(ParentCommand::Retry).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
