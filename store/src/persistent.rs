// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Persistent Actor
//!
//! The `persistent` module provides the `PersistentActor` type. The `PersistentActor` type is the responsible for
//! creating and managing actors that persist their state. The state is stored in a `Store`. The `Store` is a key-value
//! store that can be implemented by the user. The `PersistentActor` trait extends the `Actor` trait and adds the
//! capability to persist the state of the actor.
//!

use crate::{
    store::{Store, StoreManager},
    Error,
};
use actor::{actor::Handler, Actor, ActorContext, Event, Message};
use async_trait::async_trait;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use std::{
    fmt::Debug,
    marker::PhantomData,
};

/// A trait representing a persistent actor.
pub trait PersistentActor: Actor<E> + Debug + Clone + Serialize + DeserializeOwned {}

/// Commands that can be sent to a persistent actor.
#[derive(Debug, Clone)]
pub enum Command<E: Event, A: PersistentActor<E>> {
    /// Persist an event.
    Persist(E),
    /// Snapshot the state.
    Snapshot(A),
    /// Recover the state.
    Recover,
}

/// Response to a command.
#[derive(Debug, Clone)]
pub enum Response<A>
where
    A: Debug + Clone,
{
    /// The event was persisted.
    Persisted,
    /// The state was snapshoted.
    Snapshoted,
    /// The state was recovered.
    Recovered(A),
    /// An error occurred.
    Error(Error),
}

impl<E, A> Message for Command<E, A>
where
    E: Event,
    A: PersistentActor<E> + Debug + Clone,
{
    type Response = Response<A>;
}

/// Events emitted by a persistent actor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PersistentEvent {
    /// An event was persisted.
    Persisted,
    /// An event was snapshoted.
    Snapshoted,
    /// An event was recovered.
    Recovered,
    /// An error occurred.
    Error(Error),
}

impl Event for PersistentEvent {}

/// A persistent actor.
pub struct Persistent<E, A>
where
    E: Event,
    A: PersistentActor<E> + Debug + Clone,
{
    event_counter: usize,
    state_counter: usize,
    events: Box<dyn Store>,
    states: Box<dyn Store>,
    _phantom_event: PhantomData<E>,
    _phantom_actor: PhantomData<A>,
}

impl<E, A> Persistent<E, A>
where
    E: Event,
    A: PersistentActor<E> + Debug + Clone,
{
    /// Creates a new persistent actor.
    ///
    /// # Arguments
    ///
    /// - name: The name of the actor.
    /// - manager: The store manager.
    ///
    pub fn new<S>(
        name: &str,
        manager: impl StoreManager<S>,
    ) -> Result<Self, Error>
    where
        S: Store + 'static,
    {
        let events = manager.create_store(&format!("{}_events", name))?;
        let states = manager.create_store(&format!("{}_states", name))?;
        Ok(Self {
            event_counter: 0,
            state_counter: 0,
            events: Box::new(events),
            states: Box::new(states),
            _phantom_event: PhantomData,
            _phantom_actor: PhantomData,
        })
    }

    /// Persist an event.
    ///
    /// # Arguments
    ///
    /// - event: The event to persist.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    fn persist(&mut self, event: E) -> Result<(), Error> {
        let bytes = bincode::serialize(&event).map_err(|e| {
            Error::Store(format!("Can't serialize event: {}", e))
        })?;
        self.events.put(&self.event_counter.to_string(), &bytes)
    }

    /// Snapshot the state.
    ///
    /// # Arguments
    ///
    /// - actor: The actor to snapshot.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    fn snapshot(&mut self, actor: A) -> Result<(), Error> {
        let bytes = bincode::serialize(&actor).map_err(|e| {
            Error::Store(format!("Can't serialize state: {}", e))
        })?;
        self.states.put(&self.state_counter.to_string(), &bytes)
    }

    /// Recover the state.
    /// 
    /// # Returns
    /// 
    /// The recovered state.
    /// 
    /// An error if the operation failed.
    /// 
    fn recover(&mut self) -> Result<A, Error> {
        let (key, _) = self.events.last().map_err(|e| {
            Error::Store(format!("Can't get last event: {}", e))
        })?;
        self.event_counter = key.parse().map_err(|e| {
            Error::Store(format!("Can't parse event key: {}", e))
        })?;
        let (key, data) = self.states.last().map_err(|e| {
            Error::Store(format!("Can't get last state: {}", e))
        })?;
        self.state_counter = key.parse().map_err(|e| {
            Error::Store(format!("Can't parse state key: {}", e))
        })?;
        let state: A = bincode::deserialize(&data).map_err(|e| {
            Error::Store(format!("Can't deserialize state: {}", e))
        })?;
        Ok(state)
    }
}

