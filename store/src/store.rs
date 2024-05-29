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
    error::Error,
    database::{Collection, DbManager},
};

use actor::{Actor, ActorContext, Event, Message, Response, Handler};
use async_trait::async_trait;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use std::{
    fmt::Debug,
    marker::PhantomData,
};

/// A trait representing a persistent actor.
#[async_trait]
pub trait PersistentActor: Actor + Debug + Clone + Serialize + DeserializeOwned {

    /// Apply an event to the actor state.
    fn apply(&mut self, event: Self::Event);

    /// Persist an event.
    /// 
    /// # Arguments
    /// 
    /// - event: The event to persist.
    /// 
    /// # Returns
    /// 
    /// The result of the operation.
    /// 
    async fn persist(&mut self, event: Self::Event) -> Result<(), Error> {
        self.apply(event);
        Ok(())
    }

}

/// Store actor.
pub struct Store<P>
where
    P: PersistentActor,
{
    event_counter: usize,
    state_counter: usize,
    events: Box<dyn Collection>,
    states: Box<dyn Collection>,
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
    pub fn new<C>(
        name: &str,
        manager: impl DbManager<C>,
    ) -> Result<Self, Error>
    where
        C: Collection + 'static,
    {
        let events = manager.create_collection(&format!("{}_events", name))?;
        let states = manager.create_collection(&format!("{}_states", name))?;
        Ok(Self {
            event_counter: 0,
            state_counter: 0,
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
    fn snapshot(&mut self, actor: &P) -> Result<(), Error> {
        let bytes = bincode::serialize(actor).map_err(|e| {
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
    fn recover(&mut self) -> Result<P, Error> {
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
        let state: P = bincode::deserialize(&data).map_err(|e| {
            Error::Store(format!("Can't deserialize state: {}", e))
        })?;
        Ok(state)
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
    State(P),
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
    async fn handle(
        &mut self,
        msg: StoreCommand<P>,
        _ctx: &mut ActorContext<Store<P>>,
    ) -> StoreResponse<P>
    {
        // Match the command.
        match msg {
            // Persist an event.
            StoreCommand::Persist(event) => {
                match self.persist(event) {
                    Ok(_) => {
                        self.event_counter += 1;
                        StoreResponse::None
                    },
                    Err(e) => {
                        StoreResponse::Error(e)
                    },
                }
            },
            // Snapshot the state.
            StoreCommand::Snapshot(actor) => {
                match self.snapshot(&actor) {
                    Ok(_) => {
                        self.state_counter += 1;
                        StoreResponse::None
                    },
                    Err(e) => {
                        StoreResponse::Error(e)
                    },
                }
            },
            // Recover the state.
            StoreCommand::Recover => {
                match self.recover() {
                    Ok(state) => {
                        StoreResponse::State(state)
                    },
                    Err(e) => {
                        StoreResponse::Error(e)
                    },
                }
            },
        }
    }
}