

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
    database::{Collection, DbManager, State},
    error::Error,
};

use actor::{
    Actor, ActorContext, ActorPath, Error as ActorError, Event, Handler,
    Message, Response,
};

use async_trait::async_trait;

use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use memsecurity::EncryptedMem;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use tracing::{debug, error};

use std::{fmt::Debug, marker::PhantomData};

/// Nonce size for ChaCha20-Poly1305 encryption.
const NONCE_SIZE: usize = 12;

/// Defines the persistence strategy for an actor.
/// This determines how events and state are stored.
#[derive(Debug, Clone)]
pub enum PersistenceType {
    /// Light persistence: Stores each event along with the current state snapshot.
    /// Faster recovery but uses more storage space.
    Light,
    /// Full persistence: Stores only events, state is reconstructed by replaying them.
    /// Uses less storage but slower recovery due to event replay.
    Full,
}

/// Marker type for light persistence strategy.
/// Use this when you want fast recovery and don't mind larger storage footprint.
pub struct LightPersistence;

/// Marker type for full event sourcing strategy.
/// Use this for complete audit trail and smaller storage footprint.
pub struct FullPersistence;

/// Trait for defining persistence strategy at the type level.
/// Implement this on your persistence marker types.
pub trait Persistence {
    /// Returns the persistence type for this strategy.
    fn get_persistence() -> PersistenceType;
}

impl Persistence for LightPersistence {
    fn get_persistence() -> PersistenceType {
        PersistenceType::Light
    }
}

impl Persistence for FullPersistence {
    fn get_persistence() -> PersistenceType {
        PersistenceType::Full
    }
}

