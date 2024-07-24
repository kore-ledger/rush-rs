// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor runner
//!

use crate::{
    actor::{
        Actor, ActorContext, ActorLifecycle, ActorRef, ChildAction, ChildError,
        ChildErrorReceiver, ChildErrorSender, Handler,
    },
    //error::{error_box, ErrorBoxReceiver, ErrorHelper, SystemError},
    handler::{mailbox, HandleHelper, MailboxReceiver},
    supervision::{RetryStrategy, SupervisionStrategy},
    system::SystemRef,
    ActorPath,
    Error,
};

use tokio::{
    select,
    sync::{
        broadcast::{self, Sender as EventSender},
        mpsc, oneshot,
    },
};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

/// Inner sender and receiver types.
pub type InnerSender<A> = mpsc::UnboundedSender<InnerMessage<A>>;
pub type InnerReceiver<A> = mpsc::UnboundedReceiver<InnerMessage<A>>;

/// Actor runner.
pub(crate) struct ActorRunner<A: Actor> {
    path: ActorPath,
    actor: A,
    receiver: MailboxReceiver<A>,
    event_sender: EventSender<A::Event>,
    error_sender: ChildErrorSender,
    parent_sender: Option<ChildErrorSender>,
    error_receiver: ChildErrorReceiver,
    inner_sender: InnerSender<A>,
    inner_receiver: InnerReceiver<A>,
    token: CancellationToken,
}

