// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Store module.
//!
//! This module contains the store implementation.
//!
//! The `Store` actor is an actor that offers the ability to persist actors from events that modify
//! their state (applying the event sourcing pattern). It also allows you to store snapshots
//! of an actor. The `PersistentActor` trait is an extension of the `Actor` trait that must be
//! implemented by actors who need to persist.
//!

use crate::{
    database::{Collection, DbManager},
    error::Error,
};

use actor::{Actor, ActorRef, ActorContext, Event, Handler, Message, Response, Error as ActorError};
use async_trait::async_trait;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use tracing::{debug, error};

use std::{fmt::Debug, marker::PhantomData};

/// A trait representing a persistent actor.
#[async_trait]
pub trait PersistentActor:
    Actor + Debug + Clone + Serialize + DeserializeOwned
{
    
    /// Apply an event to the actor state
    ///
    /// # Arguments
    /// 
    /// - event: The event to apply.
    /// 
    fn apply(&mut self, event: Self::Event);

    /// Recover the state.
    /// 
    /// # Arguments
    /// 
    /// - state: The recovered state.
    /// 
    fn update(&mut self, state: Self) {
        *self = state;
    }

    /// Persist an event.
    ///
    /// # Arguments
    ///
    /// - event: The event to persist.
    /// - store: The store actor.
    ///
    /// # Returns
    ///
    /// The result of the operation.
    ///
    /// # Errors
    /// 
    /// An error if the operation failed.
    /// 
    async fn persist(&mut self, event: Self::Event, store: &ActorRef<Store<Self>>) -> Result<(), ActorError> {
        let response = store.ask(StoreCommand::Persist(event.clone())).await
            .map_err(|e| ActorError::Store(e.to_string()))?;
        if let StoreResponse::Persisted = response {
            self.apply(event);
            Ok(())
        } else {
            Err(ActorError::Store("Can't persist event".to_string()))
        }
    }

    /// Snapshot the state.
    /// 
    /// # Arguments
    /// 
    /// - store: The store actor.
    /// 
    /// # Returns
    /// 
    /// The result of the operation.
    /// 
    /// # Errors
    /// 
    /// An error if the operation failed.
    /// 
    async fn snapshot(&self, store: &ActorRef<Store<Self>>) -> Result<(), ActorError> {
        store.ask(StoreCommand::Snapshot(self.clone())).await
            .map_err(|e| ActorError::Store(e.to_string()))?;
        Ok(())
    }

}

/// Store actor.
pub struct Store<P>
where
    P: PersistentActor,
{
    /// The event counter index.
    event_counter: usize,
    /// The events collection.
    events: Box<dyn Collection>,
    /// The states collection.
    states: Box<dyn Collection>,
    /// The phantom actor.
    _phantom_actor: PhantomData<P>,
}

