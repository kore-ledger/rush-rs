// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor runner
//!

use crate::{
    actor::{Actor, ActorContext, ActorLifecycle, ActorRef, Handler},
    handler::{mailbox, HandleHelper, MailboxReceiver},
    supervision::SupervisionStrategy,
    system::ActorSystem,
    ActorPath,
};

use tokio::{select, sync::broadcast::{self, Sender as EventSender}};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

/// Actor runner.
pub(crate) struct ActorRunner<A: Actor> {
    path: ActorPath,
    actor: A,
    receiver: MailboxReceiver<A>,
    event_sender: EventSender<A::Event>,
    token: CancellationToken,
}

impl<A: Actor + Handler<A>> ActorRunner<A> {
    /// Creates a new actor runner and the actor reference.
    pub(crate) fn create(path: ActorPath, actor: A) -> (Self, ActorRef<A>) {
        debug!("Creating new actor runner.");
        let (sender, receiver) = mailbox();
        let (event_sender, event_receiver) = broadcast::channel(100);
        let helper = HandleHelper::new(sender);
        let actor_ref = ActorRef::new(path.clone(), helper, event_receiver);
        let token = CancellationToken::new();
        let runner = ActorRunner {
            path,
            actor,
            receiver,
            event_sender,
            token,
        };
        (runner, actor_ref)
    }

    /// Init the actor runner.
    pub(crate) async fn init(&mut self, system: ActorSystem) {
        debug!("Initializing actor {} runner.", &self.path);
        
        // Create the actor context.
        debug!("Creating actor {} context.", &self.path);
        let mut ctx: ActorContext<A> = ActorContext::new(
            self.path.clone(),
            system,
            self.token.clone(),
            self.event_sender.clone(),
        );
        
        // Main loop of the actor.
        let mut retries = 0;
        loop {
            match ctx.state() {
                ActorLifecycle::Created => {
                    debug!("Actor {} is created.", &self.path);
                    // Pre-start hook.
                    match self.actor.pre_start(&ctx).await {
                        Ok(_) => {
                            debug!("Actor '{}' has started successfully.", &self.path);
                            ctx.set_state(ActorLifecycle::Started);
                        }
                        Err(err) => {
                            error!("Actor {} failed to start: {:?}", &self.path, err);
                            ctx.set_state(ActorLifecycle::Faulty);
                        }
                    }
                }
                ActorLifecycle::Started => {
                    debug!("Actor {} is started.", &self.path);
                    self.run(&mut ctx).await;
                    if ctx.error().is_none() {
                        ctx.set_state(ActorLifecycle::Stopped);
                    } else {
                        ctx.set_state(ActorLifecycle::Faulty);
                    }
                }
                ActorLifecycle::Stopped => {
                    debug!("Actor {} is stopped.", &self.path);
                    // Post stop hook.
                    if self.actor.post_stop(&mut ctx).await.is_err() {
                        error!("Actor '{}' failed to stop!", &self.path);
                    }
                    ctx.set_state(ActorLifecycle::Terminated);
                }
                ActorLifecycle::Faulty => {
                    debug!("Actor {} is faulty.", &self.path);
                    match A::supervision_strategy() {
                        SupervisionStrategy::Stop => {
                            error!("Actor '{}' failed to start!", &self.path);
                            ctx.set_state(ActorLifecycle::Stopped);
                        }
                        SupervisionStrategy::Retry(mut retry_strategy) => {
                            debug!(
                                "Restarting actor with retry strategy: {:?}",
                                &retry_strategy
                            );
                            if retries < retry_strategy.max_retries()
                                && ctx.state() == &ActorLifecycle::Faulty
                            {
                                debug!("retries: {}", &retries);
                                if let Some(duration) = retry_strategy.next_backoff() {
                                    debug!("Backoff for {:?}", &duration);
                                    tokio::time::sleep(duration).await;
                                }
                                retries += 1;
                                let error = ctx.error();
                                match ctx
                                    .restart(&mut self.actor, error.as_ref())
                                    .await
                                {
                                    Ok(_) => {
                                        ctx.set_state(ActorLifecycle::Started);
                                        retries = 0;
                                        let token = CancellationToken::new();
                                        ctx.set_token(token.clone());
                                        self.token = token;
                                    }
                                    Err(err) => {
                                        ctx.failed(err);
                                    }
                                }
                            } else {
                                ctx.set_state(ActorLifecycle::Stopped);
                            }
                        }
                    }       
                }
                ActorLifecycle::Terminated => {
                    break;
                }
            }
        }
        self.receiver.close();
    }

    // TODO: Delete after testing.
    /* 
    /// Starts to run the actor.
    pub(crate) async fn start(&mut self, system: ActorSystem) {
        debug!("Starting actor runner.");

        // Create the actor context.
        let mut ctx = ActorContext::new(
            self.path.clone(),
            system,
            self.token.clone(),
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
            self.run(&mut ctx).await;
            /* 
            // The actor runs as long as active references exist. If all references to the actor
            // are removed, the execution ends.
            while let Some(mut msg) = self.receiver.recv().await {
                msg.handle(&mut self.actor, &mut ctx).await;
            }*/

            // Post stop hook.
            if self.actor.post_stop(&mut ctx).await.is_err() {
                error!("Actor '{}' failed to stop!", &self.path);
            }
            debug!("Actor '{}' stopped.", &self.path);
        }
        //
        self.receiver.close();
    }
*/
    /// Main loop of the actor.
    /// It runs the actor until the actor is stopped.
    /// The actor runs as long as active references exist. If all references to the actor are
    /// removed or emit `self.token.cancel(), the execution ends.
    /// 
    /// # Arguments
    /// 
    /// * `ctx` - The actor context.
    /// 
    pub(crate) async fn run(&mut self, ctx: &mut ActorContext<A>) {
        loop {
            select! {
                msg = self.receiver.recv() => {
                    if let Some(mut msg) = msg {
                        msg.handle(&mut self.actor, ctx).await;
                    } else {
                        break;
                    }
                }
                _ = self.token.cancelled() => {
                    break;
                }
            }
        }
    }
}
