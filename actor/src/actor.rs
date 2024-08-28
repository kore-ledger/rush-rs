// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor
//!
//! The `actor` module provides the `Actor` trait and the `ActorRef` type. The `Actor` trait is the
//! main trait that actors must implement. The `ActorRef` type is a reference to an actor that can
//! be used to send messages to him.
//!

use crate::{
    handler::HandleHelper,
    runner::{ChildSender, InnerMessage, InnerSender},
    supervision::SupervisionStrategy,
    system::SystemRef,
    ActorPath, Error,
};

use tokio::sync::{broadcast::Receiver as EventReceiver, mpsc, oneshot};

use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use tracing::debug;

use std::fmt::Debug;

/// The `ActorContext` is the context of the actor.
/// It is passed to the actor when it is started, and can be used to interact with the actor
/// system.
pub struct ActorContext<A: Actor> {
    /// The path of the actor.
    path: ActorPath,
    /// The actor system.
    system: SystemRef,
    /// The actor lifecycle.
    lifecycle: ActorLifecycle,
    /// Error in the actor.
    error: Option<Error>,
    /// The error sender to send errors to the parent.
    error_sender: ChildErrorSender,
    /// Inner sender.
    inner_sender: InnerSender<A>,
    /// Child action senders.
    child_senders: Vec<ChildSender>,
    /// Cancellation token.
    token: CancellationToken,
}