impl<P: PersistentActor> Store<P> {
    /// Creates a new store actor.
    ///
    /// # Arguments
    ///
    /// - name: The name of the actor.
    /// - manager: The database manager.
    ///
    /// # Returns
    ///
    /// The persistent actor.
    ///
    /// # Errors
    ///
    /// An error if it fails to create the collections.
    ///
    pub fn new<C>(name: &str, manager: impl DbManager<C>) -> Result<Self, Error>
    where
        C: Collection + 'static,
    {
        let events = manager.create_collection(&format!("{}_events", name))?;
        let states = manager.create_collection(&format!("{}_states", name))?;
        Ok(Self {
            event_counter: 0,
            events: Box::new(events),
            states: Box::new(states),
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
    fn persist(&mut self, event: P::Event) -> Result<(), Error> {
        debug!("Persisting event: {:?}", event);
        let bytes = bincode::serialize(&event).map_err(|e| {
            error!("Can't serialize event: {}", e);
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
    fn snapshot(&mut self, actor: &P) -> Result<(), Error> {
        let bytes = bincode::serialize(actor).map_err(|e| {
            Error::Store(format!("Can't serialize state: {}", e))
        })?;
        self.states.put(&self.event_counter.to_string(), &bytes)
    }

    /// Recover the state.
    ///
    /// # Returns
    ///
    /// The recovered state.
    ///
    /// An error if the operation failed.
    ///
    fn recover(&mut self) -> Result<Option<P>, Error> {
        debug!("Recovering state");
        if let Some((key, data)) = self.states.last() {
            self.event_counter = key.parse().map_err(|e| {
                Error::Store(format!("Can't parse event key: {}", e))
            })?;
            let state: P = bincode::deserialize(&data).map_err(|e| {
                Error::Store(format!("Can't deserialize state: {}", e))
            })?;
            debug!("Recovered state: {:?}", state);
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }
}

/// Store command.
#[derive(Debug, Clone)]
pub enum StoreCommand<P>
where
    P: PersistentActor,
{
    Persist(P::Event),
    Snapshot(P),
    Recover,
}

/// Implements `Message` for store command.
impl<P: PersistentActor> Message for StoreCommand<P> {}

/// Store response.
#[derive(Debug, Clone)]
pub enum StoreResponse<P>
where
    P: PersistentActor,
{
    None,
    Persisted,
    Snapshotted,
    State(Option<P>),
    Error(Error),
}

/// Implements `Response` for store response.
impl<P: PersistentActor> Response for StoreResponse<P> {}

/// Store event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoreEvent {
    Persisted,
    Snapshotted,
}

/// Implements `Event` for store event.
impl Event for StoreEvent {}

#[async_trait]
impl<P> Actor for Store<P>
where
    P: PersistentActor,
{
    type Message = StoreCommand<P>;
    type Response = StoreResponse<P>;
    type Event = StoreEvent;


}

#[async_trait]
impl<P> Handler<Store<P>> for Store<P>
where
    P: PersistentActor,
{
    async fn handle_message(
        &mut self,
        msg: StoreCommand<P>,
        _ctx: &mut ActorContext<Store<P>>,
    ) -> StoreResponse<P> {
        // Match the command.
        match msg {
            // Persist an event.
            StoreCommand::Persist(event) => match self.persist(event.clone()) {
                Ok(_) => {
                    debug!("Persisted event: {:?}", event);
                    self.event_counter += 1;
                    StoreResponse::Persisted
                }
                Err(e) => StoreResponse::Error(e),
            },
            // Snapshot the state.
            StoreCommand::Snapshot(actor) => match self.snapshot(&actor) {
                Ok(_) => {
                    debug!("Snapshotted state: {:?}", actor);
                    StoreResponse::Snapshotted
                }
                Err(e) => StoreResponse::Error(e),
            },
            // Recover the state.
            StoreCommand::Recover => match self.recover() {
                Ok(state) => {
                    debug!("Recovered state: {:?}", state);
                    StoreResponse::State(state)
                },
                Err(e) => StoreResponse::Error(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::memory::MemoryManager;
    use actor::{ActorSystem, ActorRef};

    use async_trait::async_trait;

    use tracing_test::traced_test;
    
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestActor {
        pub value: i32,
    }

    #[derive(Debug, Clone)]
    enum TestMessage {
        Increment(i32),
        Recover,
        Snapshot,
        GetValue,
    }

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestEvent(i32);

    impl Event for TestEvent {}

    #[derive(Debug, Clone, PartialEq)]
    enum TestResponse {
        Value(i32),
        None,
    }

    #[async_trait]
    impl Actor for TestActor {

        type Message = TestMessage;
        type Event = TestEvent;
        type Response = TestResponse;


        async fn pre_start(
            &mut self,
            ctx: &mut ActorContext<Self>,
        ) -> Result<(), ActorError> {
            let db = Store::<Self>::new("store", MemoryManager::default()).unwrap();
            let store = ctx.create_child("store",  db).await.unwrap();
            let response = store.ask(StoreCommand::Recover).await.unwrap();
            debug!("Recover response: {:?}", response); 
            if let StoreResponse::State(Some(state)) = response {
                debug!("Recovering state: {:?}", state);
                self.update(state);
            }
            Ok(())
        }

        async fn post_stop(
            &mut self,
            ctx: &mut ActorContext<Self>,
        ) -> Result<(), ActorError> {
            let store: ActorRef<Store<Self>> = ctx.get_child("store").await.unwrap();
            let response = store.ask(StoreCommand::Snapshot(self.clone())).await.unwrap();
            if let StoreResponse::Snapshotted = response {
                store.stop().await;
                Ok(())
            } else {
                Err(ActorError::Store("Can't snapshot state".to_string()))
            }
        }

    }

    #[async_trait]
    impl PersistentActor for TestActor {
        fn apply(&mut self, event: Self::Event) {
            println!("Applying event: {:?}, value {}", event, self.value);
            self.value += event.0;
            println!("Applied event: {:?}, value {}", event, self.value);
        }

    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {

        async fn handle_message(
            &mut self,
            msg: TestMessage,
            ctx: &mut ActorContext<TestActor>,
        ) -> TestResponse {
            match msg {
                TestMessage::Increment(value) => {
                    let event = TestEvent(value);
                    ctx.event(event).await.unwrap();
                    TestResponse::None
                },
                TestMessage::Recover => {
                    let store: ActorRef<Store<Self>> = ctx.get_child("store").await.unwrap();
                    let response = store.ask(StoreCommand::Recover).await.unwrap();
                    if let StoreResponse::State(Some(state)) = response {
                        self.update(state.clone());
                        TestResponse::Value(state.value)
                    } else {
                        TestResponse::None
                    }
                }
                TestMessage::Snapshot => {
                    let store: ActorRef<Store<Self>> = ctx.get_child("store").await.unwrap();
                    store.ask(StoreCommand::Snapshot(self.clone())).await.unwrap();
                    TestResponse::None
                },
                TestMessage::GetValue => {
                    println!("Getting value: {}", self.value);
                    TestResponse::Value(self.value)
                },
            }
        }

        async fn handle_event(
            &mut self,
            event: TestEvent,
            ctx: &mut ActorContext<TestActor>,
        ) -> () {            
            let store: ActorRef<Store<Self>> = ctx.get_child("store").await.unwrap();  
            self.persist(event, &store).await.unwrap();
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_store_actor() {
        let (system, mut runner) = ActorSystem::create();
        // Init runner.
        tokio::spawn(async move {
            runner.run().await;
        });

        let db = Store::<TestActor>::new("store", MemoryManager::default()).unwrap();
        let store = system.create_root_actor("store", db).await.unwrap();

        let actor = TestActor { value: 0 };
        store.tell(StoreCommand::Persist(TestEvent(10))).await.unwrap();

        store.tell(StoreCommand::Snapshot(actor.clone())).await.unwrap();

        let response = store.ask(StoreCommand::Recover).await.unwrap();
        println!("Recover response: {:?}", response);

    } 

    #[tokio::test]
    //#[traced_test]
    async fn test_persistent_actor() {
        let (system, mut runner) = ActorSystem::create();
        // Init runner.
        tokio::spawn(async move {
            runner.run().await;
        });
 
        let actor = TestActor { value: 0 };

        let actor_ref = system.create_root_actor("test", actor).await.unwrap();

        let result = actor_ref.ask(TestMessage::Increment(10)).await.unwrap();

        assert_eq!(result, TestResponse::None);

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        actor_ref.tell(TestMessage::Snapshot).await.unwrap();

        let result = actor_ref.ask(TestMessage::GetValue).await.unwrap();   

        assert_eq!(result, TestResponse::Value(10));
        actor_ref.tell(TestMessage::Increment(10)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let value = actor_ref.ask(TestMessage::GetValue).await.unwrap();

        assert_eq!(value, TestResponse::Value(20));


        actor_ref.ask(TestMessage::Recover).await.unwrap();

        let value = actor_ref.ask(TestMessage::GetValue).await.unwrap();

        assert_eq!(value, TestResponse::Value(10));

        actor_ref.stop().await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

} 