impl<A> ActorRunner<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor runner and the actor reference.
    pub(crate) fn create(
        path: ActorPath,
        actor: A,
        parent_sender: Option<ChildErrorSender>,
    ) -> (Self, ActorRef<A>) {
        debug!("Creating new actor runner.");
        let (sender, receiver) = mailbox();
        let (error_sender, error_receiver) = mpsc::unbounded_channel();
        let (event_sender, event_receiver) = broadcast::channel(100);
        let (inner_sender, inner_receiver) = mpsc::unbounded_channel();
        let helper = HandleHelper::new(sender);
        //let error_helper = ErrorHelper::new(error_sender);
        let actor_ref = ActorRef::new(path.clone(), helper, event_receiver);
        let token = CancellationToken::new();
        let runner = ActorRunner {
            path,
            actor,
            receiver,
            event_sender,
            error_sender,
            parent_sender,
            error_receiver,
            inner_sender,
            inner_receiver,
            token,
        };
        (runner, actor_ref)
    }

    /// Init the actor runner.
    pub(crate) async fn init(&mut self, system: SystemRef) {
        debug!("Initializing actor {} runner.", &self.path);

        // Create the actor context.
        debug!("Creating actor {} context.", &self.path);
        let mut ctx: ActorContext<A> = ActorContext::new(
            self.path.clone(),
            system.clone(),
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
                    match self.actor.pre_start(&mut ctx).await {
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
                            ctx.set_error(err);
                            ctx.set_state(ActorLifecycle::Failed);
                        }
                    }
                }
                // State: STARTED
                ActorLifecycle::Started => {
                    debug!("Actor {} is started.", &self.path);
                    self.run(&mut ctx).await;
                    if ctx.error().is_none()
                        || ctx.state() == &ActorLifecycle::Stopped
                    {
                        ctx.set_state(ActorLifecycle::Stopped);
                    } else {
                        ctx.set_state(ActorLifecycle::Failed);
                    }
                }
                // State: RESTARTED
                ActorLifecycle::Restarted => {
                    // Apply supervision strategy.
                    self.apply_supervision_strategy(
                        A::supervision_strategy(),
                        &mut ctx,
                        &mut retries,
                    )
                    .await;
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
                    if self.parent_sender.is_none() {
                        ctx.set_state(ActorLifecycle::Restarted);
                    }
                }
                // State: TERMINATED
                ActorLifecycle::Terminated => {
                    debug!("Actor {} is terminated.", &self.path);
                    ctx.system().remove_actor(&self.path.clone()).await;
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
                        debug!("Actor {} is stopped.", &self.path);
                        ctx.set_state(ActorLifecycle::Stopped);
                        break;
                    }
                }
                // Handle error from `ErrorBoxReceiver`.
                error =  self.error_receiver.recv() => {
                    if let Some(error) = error {
                        match error {
                            ChildError::Error { error } => self.actor.on_child_error(error, ctx).await,
                            ChildError::Fault { error, sender } => {
                                let action = self.actor.on_child_fault(error, ctx).await;
                                if sender.send(action).is_err() {
                                    error!("Can not send action to child!");
                                }
                            },
                        }
                    } else {
                        ctx.set_state(ActorLifecycle::Stopped);
                        break;
                    }
                }
                // Handle inner event from `inner_receiver`.
                recv = self.inner_receiver.recv() => {
                    if let Some(event) = recv {
                        self.inner_handle(event, ctx).await;
                    } else {
                        break;
                    }
                }
                _ = self.token.cancelled() => {
                    debug!("Actor {} is stopped.", &self.path);
                    ctx.set_state(ActorLifecycle::Stopped);
                    break;
                }
            }
        }
    }

    /// Inner message handler.
    async fn inner_handle(
        &mut self,
        event: InnerMessage<A>,
        ctx: &mut ActorContext<A>,
    ) {
        match event {
            InnerMessage::Event { event, publish } => {
                if publish {
                    // Publish event to subscribers.
                    match self.event_sender.send(event.clone()) {
                        Ok(size) => {
                            debug!(
                                "Event sent successfully to {} subscribers.",
                                size
                            );
                        }
                        Err(_err) => {
                            error!("Failed to send event");
                        }
                    }
                }
                // Handle inner event.
                self.actor.on_event(event, ctx).await;
            }
            InnerMessage::Message(msg) => {
                // Handle inner message.
                self.actor.on_message(msg, ctx).await;
            }
            InnerMessage::Error(error) => {
                if let Some(parent_helper) = self.parent_sender.as_mut() {
                    // Send error to parent.
                    parent_helper
                        .send(ChildError::Error { error })
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send error to parent actor: {:?}",
                                err
                            );
                        });
                }
            }
            InnerMessage::Fail(error) => {
                // If the actor has a parent, send the fail to the parent.
                if let Some(parent_helper) = self.parent_sender.as_mut() {
                    let (action_sender, action_receiver) = oneshot::channel();
                    //self.action_receiver = Some(action_receiver);
                    parent_helper
                        .send(ChildError::Fault {
                            error,
                            sender: action_sender,
                        })
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send fail to parent actor: {:?}",
                                err
                            );
                        });
                    // Sets the state from action.
                    if let Ok(action) = action_receiver.await {
                        match action {
                            ChildAction::Stop => {
                                ctx.set_state(ActorLifecycle::Stopped);
                                ctx.stop().await;
                            }
                            ChildAction::Start => {
                                ctx.set_state(ActorLifecycle::Started);
                                self.token.cancel();
                            }
                            ChildAction::Restart => {
                                ctx.set_state(ActorLifecycle::Restarted);
                                self.token.cancel();
                            }
                            ChildAction::Delegate => {
                                ctx.set_state(ActorLifecycle::Failed);
                                self.token.cancel();
                            }
                        }
                    }
                } else {
                    // If the actor has no parent, set the state to stopped.
                    ctx.set_state(ActorLifecycle::Stopped);
                    ctx.stop().await;
                }
            } 
        }
    }

    /// Apply supervision strategy.
    /// If the actor fails, the strategy is applied.
    ///
    async fn apply_supervision_strategy(
        &mut self,
        strategy: SupervisionStrategy,
        ctx: &mut ActorContext<A>,
        retries: &mut usize,
    ) {
        match strategy {
            SupervisionStrategy::Stop => {
                error!("Actor '{}' failed to start!", &self.path);
                ctx.set_state(ActorLifecycle::Stopped);
            }
            SupervisionStrategy::Retry(mut retry_strategy) => {
                debug!(
                    "Restarting actor with retry strategy: {:?}",
                    &retry_strategy
                );
                if *retries < retry_strategy.max_retries() {
                    debug!("retries: {}", &retries);
                    if let Some(duration) = retry_strategy.next_backoff() {
                        debug!("Backoff for {:?}", &duration);
                        tokio::time::sleep(duration).await;
                    }
                    *retries += 1;
                    let error = ctx.error();
                    match ctx.restart(&mut self.actor, error.as_ref()).await {
                        Ok(_) => {
                            ctx.set_state(ActorLifecycle::Started);
                            *retries = 0;
                            let token = CancellationToken::new();
                            ctx.set_token(token.clone());
                            self.token = token;
                        }
                        Err(err) => {
                            ctx.set_error(err);
                        }
                    }
                } else {
                    ctx.set_state(ActorLifecycle::Stopped);
                }
            }
        }
    }
}

