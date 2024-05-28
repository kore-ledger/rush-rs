// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor
//!
//! The `actor` module provides the `Actor` trait and the `ActorRef` type. The `Actor` trait is the
//! main trait that actors must implement. The `ActorRef` type is a reference to an actor that can
//! be used to send messages to him.
//!

use crate::{
    handler::HandleHelper, supervision::SupervisionStrategy, ActorPath,
    ActorSystem, Error,
};

use tokio::sync::broadcast::{
    Receiver as EventReceiver, Sender as EventSender,
};

use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use serde::{de::DeserializeOwned, Serialize};

use std::{fmt::Debug, marker::PhantomData};

/// The `ActorContext` is the context of the actor.
/// It is passed to the actor when it is started, and can be used to interact with the actor
/// system.
pub struct ActorContext<A: Actor> {
    /// The path of the actor.
    path: ActorPath,
    /// The actor system.
    system: ActorSystem,
    /// The event sender.
    event_sender: EventSender<A::Event>,
    /// The actor lifecycle.
    lifecycle: ActorLifecycle,
    /// Error in the actor.
    error: Option<Error>,
    /// Cancellation token.
    token: CancellationToken,
    /// Phantom data for the actor type.
    phantom_a: PhantomData<A>,
}

impl<A: Actor> ActorContext<A> {
    /// Creates a new actor context.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor.
    /// * `system` - The actor system.
    /// * `token` - The cancellation token.
    /// * `event_sender` - The event sender.
    ///
    /// # Returns
    ///
    /// Returns a new actor context.
    ///
    pub fn new(
        path: ActorPath,
        system: ActorSystem,
        token: CancellationToken,
        event_sender: EventSender<<A as Actor>::Event>,
    ) -> Self {
        Self {
            path,
            system,
            event_sender,
            lifecycle: ActorLifecycle::Created,
            error: None,
            token,
            phantom_a: PhantomData,
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
    pub fn system(&self) -> &ActorSystem {
        &self.system
    }

    /// Emits an event.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to emit.
    ///
    /// # Returns
    ///
    /// Returns the number of subscribers that received the event.
    ///
    /// # Errors
    ///
    /// Returns an error if the event could not be emitted.
    ///
    pub async fn emit(&self, event: A::Event) -> Result<usize, Error> {
        self.event_sender.send(event).map_err(|_| Error::SendEvent)
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
        &self,
        name: &str,
        actor: C,
    ) -> Result<ActorRef<C>, Error>
    where
        C: Actor + Handler<C>,
    {
        let path = self.path.clone() / name;
        self.system.create_actor_path(path, actor).await
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
        &self,
        name: &str,
        actor_fn: F,
    ) -> Result<ActorRef<C>, Error>
    where
        C: Actor + Handler<C>,
        F: FnOnce() -> C,
    {
        let path = self.path.clone() / name;
        self.system.get_or_create_actor_path(&path, actor_fn).await
    }

    /// Stops the child actor.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the child actor.
    ///
    pub async fn stop_child(&self, name: &str) {
        let path = self.path.clone() / name;
        self.system.stop_actor(&path).await;
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
    pub fn set_state(&mut self, state: ActorLifecycle) {
        self.lifecycle = state;
    }

    /// Returns the error of the actor.
    ///
    /// # Returns
    ///
    /// Returns the error of the actor.
    ///
    pub fn error(&self) -> Option<Error> {
        self.error.clone().or(None)
    }

    /// Sets the error of the actor.
    ///
    /// # Arguments
    ///
    /// * `error` - The error of the actor.
    ///
    pub fn failed(&mut self, error: Error) {
        self.error = Some(error);
        self.token.cancel();
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
    /// The actor is failed.
    Failed,
    /// The actor is stopped.
    Stopped,
    /// The actor is terminated.
    Terminated,
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
    type Response: Debug + Send + Sync + 'static;

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
        _context: &ActorContext<Self>,
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
pub trait Response: Debug + Send + Sync + 'static {}

/// This is the trait that allows an actor to handle the messages that they receive and,
/// if necessary, respond to them.
#[async_trait]
pub trait Handler<A: Actor>: Send + Sync {
    /// Handles a message.
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to handle.
    /// * `ctx` - The actor context.
    ///
    /// # Returns
    ///
    /// Returns the response of the message (if any).
    ///
    async fn handle(
        &mut self,
        msg: A::Message,
        ctx: &mut ActorContext<A>,
    ) -> A::Response;
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
        self.sender.tell(message).await
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
        self.sender.ask(message).await
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

#[cfg(test)]
mod test {

    use super::*;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone)]
    struct TestActor {
        counter: usize,
    }

    #[derive(Debug, Clone)]
    struct TestMessage(usize);

    impl Message for TestMessage {}

    #[derive(Debug, Clone)]
    struct TestResponse(usize);

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
        async fn handle(
            &mut self,
            msg: TestMessage,
            ctx: &mut ActorContext<TestActor>,
        ) -> TestResponse {
            let value = msg.0;
            self.counter += value;
            ctx.emit(TestEvent(self.counter)).await.unwrap();
            TestResponse(self.counter)
        }
    }

    #[tokio::test]
    async fn test_actor() {
        let system = ActorSystem::default();
        let actor = TestActor { counter: 0 };
        let actor_ref = system.create_actor("test", actor).await.unwrap();
        actor_ref.tell(TestMessage(10)).await.unwrap();
        let mut recv = actor_ref.subscribe();
        let response = actor_ref.ask(TestMessage(10)).await.unwrap();
        assert_eq!(response.0, 20);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 10);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 20);
    }
}
