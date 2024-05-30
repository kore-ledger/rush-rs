// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor runner
//!

use crate::{
    actor::{Actor, ActorContext, ActorLifecycle, ActorRef, Handler},
    error::{error_box, ErrorBoxReceiver, ErrorHelper, SystemError},
    handler::{mailbox, HandleHelper, MailboxReceiver},
    supervision::SupervisionStrategy,
    system::ActorSystem,
    ActorPath, Error,
};

use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{
        broadcast::{self, Sender as EventSender},
        mpsc,
    },
};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

/// Inner sender and receiver types.
pub type InnerSender<A> = mpsc::UnboundedSender<InnerEvent<A>>;
pub type InnerReceiver<A> = mpsc::UnboundedReceiver<InnerEvent<A>>;

/// Actor runner.
pub(crate) struct ActorRunner<A: Actor, P: Actor> {
    path: ActorPath,
    actor: A,
    receiver: MailboxReceiver<A>,
    event_sender: EventSender<A::Event>,
    error_sender: ErrorHelper<A>,
    parent_helper: Option<ErrorHelper<P>>,
    error_receiver: ErrorBoxReceiver<A>,
    inner_sender: InnerSender<A>,
    inner_receiver: InnerReceiver<A>,
    token: CancellationToken,
}

impl<A, P> ActorRunner<A, P>
where
    A: Actor + Handler<A>,
    P: Actor + Handler<P>,
{
    /// Creates a new actor runner and the actor reference.
    pub(crate) fn create(
        path: ActorPath,
        actor: A,
        parent_helper: Option<ErrorHelper<P>>,
    ) -> (Self, ActorRef<A>) {
        debug!("Creating new actor runner.");
        let (sender, receiver) = mailbox();
        let (error_sender, error_receiver) = error_box();
        let (event_sender, event_receiver) = broadcast::channel(100);
        let (inner_sender, inner_receiver) = mpsc::unbounded_channel();
        let helper = HandleHelper::new(sender);
        let error_helper = ErrorHelper::new(error_sender);
        let actor_ref = ActorRef::new(path.clone(), helper, event_receiver);
        let token = CancellationToken::new();
        let runner = ActorRunner {
            path,
            actor,
            receiver,
            event_sender,
            error_sender: error_helper,
            parent_helper,
            error_receiver,
            inner_sender,
            inner_receiver,
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
            self.error_sender.clone(),
            self.inner_sender.clone(),
        );

        // Main loop of the actor.
        let mut retries = 0;
        loop {
            match ctx.state() {
                // State: CREATED
                ActorLifecycle::Created => {
                    debug!("Actor {} is created.", &self.path);
                    // Pre-start hook.
                    match self.actor.pre_start(&ctx).await {
                        Ok(_) => {
                            debug!(
                                "Actor '{}' has started successfully.",
                                &self.path
                            );
                            ctx.set_state(ActorLifecycle::Started);
                        }
                        Err(err) => {
                            error!(
                                "Actor {} failed to start: {:?}",
                                &self.path, err
                            );
                            ctx.failed(err);
                            ctx.set_state(ActorLifecycle::Failed);
                        }
                    }
                }
                // State: STARTED
                ActorLifecycle::Started => {
                    debug!("Actor {} is started.", &self.path);
                    self.run(&mut ctx).await;
                    if ctx.error().is_none() {
                        ctx.set_state(ActorLifecycle::Stopped);
                    } else {
                        ctx.set_state(ActorLifecycle::Failed);
                    }
                }
                // State: STOPPED
                ActorLifecycle::Stopped => {
                    debug!("Actor {} is stopped.", &self.path);
                    // Post stop hook.
                    if self.actor.post_stop(&mut ctx).await.is_err() {
                        error!("Actor '{}' failed to stop!", &self.path);
                    }
                    ctx.set_state(ActorLifecycle::Terminated);
                }
                // State: FAILED
                ActorLifecycle::Failed => {
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
                                && ctx.state() == &ActorLifecycle::Failed
                            {
                                debug!("retries: {}", &retries);
                                if let Some(duration) =
                                    retry_strategy.next_backoff()
                                {
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
                // State: TERMINATED
                ActorLifecycle::Terminated => {
                    debug!("Actor {} is terminated.", &self.path);
                    break;
                }
            }
        }
        self.receiver.close();
    }

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
        debug!("Running actor {}.", &self.path);
        loop {
            select! {
                // Handle message from `ActorRef`.
                msg = self.receiver.recv() => {
                    if let Some(mut msg) = msg {
                        msg.handle(&mut self.actor, ctx).await;
                    } else {
                        break;
                    }
                }
                // Handle error from `ErrorBoxReceiver`.
                error =  self.error_receiver.recv() => {
                    if let Some(mut error) = error {
                        error.handle(&mut self.actor, ctx).await;
                    } else {
                        break;
                    }
                }
                // Handle inner event from `inner_receiver`.
                event = self.inner_receiver.recv() => {
                    if let Some(event) = event {
                        self.inner_handle(event).await;
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

    /// Inner handle event.
    async fn inner_handle(&mut self, event: InnerEvent<A>) {
        match event {
            InnerEvent::Event(event) => match self.event_sender.send(event) {
                Ok(size) => {
                    debug!("Event sent successfully to {} subscribers.", size);
                }
                Err(err) => {
                    error!("Failed to send event: {:?}", err);
                }
            },
            InnerEvent::Error(error) => {
                if let Some(parent_helper) = self.parent_helper.as_mut() {
                    parent_helper
                        .send(SystemError::Error(error))
                        .await
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send error to parent actor: {:?}",
                                err
                            );
                        });
                }
            }
            InnerEvent::Fail(error) => {
                if let Some(parent_helper) = self.parent_helper.as_mut() {
                    parent_helper
                        .send(SystemError::Fail(error))
                        .await
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send fail to parent actor: {:?}",
                                err
                            );
                        });
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InnerEvent<A: Actor> {
    Event(A::Event),
    Error(Error),
    Fail(Error),
}

#[cfg(test)]
mod tests {

    //use super::*;

    use crate::{
        actor::{Actor, ActorContext, Event, Handler, Message},
        supervision::{FixedIntervalStrategy, SupervisionStrategy},
        Error,
    };

    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use tracing_test::traced_test;

    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub struct TestMessage;

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestEvent;

    impl Event for TestEvent {}

    #[derive(Debug, Clone)]
    pub struct TestActor {
        failed: bool,
    }

    #[async_trait]
    impl Actor for TestActor {
        type Message = TestMessage;
        type Response = ();
        type Event = TestEvent;

        fn supervision_strategy() -> SupervisionStrategy {
            SupervisionStrategy::Retry(Box::new(FixedIntervalStrategy::new(
                3,
                Duration::from_secs(1),
            )))
        }

        async fn pre_start(
            &mut self,
            _ctx: &ActorContext<Self>,
        ) -> Result<(), Error> {
            if self.failed {
                Err(Error::Start)
            } else {
                Ok(())
            }
        }

        async fn pre_restart(
            &mut self,
            _ctx: &mut ActorContext<Self>,
            _error: Option<&Error>,
        ) -> Result<(), Error> {
            if self.failed {
                self.failed = false;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle(
            &mut self,
            _msg: TestMessage,
            _ctx: &mut ActorContext<Self>,
        ) {
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_actor_runner_failed() {
        /*     let system = ActorSystem::default();

                let actor = TestActor { failed: true };

                let (mut runner, actor_ref) = ActorRunner::create(
                    ActorPath::from("/user/test"),
                    actor,
                );
                // Init the actor runner.
                tokio::spawn(async move {
                    runner.init(system).await;
                });
                tokio::time::sleep(Duration::from_secs(2)).await;

                assert!(logs_contain("Creating new actor runner"));
                assert!(logs_contain("Actor /user/test is created"));
                assert!(logs_contain("Actor /user/test failed to start"));
                assert!(logs_contain("Actor /user/test is faulty"));
                assert!(logs_contain("Restarting actor with retry strategy"));
                assert!(logs_contain("Actor /user/test is started"));

                let _ = actor_ref.tell(TestMessage).await;

                assert!(logs_contain("Message sent successfully"));
        */
    }
}