/// Inner message.
#[derive(Debug, Clone)]
pub enum InnerMessage<A: Actor> {
    /// Event
    Event { event: A::Event, publish: bool },
    /// Message
    Message(A::Message),
    /// Error
    Error(Error),
    /// Fail
    Fail(Error),
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::{
        actor::{Actor, ActorContext, Event, Handler, Message, Response},
        supervision::{FixedIntervalStrategy, Strategy, SupervisionStrategy},
        system::SystemRef,
        Error,
    };
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use tokio::sync::RwLock;

    use tracing_test::traced_test;

    use std::{collections::HashMap, sync::Arc, time::Duration};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestMessage(ErrorMessage);

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ErrorMessage {
        Stop,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestResponse;

    impl Response for TestResponse {}

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
        type Response = TestResponse;
        type Event = TestEvent;

        fn supervision_strategy() -> SupervisionStrategy {
            SupervisionStrategy::Retry(Strategy::FixedInterval(
                FixedIntervalStrategy::new(3, Duration::from_secs(1)),
            ))
        }

        async fn pre_start(
            &mut self,
            _ctx: &mut ActorContext<Self>,
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

        async fn post_stop(
            &mut self,
            _ctx: &mut ActorContext<Self>,
        ) -> Result<(), Error> {
            debug!("Post stop");
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            msg: TestMessage,
            ctx: &mut ActorContext<Self>,
        ) -> Result<TestResponse, Error> {
            debug!("Handling empty message");
            match msg {
                TestMessage(ErrorMessage::Stop) => {
                    ctx.stop().await;
                    debug!("Actor stopped");
                }
            }
            Ok(TestResponse)
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_actor_root_failed() {
        let (event_sender, _) = mpsc::channel(100);
        let actors = Arc::new(RwLock::new(HashMap::new()));
        let helpers = Arc::new(RwLock::new(HashMap::new()));
        let system = SystemRef::new(actors, helpers, event_sender);

        let actor = TestActor { failed: false };
        let (mut runner, actor_ref) =
            ActorRunner::create(ActorPath::from("/user/test"), actor, None);
        let inner_system = system.clone();
        // Init the actor runner.
        tokio::spawn(async move {
            runner.init(inner_system).await;
        });
        tokio::time::sleep(Duration::from_secs(1)).await;

        actor_ref
            .tell(TestMessage(ErrorMessage::Stop))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(1)).await;

        assert!(logs_contain("Actor /user/test is terminated"));

        assert!(system
            .get_actor::<TestActor>(&ActorPath::from("/user/test"))
            .await
            .is_none());

        let actor = TestActor { failed: true };

        let (mut runner, actor_ref) =
            ActorRunner::create(ActorPath::from("/user/test"), actor, None);
        let inner_system = system.clone();
        // Init the actor runner.
        tokio::spawn(async move {
            runner.init(inner_system).await;
        });

        tokio::time::sleep(Duration::from_secs(2)).await;

        assert!(logs_contain("Creating new actor runner"));
        assert!(logs_contain("Creating new handle reference"));
        assert!(logs_contain("Initializing actor /user/test runner"));
        assert!(logs_contain("Creating actor /user/test context"));
        assert!(logs_contain("Actor /user/test is created"));
        assert!(logs_contain("Actor /user/test failed to start"));
        assert!(logs_contain("Actor /user/test is faulty"));
        assert!(logs_contain("Restarting actor with retry strategy"));
        assert!(logs_contain("Actor /user/test is started"));
        assert!(logs_contain("Running actor /user/test"));

        actor_ref
            .tell(TestMessage(ErrorMessage::Stop))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(logs_contain("Actor /user/test is terminated"));

        assert!(system
            .get_actor::<TestActor>(&ActorPath::from("/user/test"))
            .await
            .is_none());
    }
}