/// Trait for actors that persist their state using event sourcing.
/// PersistentActor extends the Actor trait with methods for persisting events,
/// snapshotting state, and recovering from storage.
///
/// # Event Sourcing Pattern
///
/// This trait implements the event sourcing pattern where:
/// 1. Events represent state changes and are persisted immutably.
/// 2. Actor state is derived by applying events in sequence.
/// 3. Snapshots can be taken to speed up recovery.
///
/// # Type Requirements
///
/// Implementing types must be:
/// - Cloneable (for state snapshots)
/// - Serializable (for persistence)
/// - Debuggable (for logging)
///
/// # Example
///
/// ```
/// #[derive(Clone, Debug, Serialize, Deserialize)]
/// struct BankAccount {
///     balance: i64,
/// }
///
/// #[async_trait]
/// impl PersistentActor for BankAccount {
///     type Persistence = FullPersistence;
///
///     fn apply(&mut self, event: &Self::Event) -> Result<(), ActorError> {
///         match event {
///             AccountEvent::Deposited(amount) => {
///                 self.balance += amount;
///                 Ok(())
///             }
///             // ... other events
///         }
///     }
/// }
/// ```
///
#[async_trait]
pub trait PersistentActor:
    Actor + Handler<Self> + Debug + Clone + Serialize + DeserializeOwned
{
    /// The persistence strategy type (Light or Full).
    type Persistence: Persistence;

    /// Applies an event to the actor's state.
    /// This method should be deterministic - applying the same event
    /// to the same state should always produce the same result.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to apply to the actor's state.
    ///
    /// # Returns
    ///
    /// Ok(()) if the event was applied successfully.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be applied (e.g., invalid state transition).
    ///
    /// # Important
    ///
    /// This method should NOT persist the event - that's handled by the `persist()` method.
    /// This only updates the in-memory state.
    ///
    fn apply(&mut self, event: &Self::Event) -> Result<(), ActorError>;

    /// Updates the actor's state by replacing it with a recovered state.
    /// This is called during recovery to restore the actor from a snapshot.
    ///
    /// # Arguments
    ///
    /// * `state` - The recovered state to restore.
    ///
    /// # Default Behavior
    ///
    /// The default implementation simply replaces the current state.
    /// Override this if you need custom recovery logic.
    ///
    fn update(&mut self, state: Self) {
        *self = state;
    }

    /// Persists an event by applying it to state and storing it in the database.
    /// This is the main method for persisting state changes in event sourcing.
    ///
    /// # Process
    ///
    /// 1. Retrieves the Store child actor
    /// 2. Creates a backup of the current state
    /// 3. Applies the event to the state
    /// 4. Persists the event (and possibly state) based on persistence type
    /// 5. Rolls back state if persistence fails
    ///
    /// # Arguments
    ///
    /// * `event` - The event to persist.
    /// * `ctx` - The actor context (used to access the store child actor).
    ///
    /// # Returns
    ///
    /// Ok(()) if the event was applied and persisted successfully.
    ///
    /// # Errors
    ///
    /// Returns ActorError::Store if:
    /// - The store actor cannot be accessed
    /// - The event application fails
    /// - The persistence operation fails
    ///
    /// # Persistence Behavior
    ///
    /// - **Light**: Persists both the event and current state snapshot
    /// - **Full**: Persists only the event
    ///
    async fn persist(
        &mut self,
        event: &Self::Event,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), ActorError> {
        let store = match ctx.get_child::<Store<Self>>("store").await {
            Some(store) => store,
            None => {
                return Err(ActorError::Store(
                    "Can't get store actor".to_string(),
                ));
            }
        };

        let prev_state = self.clone();

        if let Err(e) = self.apply(event) {
            error!("");
            self.update(prev_state);
            return Err(e);
        }

        let response = match Self::Persistence::get_persistence() {
            PersistenceType::Light => store
                .ask(StoreCommand::PersistLight(event.clone(), self.clone()))
                .await
                .map_err(|e| ActorError::Store(e.to_string()))?,
            PersistenceType::Full => store
                .ask(StoreCommand::Persist(event.clone()))
                .await
                .map_err(|e| ActorError::Store(e.to_string()))?,
        };

        match response {
            StoreResponse::Persisted => Ok(()),
            StoreResponse::Error(error) => {
                error!("");
                Err(ActorError::Store(error.to_string()))
            }
            _ => Err(ActorError::UnexpectedResponse(
                ActorPath::from(format!("{}/store", ctx.path().clone())),
                "StoreResponse::Persisted".to_owned(),
            )),
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
                ));
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
    /// - name: Actor type.
    /// - manager: The database manager.
    /// - password: Optional password.
    ///
    /// # Returns
    ///
    /// Void.
    ///
    /// # Errors
    ///
    /// An error if the operation failed.
    ///
    async fn start_store<C: Collection, S: State>(
        &mut self,
        name: &str,
        prefix: Option<String>,
        ctx: &mut ActorContext<Self>,
        manager: impl DbManager<C, S>,
        password: Option<[u8; 32]>,
    ) -> Result<(), ActorError> {
        let prefix = match prefix {
            Some(prefix) => prefix,
            None => ctx.path().key(),
        };

        let store = Store::<Self>::new(name, &prefix, manager, password)
            .map_err(|e| ActorError::Store(e.to_string()))?;
        let store = ctx.create_child("store", store).await?;
        let response = store
            .ask(StoreCommand::Recover)
            .await?;

        if let StoreResponse::State(Some(state)) = response {
            self.update(state);
        } else {
            debug!("Create first snapshot");
            store.tell(StoreCommand::Snapshot(self.clone())).await?;
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
            if let PersistenceType::Full = Self::Persistence::get_persistence() {
                let _ = store.ask(StoreCommand::Snapshot(self.clone())).await?;
            }

            store.ask_stop().await
        } else {
            Err(ActorError::Store("Can't get store".to_string()))
        }
    }
}