/// Actor for a persistent actor.
#[async_trait]
impl<E, A> Actor<PersistentEvent> for Persistent<E, A>
where
    E: Event,
    A: PersistentActor<E> + Debug + Clone,
{
    fn apply_event(&mut self, event: PersistentEvent) {
        match event {
            PersistentEvent::Persisted => self.event_counter += 1,
            PersistentEvent::Snapshoted => self.state_counter += 1,
            _ => (),
        }
    }
}

/// Handler for a persistent actor.
#[async_trait]
impl<E, A> Handler<PersistentEvent, Command<E, A>> for Persistent<E, A>
where
    E: Event,
    A: PersistentActor<E> + Debug + Clone,
{
    async fn handle<'a, 'b>(
        &'a mut self,
        msg: Command<E, A>,
        _ctx: &'b mut ActorContext<PersistentEvent>,
    ) -> Response<A>
    {
        // Match the command.
        match msg { 
            // Persist an event.
            Command::Persist(event) => {
                match self.persist(event) {
                    Ok(_) => {
                        self.apply_event(PersistentEvent::Persisted);
                        return Response::Persisted;
                    },
                    Err(e) => {
                        return Response::Error(e);
                    },
                }
            },
            // Snapshot the state.
            Command::Snapshot(actor) => {
                match self.snapshot(actor) {
                    Ok(_) => {
                        self.apply_event(PersistentEvent::Snapshoted);
                        Response::Snapshoted
                    },
                    Err(e) => {
                        Response::Error(e)
                    },
                }
            },
            // Recover the state.
            Command::Recover => {
                match self.recover() {
                    Ok(state) => {
                        Response::Recovered(state)
                    },
                    Err(e) => {
                        Response::Error(e)
                    },
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryManager;

    use actor::{ActorSystem, Actor, Handler, Message, Event, ActorContext, Error as ActorError};

    use async_trait::async_trait;

    #[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
    #[allow(dead_code)]
    struct TestEvent(usize);

    impl Event for TestEvent {}

    #[derive(Debug, Clone)]
    struct TestMessage(usize);

    impl Message for TestMessage {
        type Response = usize;
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct TestActor {
        counter: usize,
    }

    #[async_trait]
    impl PersistentActor<TestEvent> for TestActor {}

    #[async_trait]
    impl Actor<TestEvent> for TestActor {
        fn apply_event(&mut self, event: TestEvent) {
            self.counter = event.0;
        }

        async fn pre_start(&mut self, ctx: &ActorContext<TestEvent>) -> Result<(), ActorError> {
            // let persistent = Persistent::new("test", MemoryManager::default()).unwrap();
            
            //ctx.create_child("persistent", persistent).await.unwrap();
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<TestEvent, TestMessage> for TestActor {
        async fn handle(
            &mut self,
            msg: TestMessage,
            ctx: &mut ActorContext<TestEvent>,
        ) -> usize {
            self.apply_event(TestEvent(msg.0));
            ctx.emit(TestEvent(self.counter)).await.unwrap();
            self.counter
        }
    }

    #[tokio::test]
    async fn test_persistent_actor() {
        let system = ActorSystem::default();
        let actor_ref = system
            .create_actor("test_actor", TestActor::default())
            .await
            .unwrap();

        let persistent: Persistent<TestEvent, TestActor> = Persistent::new("test", MemoryManager::default()).unwrap();
        let per_ref = system.create_actor("persistent", persistent).await.unwrap();

    }
}