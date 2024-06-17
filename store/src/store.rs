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

use actor::{
    Actor, ActorContext, Error as ActorError, Event, Handler, Message, Response,
};

use async_trait::async_trait;

use memsecurity::EncryptedMem;
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use tracing::{debug, error};

use std::{fmt::Debug, marker::PhantomData};


/// Nonce size.
const NONCE_SIZE: usize = 12;

/// A trait representing a persistent actor.
#[async_trait]
pub trait PersistentActor:
    Actor + Handler<Self> + Debug + Clone + Serialize + DeserializeOwned
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
    async fn persist(
        &mut self,
        event: Self::Event,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), ActorError> {
        let store = match ctx.get_child::<Store<Self>>("store").await {
            Some(store) => store,
            None => {
                return Err(ActorError::Store(
                    "Can't get store actor".to_string(),
                ))
            }
        };
        let response = store
            .ask(StoreCommand::Persist(event.clone()))
            .await
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
    /// Void.
    ///
    /// # Errors
    ///
    /// An error if the operation failed.
    ///
    async fn snapshot(
        &self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), ActorError> {
        let store = match ctx.get_child::<Store<Self>>("store").await {
            Some(store) => store,
            None => {
                return Err(ActorError::Store(
                    "Can't get store actor".to_string(),
                ))
            }
        };
        store
            .ask(StoreCommand::Snapshot(self.clone()))
            .await
            .map_err(|e| ActorError::Store(e.to_string()))?;
        Ok(())
    }

    /// Start the child store and recover the state (if any).
    ///
    /// # Arguments
    ///
    /// - ctx: The actor context.
    /// - manager: The database manager.
    ///
    /// # Returns
    ///
    /// Void.
    ///
    /// # Errors
    ///
    /// An error if the operation failed.
    ///
    async fn start_store<C: Collection>(
        &mut self,
        ctx: &mut ActorContext<Self>,
        manager: impl DbManager<C>,
        password: Option<[u8; 32]>,
    ) -> Result<(), ActorError> {
        let store = Store::<Self>::new("store", manager, password)
            .map_err(|e| ActorError::Store(e.to_string()))?;
        let store = ctx.create_child("store", store).await?;
        let response = store.ask(StoreCommand::Recover).await?;
        if let StoreResponse::State(Some(state)) = response {
            self.update(state);
        }
        Ok(())
    }

    /// Stop the child store and snapshot the state.
    ///
    /// # Arguments
    ///
    /// - ctx: The actor context.
    ///
    /// # Returns
    ///
    /// Void.
    ///
    /// # Errors
    ///
    /// An error if the operation failed.
    ///
    async fn stop_store(
        &mut self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), ActorError> {
        if let Some(store) = ctx.get_child::<Store<Self>>("store").await {
            let _ = store.ask(StoreCommand::Snapshot(self.clone())).await?;
            store.stop().await;
            Ok(())
        } else {
            Err(ActorError::Store("Can't get store".to_string()))
        }
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
    /// Key box that encrypts contents.
    key_box: EncryptedMem,
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
    pub fn new<C>(name: &str, manager: impl DbManager<C>, password: Option<[u8; 32]>) -> Result<Self, Error>
    where
        C: Collection + 'static,
    {
        let mut key_box = EncryptedMem::new();
        if let Some(password) = password {
            key_box.encrypt(&password).
                map_err(|_| Error::Store("".to_owned()))?;
        }
        let events = manager.create_collection(&format!("{}_events", name))?;
        let states = manager.create_collection(&format!("{}_states", name))?;
        Ok(Self {
            event_counter: 0,
            events: Box::new(events),
            states: Box::new(states),
            key_box,
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
        let bytes = if let Ok(key) = self.key_box.decrypt() {
            self.encrypt(key.as_ref(), bytes.as_slice())?
        } else {
            bytes
        };
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
            let bytes = if let Ok(key) = self.key_box.decrypt() {
                self.decrypt(key.as_ref(), data.as_slice())?
            } else {
                data
            };    
            self.event_counter = key.parse().map_err(|e| {
                Error::Store(format!("Can't parse event key: {}", e))
            })?;
            let state: P = bincode::deserialize(&bytes).map_err(|e| {
                Error::Store(format!("Can't deserialize state: {}", e))
            })?;
            debug!("Recovered state: {:?}", state);
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    /// Encrypt bytes.
    /// 
    fn encrypt(&self, key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng); // 96-bits; unique per message
        let ciphertext: Vec<u8> = cipher.encrypt(&nonce, bytes.as_ref())
            .map_err(|e| Error::Store(format!("Encrypt error: {}", e)))?;

        Ok([nonce.to_vec(), ciphertext].concat())
    }

    /// Decrypt bytes 
    /// 
    fn decrypt(&self, key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce: [u8; 12] = ciphertext[..NONCE_SIZE].try_into().map_err(|e| {
            Error::Store(format!("Nonce error: {}", e))
        })?;
        let nonce = Nonce::from_slice(&nonce);
        let ciphertext = &ciphertext[NONCE_SIZE..];
        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|e| Error::Store(format!("Decrypt error: {}", e)))?;
        Ok(plaintext)
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
                }
                Err(e) => StoreResponse::Error(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::memory::MemoryManager;
    use actor::{ActorRef, ActorSystem};

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
            let db =
                Store::<Self>::new("store", MemoryManager::default(), None).unwrap();
            let store = ctx.create_child("store", db).await.unwrap();
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
            let store: ActorRef<Store<Self>> =
                ctx.get_child("store").await.unwrap();
            let response = store
                .ask(StoreCommand::Snapshot(self.clone()))
                .await
                .unwrap();
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
                }
                TestMessage::Recover => {
                    let store: ActorRef<Store<Self>> =
                        ctx.get_child("store").await.unwrap();
                    let response =
                        store.ask(StoreCommand::Recover).await.unwrap();
                    if let StoreResponse::State(Some(state)) = response {
                        self.update(state.clone());
                        TestResponse::Value(state.value)
                    } else {
                        TestResponse::None
                    }
                }
                TestMessage::Snapshot => {
                    let store: ActorRef<Store<Self>> =
                        ctx.get_child("store").await.unwrap();
                    store
                        .ask(StoreCommand::Snapshot(self.clone()))
                        .await
                        .unwrap();
                    TestResponse::None
                }
                TestMessage::GetValue => {
                    println!("Getting value: {}", self.value);
                    TestResponse::Value(self.value)
                }
            }
        }

        async fn handle_event(
            &mut self,
            event: TestEvent,
            ctx: &mut ActorContext<TestActor>,
        ) -> () {
            self.persist(event, ctx).await.unwrap();
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

        let db =
            Store::<TestActor>::new("store", MemoryManager::default(), None).unwrap();
        let store = system.create_root_actor("store", db).await.unwrap();

        let actor = TestActor { value: 0 };
        store
            .tell(StoreCommand::Persist(TestEvent(10)))
            .await
            .unwrap();

        store
            .tell(StoreCommand::Snapshot(actor.clone()))
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_encrypt_decrypt() {
        let key = [0u8; 32];
        let store = Store::<TestActor>::new("store", MemoryManager::default(), Some(key)).unwrap();
        let data = b"Hello, world!";
        let encrypted = store.encrypt(&key, data).unwrap();
        let decrypted = store.decrypt(&key, &encrypted).unwrap();
        assert_eq!(data, decrypted.as_slice());
    }
}