/// Store actor that manages persistent storage for a PersistentActor.
/// The Store handles event persistence, state snapshots, and recovery.
/// It operates as a child actor of the persistent actor it serves.
///
/// # Storage Model
///
/// - **Events**: Stored in a Collection with sequence numbers as keys
/// - **Snapshots**: Stored in State storage (single value, updated on each snapshot)
/// - **Encryption**: Optional ChaCha20-Poly1305 encryption for at-rest data
///
/// # Type Parameters
///
/// * `P` - The PersistentActor type this store manages
///
pub struct Store<P>
where
    P: PersistentActor,
{
    /// Current event sequence number (auto-incrementing).
    event_counter: u64,
    /// Current state version number (for snapshots).
    state_counter: u64,
    /// Collection for storing events with sequence numbers as keys.
    events: Box<dyn Collection>,
    /// Storage for the latest state snapshot.
    states: Box<dyn State>,
    /// Encrypted password for data encryption (ChaCha20-Poly1305).
    /// If None, data is stored unencrypted.
    key_box: Option<EncryptedMem>,
    /// Phantom data to associate with the PersistentActor type.
    _phantom: PhantomData<P>,
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
    pub fn new<C, S>(
        name: &str,
        prefix: &str,
        manager: impl DbManager<C, S>,
        password: Option<[u8; 32]>,
    ) -> Result<Self, Error>
    where
        C: Collection + 'static,
        S: State + 'static,
    {
        let key_box = match password {
            Some(key) => {
                let mut key_box = EncryptedMem::new();
                key_box.encrypt(&key).map_err(|e| {
                    Error::Store(format!("Can't encrypt password: {:?}", e))
                })?;
                Some(key_box)
            }
            None => None,
        };
        let events =
            manager.create_collection(&format!("{}_events", name), prefix)?;
        let states =
            manager.create_state(&format!("{}_states", name), prefix)?;

        // Initialize event_counter from the last event in the database
        let initial_event_counter = if let Some((key, _)) = events.last() {
            key.parse().unwrap_or(0)
        } else {
            0
        };

        debug!("Initializing Store with event_counter: {}", initial_event_counter);

        Ok(Self {
            event_counter: initial_event_counter,
            state_counter: 0,
            events: Box::new(events),
            states: Box::new(states),
            key_box,
            _phantom: PhantomData,
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
    fn persist<E>(&mut self, event: &E) -> Result<(), Error>
    where
        E: Event + Serialize + DeserializeOwned,
    {
        debug!("Persisting event: {:?}", event);
        let bin_config = bincode::config::standard();

        let bytes = if let Some(key_box) = &self.key_box {
            if let Ok(key) = key_box.decrypt() {
                let bytes = bincode::serde::encode_to_vec(event, bin_config)
                    .map_err(|e| {
                        error!("Can't encode event: {}", e);
                        Error::Store(format!("Can't encode event: {}", e))
                    })?;
                self.encrypt(key.as_ref(), &bytes)?
            } else {
                return Err(Error::Store("Can't decrypt key".to_owned()));
            }
        } else {
            bincode::serde::encode_to_vec(event, bin_config).map_err(|e| {
                error!("Can't encode event: {}", e);
                Error::Store(format!("Can't encode event: {}", e))
            })?
        };

        // Calculate next event number but don't increment yet
        let next_event_number = if self.event_counter != 0 {
            self.event_counter + 1
        } else if let Ok(last) = self.last_event() && last.is_some(){
            self.event_counter + 1
        } else {
            0
        };

        debug!("Persisting event {} at index {}", std::any::type_name::<E>(), next_event_number);

        // First persist the event, then increment counter (atomic operation)
        let result = self.events
            .put(&format!("{:020}", next_event_number), &bytes);

        // Only increment counter if persist was successful
        if result.is_ok() {
            self.event_counter = next_event_number;
            debug!("Successfully persisted event, event_counter now: {}", self.event_counter);
        }

        result
    }

    /// Persist an event and the state.
    /// This method is used to persist an event and the state of the actor in a single operation.
    /// This applies in scenarios where we want to keep only the last event and state.
    ///
    /// # Arguments
    ///
    /// - event: The event to persist.
    /// - state: The state of the actor (without applying the event).
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    fn persist_state<E>(&mut self, event: &E, state: &P) -> Result<(), Error>
    where
        E: Event + Serialize + DeserializeOwned,
    {
        debug!("Persisting event: {:?}", event);
        let bin_config = bincode::config::standard();

        let bytes = if let Some(key_box) = &self.key_box {
            if let Ok(key) = key_box.decrypt() {
                let bytes = bincode::serde::encode_to_vec(event, bin_config)
                    .map_err(|e| {
                        error!("Can't encode event: {}", e);
                        Error::Store(format!("Can't encode event: {}", e))
                    })?;
                self.encrypt(key.as_ref(), &bytes)?
            } else {
                return Err(Error::Store("Can't decrypt key".to_owned()));
            }
        } else {
            bincode::serde::encode_to_vec(event, bin_config).map_err(|e| {
                error!("Can't encode event: {}", e);
                Error::Store(format!("Can't encode event: {}", e))
            })?
        };

        self.snapshot(state)?;
        self.events
            .put(&format!("{:020}", self.event_counter), &bytes)
    }

    /// Returns the last event.
    ///
    /// # Returns
    ///
    /// The last event.
    ///
    /// An error if the operation failed.
    ///
    fn last_event(&self) -> Result<Option<P::Event>, Error> {
        if let Some((_, data)) = self.events.last() {
            let bin_config = bincode::config::standard();

            let event: P::Event = if let Some(key_box) = &self.key_box {
                if let Ok(key) = key_box.decrypt() {
                    let data = self.decrypt(key.as_ref(), data.as_slice())?;
                    bincode::serde::decode_from_slice(&data, bin_config)
                        .map_err(|e| {
                            error!("Can't decode event: {}", e);
                            Error::Store(format!("Can't decode event: {}", e))
                        })?
                        .0
                } else {
                    return Err(Error::Store("Can't decrypt key".to_owned()));
                }
            } else {
                bincode::serde::decode_from_slice(&data, bin_config)
                    .map_err(|e| {
                        error!("Can't decode event: {}", e);
                        Error::Store(format!("Can't decode event: {}", e))
                    })?
                    .0
            };
            Ok(Some(event))
        } else {
            Ok(None)
        }
    }

    fn get_state(&self) -> Result<Option<(P, u64)>, Error> {
        let data = match self.states.get() {
            Ok(data) => data,
            Err(e) => {
                if let Error::EntryNotFound(_) = e {
                    return Ok(None);
                } else {
                    return Err(e);
                }
            }
        };

        let bytes = if let Some(key_box) = &self.key_box {
            if let Ok(key) = key_box.decrypt() {
                self.decrypt(key.as_ref(), data.as_slice())?
            } else {
                return Err(Error::Store("Can't decrypt key".to_owned()));
            }
        } else {
            data
        };

        let bin_config = bincode::config::standard();

        let state: (P, u64) =
            bincode::serde::decode_from_slice(&bytes, bin_config)
                .map_err(|e| {
                    error!("Can't decode state: {}", e);
                    Error::Store(format!("Can't decode state: {}", e))
                })?
                .0;
        Ok(Some(state))
    }

    /// Retrieve events.
    fn events(&mut self, from: u64, to: u64) -> Result<Vec<P::Event>, Error> {
        let mut events = Vec::new();
        let bin_config = bincode::config::standard();

        for i in from..=to {
            if let Ok(data) = self.events.get(&format!("{:020}", i)) {
                let event: P::Event = if let Some(key_box) = &self.key_box {
                    if let Ok(key) = key_box.decrypt() {
                        let data =
                            self.decrypt(key.as_ref(), data.as_slice())?;
                        bincode::serde::decode_from_slice(&data, bin_config)
                            .map_err(|e| {
                                error!("Can't decode event: {}", e);
                                Error::Store(format!(
                                    "Can't decode event: {}",
                                    e
                                ))
                            })?
                            .0
                    } else {
                        return Err(Error::Store(
                            "Can't decrypt key".to_owned(),
                        ));
                    }
                } else {
                    bincode::serde::decode_from_slice(&data, bin_config)
                        .map_err(|e| {
                            error!("Can't decode event: {}", e);
                            Error::Store(format!("Can't decode event: {}", e))
                        })?
                        .0
                };
                events.push(event);
            } else {
                break;
            }
        }
        Ok(events)
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
            debug!("Snapshotting state: {:?}", actor);
            let bin_config = bincode::config::standard();

            self.state_counter = self.event_counter;

            let data = bincode::serde::encode_to_vec(
                (actor, self.state_counter),
                bin_config,
            )
            .map_err(|e| {
                error!("Can't encode actor: {}", e);
                Error::Store(format!("Can't encode actor: {}", e))
            })?;
            let bytes = if let Some(key_box) = &self.key_box {
                if let Ok(key) = key_box.decrypt() {
                    self.encrypt(key.as_ref(), data.as_slice())?
                } else {
                    data
                }
            } else {
                data
            };

            self.states.put(&bytes)

    }

    /// Recover the state.
    ///
    /// # Returns
    ///
    /// The recovered state.
    ///
    /// An error if the operation failed.
    ///
    fn recover(
        &mut self,
    ) -> Result<Option<P>, Error> {
        debug!("Starting recovery process");

        if let Some((mut state, counter)) = self.get_state()? {
            self.state_counter = counter;
            debug!("Recovered state with counter: {}", counter);

            if let Some((key, ..)) = self.events.last() {
                self.event_counter = key.parse().map_err(|e| {
                    Error::Store(format!("Can't parse event key: {}", e))
                })?;

                debug!("Recovery state: event_counter={}, state_counter={}",
                       self.event_counter, self.state_counter);

                if self.event_counter != self.state_counter {
                    debug!("Applying events from {} to {}", self.state_counter + 1, self.event_counter);
                    let events =
                        self.events(self.state_counter + 1, self.event_counter)?;
                    debug!("Found {} events to replay", events.len());

                    for (i, event) in events.iter().enumerate() {
                        debug!("Applying event {} of {}", i + 1, events.len());
                        state
                            .apply(event)
                            .map_err(|e| Error::Store(e.to_string()))?;
                    }

                    debug!("Updating snapshot after applying {} events", events.len());
                    self.snapshot(&state)?;
                    debug!("Recovery completed. Final event_counter: {}", self.event_counter);
                    // Note: We don't increment event_counter here as it already has the correct value
                    // from the last persisted event key
                } else {
                    debug!("State is up to date, no events to apply");
                }

                Ok(Some(state))
            } else {
                debug!("No events found in database, using recovered state as-is");
                Ok(Some(state))
            }
        } else {
            debug!("No previous state found, starting fresh");
            Ok(None)
        }
    }

    /// Purge the store.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    pub fn purge(&mut self) -> Result<(), Error> {
        self.events.purge()?;
        self.states.purge()?;
        Ok(())
    }

    /// Encrypt bytes.
    ///
    fn encrypt(&self, key: &[u8], bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng); // 96-bits; unique per message
        let ciphertext: Vec<u8> = cipher
            .encrypt(&nonce, bytes.as_ref())
            .map_err(|e| Error::Store(format!("Encrypt error: {}", e)))?;

        Ok([nonce.to_vec(), ciphertext].concat())
    }

    /// Decrypt bytes
    ///
    fn decrypt(&self, key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce = ciphertext[..NONCE_SIZE].to_vec();
        let nonce = Nonce::from_slice(&nonce);
        let ciphertext = &ciphertext[NONCE_SIZE..];

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::Store(format!("Decrypt error: {}", e)))?;
        Ok(plaintext)
    }
}