impl<A> ActorContext<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor context.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor.
    /// * `system` - The actor system.
    /// * `error_sender` - The error sender helper.
    /// * `token` - The cancellation token.
    /// * `event_sender` - The event sender.
    ///
    /// # Returns
    ///
    /// Returns a new actor context.
    ///
    pub(crate) fn new(
        path: ActorPath,
        system: SystemRef,
        token: CancellationToken,
        error_sender: ChildErrorSender,
        inner_sender: InnerSender<A>,
    ) -> Self {
        Self {
            path,
            system,
            lifecycle: ActorLifecycle::Created,
            error: None,
            error_sender,
            inner_sender,
            child_senders: Vec::new(),
            token,
        }
    }

    /// Restart the actor.
    pub(crate) async fn restart(
        &mut self,
        actor: &mut A,
        error: Option<&Error>,
    ) -> Result<(), Error>
    where
        A: Actor,
    {
        actor.pre_restart(self, error).await
    }

    /// Returns the actor reference.
    ///
    /// # Returns
    ///
    /// Returns the actor reference. `None` if the actor is removed.
    ///
    pub async fn reference(&self) -> Option<ActorRef<A>> {
        self.system.get_actor(&self.path).await
    }

    /// Returns the path of the actor.
    ///
    /// # Returns
    ///
    /// Returns the path of the actor.
    ///
    pub fn path(&self) -> &ActorPath {
        &self.path
    }

    /// Returns the actor system.
    ///
    /// # Returns
    ///
    /// Returns the actor system.
    ///
    pub fn system(&self) -> &SystemRef {
        &self.system
    }

    /// Returns the actor parent or None if the actor is root actor.
    ///
    /// # Returns
    ///
    /// Returns the actor parent or None if the actor is root actor.
    ///
    pub async fn parent<P: Actor + Handler<P>>(&self) -> Option<ActorRef<P>> {
        self.system.get_actor(&self.path().parent()).await
    }

    /// Emits an event to subscribers.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to emit.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the event could not be emitted.
    ///
    pub async fn publish_event(&self, event: A::Event) -> Result<(), Error> {
        self.inner_sender
            .send(InnerMessage::Event {
                event,
                publish: true,
            })
            .map_err(|_| Error::SendEvent)
    }

    /// Emits an event to inner handler.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to emit.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the event could not be emitted.
    ///
    pub async fn event(&self, event: A::Event) -> Result<(), Error> {
        self.inner_sender
            .send(InnerMessage::Event {
                event,
                publish: false,
            })
            .map_err(|_| Error::SendEvent)
    }

    /// Emits a message to inner handler.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to emit.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the message could not be emitted.
    ///
    pub async fn message(&self, message: A::Message) -> Result<(), Error> {
        self.inner_sender
            .send(InnerMessage::Message(message))
            .map_err(|_| Error::Send("Message".to_string()))
    }

    /// Emits an error.
    ///
    /// # Arguments
    ///
    /// * `error` - The error to emit.
    ///
    /// # Returns
    ///
    /// Void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the error could not be emitted.
    ///
    pub async fn emit_error(&mut self, error: Error) -> Result<(), Error> {
        self.inner_sender
            .send(InnerMessage::Error(error))
            .map_err(|_| Error::Send("Error".to_string()))
    }

    /// Emits a fail.
    /// This is used to emit a fail in a child actor.
    ///
    /// # Arguments
    ///
    /// * `error` - The error to emit.
    ///
    /// # Returns
    ///
    /// Void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the fail could not be emitted.
    ///
    pub async fn emit_fail(&mut self, error: Error) -> Result<(), Error> {
        // Send fail to parent actor.
        //self.token.cancel();
        self.inner_sender
            .send(InnerMessage::Fail(error.clone()))
            .map_err(|_| Error::Send("Error".to_string()))
    }

    /// Stop the actor.
    pub async fn stop(&mut self) {
        while let Some(sender) = self.child_senders.pop() {
            let _ = sender.send(ChildAction::Stop);
        }
        self.set_state(ActorLifecycle::Stopped);
        self.token.cancel();
        self.system.remove_actor(self.path()).await;
    }

    /// Create a child actor under this actor.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the child actor.
    /// * `actor` - The actor to create.
    ///
    /// # Returns
    ///
    /// Returns the actor reference of the child actor.
    ///
    /// # Errors
    ///
    /// Returns an error if the child actor could not be created.
    ///
    pub async fn create_child<C>(
        &mut self,
        name: &str,
        actor: C,
    ) -> Result<ActorRef<C>, Error>
    where
        C: Actor + Handler<C>,
    {
        let path = self.path.clone() / name;
        let (actor_ref, sender) = self
            .system
            .create_actor_path(path, actor, Some(self.error_sender.clone()))
            .await?;
        self.child_senders.push(sender);
        Ok(actor_ref)
    }

    /// Retrieve a child actor running under this actor.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the child actor.
    ///
    /// # Returns
    ///
    /// Returns the actor reference of the child actor if it exists.
    ///
    pub async fn get_child<C>(&self, name: &str) -> Option<ActorRef<C>>
    where
        C: Actor + Handler<C>,
    {
        let path = self.path.clone() / name;
        self.system.get_actor(&path).await
    }

    /// Retrieve or create a new child under this actor if it does not exist yet.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the child actor.
    /// * `actor_fn` - The function to create the actor if it does not exist.
    ///
    /// # Returns
    ///
    /// Returns the actor reference of the child actor.
    ///
    /// # Errors
    ///
    /// Returns an error if the child actor could not be created.
    ///
    pub async fn get_or_create_child<C, F>(
        &mut self,
        name: &str,
        actor_fn: F,
    ) -> Result<ActorRef<C>, Error>
    where
        C: Actor + Handler<C>,
        F: FnOnce() -> C,
    {
        let path = self.path.clone() / name;
        let (actor_ref, sender) = self
            .system
            .get_or_create_actor_path(
                &path,
                Some(self.error_sender.clone()),
                actor_fn,
            )
            .await?;
        if let Some(sender) = sender {
            self.child_senders.push(sender);
        }
        Ok(actor_ref)
    }

    /// Returns the lifecycle state of the actor.
    /// The lifecycle state can be one of the following:
    /// - `ActorLifecycle::Started` - The actor is started.
    /// - `ActorLifecycle::Failed` - The actor is faulty.
    /// - `ActorLifecycle::Stopped` - The actor is stopped.
    /// - `ActorLifecycle::Terminated` - The actor is terminated.
    ///
    /// # Returns
    ///
    /// Returns the lifecycle state of the actor.
    ///
    pub fn state(&self) -> &ActorLifecycle {
        &self.lifecycle
    }

    /// Sets the lifecycle state of the actor.
    ///
    /// # Arguments
    ///
    /// * `state` - The lifecycle state of the actor.
    ///
    pub(crate) fn set_state(&mut self, state: ActorLifecycle) {
        self.lifecycle = state;
    }

    /// Returns the error of the actor.
    ///
    /// # Returns
    ///
    /// Returns the error of the actor.
    ///
    pub(crate) fn error(&self) -> Option<Error> {
        self.error.clone().or(None)
    }

    /// Sets the error of the actor.
    ///
    /// # Arguments
    ///
    /// * `error` - The error of the actor.
    ///
    pub(crate) fn set_error(&mut self, error: Error) {
        self.error = Some(error);
    }

    /// Sets the cancelation token.
    ///
    /// # Arguments
    ///
    /// * `token` - The cancelation token.
    ///
    pub fn set_token(&mut self, token: CancellationToken) {
        self.token = token;
    }
}

