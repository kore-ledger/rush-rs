// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor runner
//!

use crate::{
    actor::{Actor, ActorContext, ActorRef, Handler},
    handler::{mailbox, HandleHelper, MailboxReceiver},
    supervision::SupervisionStrategy,
    system::ActorSystem,
    ActorPath,
};

use tokio::sync::broadcast::{self, Sender as EventSender};

use tracing::{debug, error};

/// Actor runner.
pub(crate) struct ActorRunner<A: Actor> {
    path: ActorPath,
    actor: A,
    receiver: MailboxReceiver<A>,
    event_sender: EventSender<A::Event>,
}

impl<A: Actor + Handler<A>> ActorRunner<A> {
    /// Creates a new actor runner and the actor reference.
    pub(crate) fn create(path: ActorPath, actor: A) -> (Self, ActorRef<A>) {
        debug!("Creating new actor runner.");
        let (sender, receiver) = mailbox();
        let (event_sender, event_receiver) = broadcast::channel(100);
        let helper = HandleHelper::new(sender);
        let actor_ref = ActorRef::new(path.clone(), helper, event_receiver);
        let runner = ActorRunner {
            path,
            actor,
            receiver,
            event_sender,
        };
        (runner, actor_ref)
    }

    /// Starts to run the actor.
    pub(crate) async fn start(&mut self, system: ActorSystem) {
        debug!("Starting actor runner.");

        // Create the actor context.
        let mut ctx = ActorContext::new(
            self.path.clone(),
            system,
            self.event_sender.clone(),
        );

        // Pre-start hook.
        let mut start_err = self.actor.pre_start(&ctx).await.err();

        // If pre-start hook fails, tries to restart the actor from `SupervisionStrategy`.
        if start_err.is_some() {
            let mut retries = 0;
            match A::supervision_strategy() {
                SupervisionStrategy::Stop => {
                    error!("Actor '{}' failed to start!", &self.path);
                }
                SupervisionStrategy::Retry(mut retry_strategy) => {
                    debug!(
                        "Restarting actor with retry strategy: {:?}",
                        &retry_strategy
                    );
                    while retries < retry_strategy.max_retries()
                        && start_err.is_some()
                    {
                        debug!("retries: {}", &retries);
                        if let Some(duration) = retry_strategy.next_backoff() {
                            debug!("Backoff for {:?}", &duration);
                            tokio::time::sleep(duration).await;
                        }
                        retries += 1;
                        start_err = ctx
                            .restart(&mut self.actor, start_err.as_ref())
                            .await
                            .err();
                    }
                }
            }
        }

        // If the actor can start, run it.
        if start_err.is_none() {
            debug!("Actor '{}' has started successfully.", &self.path);
            // The actor runs as long as active references exist. If all references to the actor
            // are removed, the execution ends.
            while let Some(mut msg) = self.receiver.recv().await {
                msg.handle(&mut self.actor, &mut ctx).await;
            }

            // Post stop hook.
            if self.actor.post_stop(&mut ctx).await.is_err() {
                error!("Actor '{}' failed to stop!", &self.path);
            }
            debug!("Actor '{}' stopped.", &self.path);
        }
        //
        self.receiver.close();
    }
}
