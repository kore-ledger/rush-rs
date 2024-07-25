// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Retries module.
//!
//! This module provides the necessary components for retrying messages through a backoff strategy.
//!

use crate::{
    supervision::{RetryStrategy, Strategy},
    Actor, ActorContext, ActorPath, ActorRef, Error, Handler,
};

use async_trait::async_trait;
use std::fmt::Debug;
use tracing::{debug, error};

/// Retry trait.
#[async_trait]
pub trait Retry: Actor + Handler<Self> + Debug + Clone {
    type Child: Actor + Handler<Self::Child> + Debug + Clone;

    /// Returns child actor.
    async fn child(
        &self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<ActorRef<Self::Child>, Error>;

    /// Retry message.
    async fn apply_retries(
        &self,
        ctx: &mut ActorContext<Self>,
        path: ActorPath,
        retry_strategy: &mut Strategy,
        message: <<Self as Retry>::Child as Actor>::Message,
    ) -> Result<<<Self as Retry>::Child as Actor>::Response, Error> {
        if let Ok(child) = self.child(ctx).await {
            let mut retries = 0;
            while retries < retry_strategy.max_retries() {
                debug!(
                    "Retry {}/{}.",
                    retries + 1,
                    retry_strategy.max_retries()
                );
                if let Ok(response) = child.ask(message.clone()).await {
                    debug!("Response from retry");
                    return Ok(response);
                } else {
                    if let Some(duration) = retry_strategy.next_backoff() {
                        debug!("Backoff for {:?}", &duration);
                        tokio::time::sleep(duration).await;
                    }
                    retries += 1;
                }
            }
            error!("Max retries with actor {} reached.", path);
            Err(Error::Functional(format!(
                "Max retries with actor {} reached.",
                path
            )))
        } else {
            error!("Retries with actor {} failed. Unknown actor.", path);
            Err(Error::Functional(format!(
                "Retries with actor {} failed. Unknown actor.",
                path
            )))
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
    struct ParentCommand;

    impl Message for ParentCommand {}

    #[derive(Debug, Clone)]
    struct ParentResponse(usize);

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
            _message: ParentCommand,
            ctx: &mut ActorContext<ParentActor>,
        ) -> Result<ParentResponse, Error> {
            //let name = message.0;
            let mut strategy = Strategy::FixedInterval(
                FixedIntervalStrategy::new(3, Duration::from_secs(1)),
            );
            let path = ctx.path().clone() / &self.child;
            let response = self
                .apply_retries(ctx, path, &mut strategy, ChildCommand)
                .await
                .unwrap();
            Ok(ParentResponse(response.0))
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

        let response = actor.ask(ParentCommand).await.unwrap();
        assert_eq!(response.0, 2);
    }
}