/// The `Actor` lifecycle enum
#[derive(Debug, Clone, PartialEq)]
pub enum ActorLifecycle {
    /// The actor is created.
    Created,
    /// The actor is started.
    Started,
    /// The actor is restarted.
    Restarted,
    /// The actor is failed.
    Failed,
    /// The actor is stopped.
    Stopped,
    /// The actor is terminated.
    Terminated,
}

/// The action that a child actor will take when an error occurs.
#[derive(Debug, Clone)]
pub enum ChildAction {
    /// The child actor will stop.
    Stop,
    /// The child actor will start.
    Start,
    /// The child actor will restart.
    Restart,
    /// Delegate the action to the child supervision strategy.
    Delegate,
}

/// Child error receiver.
pub(crate) type ChildErrorReceiver = mpsc::UnboundedReceiver<ChildError>;

/// Child error sender.
pub(crate) type ChildErrorSender = mpsc::UnboundedSender<ChildError>;

/// Child error.
///
pub enum ChildError {
    /// Error in child.
    Error {
        /// The error that caused the failure.
        error: Error,
    },
    /// Fault in child.
    Fault {
        /// The error that caused the failure.
        error: Error,
        /// The sender will communicate the action to be carried out to the child.
        sender: oneshot::Sender<ChildAction>,
    },
}