/// Store command.
#[derive(Debug, Clone)]
pub enum StoreCommand<P, E> {
    Persist(E),
    PersistLight(E, P),
    Snapshot(P),
    LastEvent,
    LastEventNumber,
    LastEventsFrom(u64),
    GetEvents { from: u64, to: u64 },
    Recover,
    Purge,
}

/// Implements `Message` for store command.
impl<P, E> Message for StoreCommand<P, E>
where
    P: PersistentActor,
    E: Event + Serialize + DeserializeOwned,
{
}

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
    LastEvent(Option<P::Event>),
    LastEventNumber(u64),
    Events(Vec<P::Event>),
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
    type Message = StoreCommand<P, P::Event>;
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
        _sender: ActorPath,
        msg: StoreCommand<P, P::Event>,
        _ctx: &mut ActorContext<Store<P>>,
    ) -> Result<StoreResponse<P>, ActorError> {
        // Match the command.
        match msg {
            // Persist an event.
            StoreCommand::Persist(event) => match self.persist(&event) {
                Ok(_) => {
                    debug!("Persisted event: {:?}", event);
                    Ok(StoreResponse::Persisted)
                }
                Err(e) => Ok(StoreResponse::Error(e)),
            },
            // Light persistence of an event.
            StoreCommand::PersistLight(event, actor) => {
                match self.persist_state(&event, &actor) {
                    Ok(_) => {
                        debug!("Light persistence of event: {:?}", event);
                        Ok(StoreResponse::Persisted)
                    }
                    Err(e) => Ok(StoreResponse::Error(e)),
                }
            }
            // Snapshot the state.
            StoreCommand::Snapshot(actor) => match self.snapshot(&actor) {
                Ok(_) => {
                    debug!("Snapshotted state: {:?}", actor);
                    Ok(StoreResponse::Snapshotted)
                }
                Err(e) => Ok(StoreResponse::Error(e)),
            },
            // Recover the state.
            StoreCommand::Recover => {
                match self.recover() {
                    Ok(state) => {
                        debug!("Recovered state: {:?}", state);
                        Ok(StoreResponse::State(state))
                    }
                    Err(e) => Ok(StoreResponse::Error(e)),
                }
            }
            StoreCommand::GetEvents { from, to } => {
                let events = self.events(from, to).map_err(|e| {
                    ActorError::Store(format!(
                        "Unable to get events range: {}",
                        e
                    ))
                })?;
                Ok(StoreResponse::Events(events))
            }
            // Get the last event.
            StoreCommand::LastEvent => match self.last_event() {
                Ok(event) => {
                    debug!("Last event: {:?}", event);
                    Ok(StoreResponse::LastEvent(event))
                }
                Err(e) => Ok(StoreResponse::Error(e)),
            },
            // Purge the store.
            StoreCommand::Purge => match self.purge() {
                Ok(_) => {
                    debug!("Purged store");
                    Ok(StoreResponse::None)
                }
                Err(e) => Ok(StoreResponse::Error(e)),
            },
            // Get the last event number.
            StoreCommand::LastEventNumber => {
                Ok(StoreResponse::LastEventNumber(self.event_counter))
            }
            // Get the last events from a number of counter.
            StoreCommand::LastEventsFrom(from) => {
                let events =
                    self.events(from, self.event_counter).map_err(|e| {
                        ActorError::Store(format!(
                            "Unable to get the latest events: {}",
                            e
                        ))
                    })?;
                Ok(StoreResponse::Events(events))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::vec;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::memory::MemoryManager;

    use actor::{ActorRef, ActorSystem, Error as ActorError};

    use async_trait::async_trait;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestActor {
        pub version: usize,
        pub value: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestActorLight {
        pub data: Vec<i32>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum TestMessageLight {
        SetData(Vec<i32>),
        GetData,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum TestMessage {
        Increment(i32),
        Recover,
        Snapshot,
        GetValue,
    }

    impl Message for TestMessage {}
    impl Message for TestMessageLight {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestEvent(i32);

    impl Event for TestEvent {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestEventLight(Vec<i32>);

    impl Event for TestEventLight {}

    #[derive(Debug, Clone, PartialEq)]
    enum TestResponse {
        Value(i32),
        None,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TestResponseLight {
        Data(Vec<i32>),
        None,
    }

    impl Response for TestResponse {}
    impl Response for TestResponseLight {}

    #[async_trait]
    impl Actor for TestActorLight {
        type Message = TestMessageLight;
        type Event = TestEventLight;
        type Response = TestResponseLight;

        async fn pre_start(
            &mut self,
            ctx: &mut ActorContext<Self>,
        ) -> Result<(), ActorError> {
            let memory_db: MemoryManager =
                ctx.system().get_helper("db").await.unwrap();

            let db = Store::<Self>::new(
                "store",
                "prefix",
                memory_db,
                Some([3u8; 32]),
            )
            .unwrap();

            let store = ctx.create_child("store", db).await.unwrap();
            let response = store
                .ask(StoreCommand::Recover)
                .await?;

            if let StoreResponse::State(Some(state)) = response {
                self.update(state);
            } else {
                debug!("Create first snapshot");
                store
                    .tell(StoreCommand::Snapshot(self.clone()))
                    .await
                    .unwrap();
            }

            Ok(())
        }

        async fn pre_stop(
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
                store.ask_stop().await
            } else {
                Err(ActorError::Store("Can't snapshot state".to_string()))
            }
        }
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
            let db = Store::<Self>::new(
                "store",
                "prefix",
                MemoryManager::default(),
                None,
            )
            .unwrap();
            let store = ctx.create_child("store", db).await.unwrap();
            let response = store
                .ask(StoreCommand::Recover)
                .await
                .unwrap();
            debug!("Recover response: {:?}", response);
            if let StoreResponse::State(Some(state)) = response {
                debug!("Recovering state: {:?}", state);
                self.update(state);
            }
            Ok(())
        }

        async fn pre_stop(
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
                store.ask_stop().await
            } else {
                Err(ActorError::Store("Can't snapshot state".to_string()))
            }
        }
    }

    #[async_trait]
    impl PersistentActor for TestActorLight {
        type Persistence = LightPersistence;

        fn apply(&mut self, event: &Self::Event) -> Result<(), ActorError> {
            self.data.clone_from(&event.0);
            Ok(())
        }
    }

    #[async_trait]
    impl PersistentActor for TestActor {
        type Persistence = FullPersistence;

        fn apply(&mut self, event: &Self::Event) -> Result<(), ActorError> {
            self.version += 1;
            self.value += event.0;
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<TestActorLight> for TestActorLight {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: TestMessageLight,
            ctx: &mut ActorContext<TestActorLight>,
        ) -> Result<TestResponseLight, ActorError> {
            match msg {
                TestMessageLight::SetData(data) => {
                    self.on_event(TestEventLight(data), ctx).await;
                    Ok(TestResponseLight::None)
                }
                TestMessageLight::GetData => {
                    Ok(TestResponseLight::Data(self.data.clone()))
                }
            }
        }

        async fn on_event(
            &mut self,
            event: TestEventLight,
            ctx: &mut ActorContext<TestActorLight>,
        ) -> () {
            self.persist(&event, ctx).await.unwrap();
        }
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: TestMessage,
            ctx: &mut ActorContext<TestActor>,
        ) -> Result<TestResponse, ActorError> {
            match msg {
                TestMessage::Increment(value) => {
                    let event = TestEvent(value);
                    self.on_event(event, ctx).await;
                    Ok(TestResponse::None)
                }
                TestMessage::Recover => {
                    let store: ActorRef<Store<Self>> =
                        ctx.get_child("store").await.unwrap();
                    let response = store
                        .ask(StoreCommand::Recover)
                        .await
                        .unwrap();
                    if let StoreResponse::State(Some(state)) = response {
                        self.update(state.clone());
                        Ok(TestResponse::Value(state.value))
                    } else {
                        Ok(TestResponse::None)
                    }
                }
                TestMessage::Snapshot => {
                    let store: ActorRef<Store<Self>> =
                        ctx.get_child("store").await.unwrap();
                    store
                        .ask(StoreCommand::Snapshot(self.clone()))
                        .await
                        .unwrap();
                    Ok(TestResponse::None)
                }
                TestMessage::GetValue => Ok(TestResponse::Value(self.value)),
            }
        }

        async fn on_event(
            &mut self,
            event: TestEvent,
            ctx: &mut ActorContext<TestActor>,
        ) -> () {
            self.persist(&event, ctx).await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_store_actor() {
        let (system, mut runner) = ActorSystem::create(CancellationToken::new());
        // Init runner.
        tokio::spawn(async move {
            runner.run().await;
        });
        let password = b"0123456789abcdef0123456789abcdef";
        let db = Store::<TestActor>::new(
            "store",
            "test",
            MemoryManager::default(),
            Some(*password),
        )
        .unwrap();
        let store = system.create_root_actor("store", db).await.unwrap();

        let mut actor = TestActor {
            version: 0,
            value: 0,
        };
        store
            .tell(StoreCommand::Snapshot(actor.clone()))
            .await
            .unwrap();
        store
            .tell(StoreCommand::Persist(TestEvent(10)))
            .await
            .unwrap();
        actor.apply(&TestEvent(10)).unwrap();
        store
            .tell(StoreCommand::Snapshot(actor.clone()))
            .await
            .unwrap();
        store
            .tell(StoreCommand::Persist(TestEvent(10)))
            .await
            .unwrap();

        actor.apply(&TestEvent(10)).unwrap();
        let response = store
            .ask(StoreCommand::Recover)
            .await
            .unwrap();
        if let StoreResponse::State(Some(state)) = response {
            assert_eq!(state.value, actor.value);
        }
        let response = store
            .ask(StoreCommand::Recover)
            .await
            .unwrap();
        if let StoreResponse::State(Some(state)) = response {
            assert_eq!(state.value, actor.value);
        }
        let response = store.ask(StoreCommand::LastEvent).await.unwrap();
        if let StoreResponse::LastEvent(Some(event)) = response {
            assert_eq!(event.0, 10);
        } else {
            panic!("Event not found");
        }
        let response = store.ask(StoreCommand::LastEventNumber).await.unwrap();
        if let StoreResponse::LastEventNumber(number) = response {
            assert_eq!(number, 1);
        } else {
            panic!("Event number not found");
        }
        let response =
            store.ask(StoreCommand::LastEventsFrom(1)).await.unwrap();
        if let StoreResponse::Events(events) = response {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].0, 10);
        } else {
            panic!("Events not found");
        }
        let response = store
            .ask(StoreCommand::GetEvents { from: 0, to: 2 })
            .await
            .unwrap();
        if let StoreResponse::Events(events) = response {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].0, 10);
            assert_eq!(events[1].0, 10);
        } else {
            panic!("Events not found");
        }
    }

    #[tokio::test]
    async fn test_persistent_light_actor() {
        let (system, ..) = ActorSystem::create(CancellationToken::new());

        system.add_helper("db", MemoryManager::default()).await;

        let actor = TestActorLight { data: vec![] };

        let actor_ref = system.create_root_actor("test", actor).await.unwrap();

        let result = actor_ref
            .ask(TestMessageLight::SetData(vec![12, 13, 14, 15]))
            .await
            .unwrap();

        assert_eq!(result, TestResponseLight::None);

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        actor_ref.ask_stop().await.unwrap();

        let actor = TestActorLight { data: vec![] };

        let actor_ref = system.create_root_actor("test", actor).await.unwrap();

        let result = actor_ref.ask(TestMessageLight::GetData).await.unwrap();

        let TestResponseLight::Data(data) = result else {
            panic!("Invalid response")
        };

        assert_eq!(data, vec![12, 13, 14, 15]);
    }

    #[tokio::test]
    //#[traced_test]
    async fn test_persistent_actor() {
        let (system, mut runner) = ActorSystem::create(CancellationToken::new());
        // Init runner.
        tokio::spawn(async move {
            runner.run().await;
        });

        let actor = TestActor {
            version: 0,
            value: 0,
        };

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

        assert_eq!(value, TestResponse::Value(20));

        actor_ref.ask_stop().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn test_encrypt_decrypt() {
        let key = [0u8; 32];
        let store = Store::<TestActor>::new(
            "store",
            "test",
            MemoryManager::default(),
            Some(key),
        )
        .unwrap();
        let data = b"Hello, world!";
        let encrypted = store.encrypt(&key, data).unwrap();
        let decrypted = store.decrypt(&key, &encrypted).unwrap();
        assert_eq!(data, decrypted.as_slice());
    }
}