/// The `Actor` trait is the main trait that actors must implement.
#[async_trait]
pub trait Actor: Send + Sync + Sized + 'static {
    /// The `Message` type is the type of the messages that the actor can receive.
    type Message: Message;

    /// The `Event` type is the type of the events that the actor can emit.
    type Event: Event;

    /// The `Response` type is the type of the response that the actor can give when it receives a
    /// message.
    type Response: Response;

    /// Defines the supervision strategy to use for this actor. By default it is
    /// `Stop` which simply stops the actor if an error occurs at startup or when an
    /// error or fault is issued from a handler. You can also set this to
    /// [`SupervisionStrategy::Retry`] with a chosen [`supervision::RetryStrategy`].
    ///
    /// # Returns
    ///
    /// Returns the supervision strategy to use for this actor.
    ///
    fn supervision_strategy() -> SupervisionStrategy {
        SupervisionStrategy::Stop
    }

    /// Called when the actor is started.
    /// Override this method to perform initialization when the actor is started.
    ///
    /// # Arguments
    ///
    /// * `context` - The context of the actor.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the actor could not be started.
    ///
    async fn pre_start(
        &mut self,
        _context: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Override this function if you want to define what should happen when an
    /// error occurs in [`Actor::pre_start()`]. By default it simply calls
    /// `pre_start()` again, but you can also choose to reinitialize the actor
    /// in some other way.
    async fn pre_restart(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _error: Option<&Error>,
    ) -> Result<(), Error> {
        self.pre_start(ctx).await
    }

    /// Called before stopping the actor.
    /// Override this method to define what should happen before the actor is stopped.
    /// By default it does nothing.
    ///
    /// # Arguments
    ///
    /// * `context` - The context of the actor.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the actor could not be stopped.
    ///
    async fn pre_stop(
        &mut self,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Called when the actor is stopped.
    ///
    /// # Arguments
    ///
    /// * `context` - The context of the actor.
    ///
    async fn post_stop(
        &mut self,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Create event from response.
    fn from_response(_response: Self::Response) -> Result<Self::Event, Error> {
        Err(Error::Functional("Not implemented".to_string()))
    }
}

/// Events that this actor will emit after processing a message. The events emitted by a message
/// handler will be used to apply the event sourcing pattern.
pub trait Event:
    Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static
{
}

/// Defines what an actor will receive as its message, and with what it should respond.
pub trait Message: Clone + Send + Sync + 'static {}

/// Defines the response of a message.
pub trait Response: Send + Sync + 'static {}

/// This is the trait that allows an actor to handle the messages that they receive and,
/// if necessary, respond to them.
#[async_trait]
pub trait Handler<A: Actor + Handler<A>>: Send + Sync {
    /// Handles a message.
    ///
    /// # Arguments
    ///
    /// * `sender` - The `ActorPath` of the sender of the message.
    /// * `msg` - The message to handle.
    /// * `ctx` - The actor context.
    ///
    /// # Returns
    ///
    /// Returns the response of the message (if any).
    ///
    /// # Errors
    ///
    /// Returns an error if the message could not be handled.
    async fn handle_message(
        &mut self,
        sender: ActorPath,
        msg: A::Message,
        ctx: &mut ActorContext<A>,
    ) -> Result<A::Response, Error>;

    /// Internal event.
    /// Override this method to define what should happen when an internal event is emitted by the
    /// actor.
    /// By default it does nothing.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to handle.
    /// * `ctx` - The actor context.
    ///
    async fn on_event(&mut self, _event: A::Event, _ctx: &mut ActorContext<A>) {
        // Default implementation.
    }

    /// Internal message.
    /// Override this method to define what should happen when an internal message is emitted by the
    /// actor.
    /// By default it does nothing.
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to handle.
    /// * `ctx` - The actor context.
    ///
    async fn on_message(
        &mut self,
        _msg: A::Message,
        _ctx: &mut ActorContext<A>,
    ) {
        // Default implementation.
    }

    /// Called when an error occurs in a child actor.
    /// Override this method to define what should happen when an error occurs in a child actor.
    /// By default it does nothing.
    ///
    /// # Arguments
    ///
    /// * `error` - The error that occurred.
    /// * `ctx` - The actor context.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the error could not be handled.
    ///
    async fn on_child_error(
        &mut self,
        error: Error,
        _ctx: &mut ActorContext<A>,
    ) {
        debug!("Handling error: {:?}", error);
        // Default implementation from child actor errors.
        //self.on_child_fault(error, ctx).await;
    }

    /// Called when a fault occurs in a child actor.
    /// Override this method to define what should happen when a fault occurs in a child actor.
    /// By default it does nothing.
    ///
    /// # Arguments
    ///
    /// * `error` - The error that occurred.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the fault could not be handled.
    ///
    async fn on_child_fault(
        &mut self,
        error: Error,
        _ctx: &mut ActorContext<A>,
    ) -> ChildAction {
        // Default implementation from child actor errors.
        debug!("Handling fault: {:?}", error);
        ChildAction::Stop
    }
}

/// Actor reference.
///
/// This is a reference to an actor that can be used to send messages to him.
///
pub struct ActorRef<A>
where
    A: Actor + Handler<A>,
{
    /// The path of the actor.
    path: ActorPath,
    /// The handle helper.
    sender: HandleHelper<A>,
    /// The actor event receiver.
    event_receiver: EventReceiver<<A as Actor>::Event>,
}

impl<A> ActorRef<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor reference.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor.
    /// * `sender` - The handle helper.
    ///
    /// # Returns
    ///
    /// Returns a new actor reference.
    ///
    pub fn new(
        path: ActorPath,
        sender: HandleHelper<A>,
        event_receiver: EventReceiver<<A as Actor>::Event>,
    ) -> Self {
        Self {
            path,
            sender,
            event_receiver,
        }
    }

    /// Tells a message to the actor.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to send.
    ///
    /// # Returns
    ///
    /// Returns a () if success.
    ///
    pub async fn tell(&self, message: A::Message) -> Result<(), Error> {
        self.sender.tell(self.path(), message).await
    }

    /// Asks a message to the actor.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to send.
    ///
    /// # Returns
    ///
    /// Returns the response of the message.
    ///
    /// # Errors
    ///
    /// Returns an error if the message could not be sent.
    ///
    pub async fn ask(&self, message: A::Message) -> Result<A::Response, Error> {
        self.sender.ask(self.path(), message).await
    }

    /// Stops the actor.
    /// This will stop the actor and remove it from the actor system.
    /// The actor will not be able to receive any more messages.
    ///
    pub async fn stop(&self) {
        self.sender.stop(self.path()).await;
    }

    /// Returns the path of the actor.
    ///
    /// # Returns
    ///
    /// Returns the path of the actor.
    ///
    pub fn path(&self) -> ActorPath {
        self.path.clone()
    }

    /// Returns true if the sender is closed.
    ///
    /// # Returns
    ///
    /// Returns true if the sender is closed.
    ///
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Subscribes to the actor event bus.
    /// This will return an event receiver that can be used to receive events from the actor.
    /// The event receiver will receive events that the actor emits after processing a message.
    ///
    /// # Returns
    ///
    /// Returns an event receiver.
    ///
    pub fn subscribe(&self) -> EventReceiver<<A as Actor>::Event> {
        self.event_receiver.resubscribe()
    }
}

/// Clone implementation for ActorRef.
impl<A> Clone for ActorRef<A>
where
    A: Actor + Handler<A>,
{
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            sender: self.sender.clone(),
            event_receiver: self.event_receiver.resubscribe(),
        }
    }
}

/// Dummy actor.
#[derive(Debug, Clone)]
pub struct DummyActor;

/// Dummy message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyMessage;

impl Message for DummyMessage {}

/// Dummy response.
#[derive(Debug, Clone)]
pub struct DummyResponse;

impl Response for DummyResponse {}

/// Dummy event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyEvent;

impl Event for DummyEvent {}

impl Actor for DummyActor {
    type Message = DummyMessage;
    type Response = DummyResponse;
    type Event = DummyEvent;
}

#[async_trait]
impl Handler<DummyActor> for DummyActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        _message: DummyMessage,
        _ctx: &mut ActorContext<DummyActor>,
    ) -> Result<DummyResponse, Error> {
        Ok(DummyResponse)
    }

    async fn on_child_error(
        &mut self,
        error: Error,
        _ctx: &mut ActorContext<DummyActor>,
    ) {
        // Default implementation from child actor errors.
        assert_eq!(error, Error::Send("Error".to_string()));
    }
}

#[cfg(test)]
mod test {

    use super::*;

    use crate::sink::{Sink, Subscriber};

    use serde::{Deserialize, Serialize};
    use tokio::sync::{mpsc, RwLock};

    use tracing_test::traced_test;

    use std::{collections::HashMap, sync::Arc};

    #[derive(Debug, Clone)]
    struct TestActor {
        counter: usize,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestMessage(usize);

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestResponse(usize);

    impl Response for TestResponse {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestEvent(usize);

    impl Event for TestEvent {}

    #[async_trait]
    impl Actor for TestActor {
        type Message = TestMessage;
        type Event = TestEvent;
        type Response = TestResponse;
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: TestMessage,
            ctx: &mut ActorContext<TestActor>,
        ) -> Result<TestResponse, Error> {
            let value = msg.0;
            self.counter += value;
            ctx.publish_event(TestEvent(self.counter)).await.unwrap();
            Ok(TestResponse(self.counter))
        }
    }

    pub struct TestSubscriber;

    impl Subscriber<TestEvent> for TestSubscriber {
        fn notify(&self, event: TestEvent) {
            debug!("Received event: {:?}", event);
            assert!(event.0 > 0);
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_actor() {
        let (event_sender, _event_receiver) = mpsc::channel(100);
        let actors = Arc::new(RwLock::new(HashMap::new()));
        let helpers = Arc::new(RwLock::new(HashMap::new()));
        let senders = Arc::new(RwLock::new(Vec::new()));
        let system = SystemRef::new(actors, helpers, senders, event_sender);
        let actor = TestActor { counter: 0 };
        let actor_ref = system.create_root_actor("test", actor).await.unwrap();

        let sink = Sink::new(actor_ref.subscribe(), TestSubscriber);
        system.run_sink(sink).await;

        actor_ref.tell(TestMessage(10)).await.unwrap();
        let mut recv = actor_ref.subscribe();
        let response = actor_ref.ask(TestMessage(10)).await.unwrap();
        assert_eq!(response.0, 20);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 10);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 20);
        actor_ref.stop().await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
