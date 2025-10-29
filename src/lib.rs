

//! Core library for the Rush framework.
//! Provides the foundational components for building actor-based applications.
//! This library includes the core actor model, message passing, and persistence layers.
//! It is designed to be modular and extensible, allowing developers to build custom actors and message types.

pub use actor::{
    Actor, ActorContext, ActorPath, ActorRef, ActorSystem, ChildAction,
    CustomIntervalStrategy, Error as ActorError, Event, 
    FixedIntervalStrategy, Handler, Message, NoIntervalStrategy,
    Response, RetryActor, RetryMessage, RetryStrategy, Sink,
    Strategy, Subscriber, SupervisionStrategy, SystemEvent, SystemRef,
    SystemRunner,
};

#[cfg(any(feature = "rocksdb", feature = "sqlite"))]
pub use store::{
    Error as StoreError,
    database::{Collection, DbManager, State},
    store::{PersistentActor, FullPersistence, LightPersistence, Store, StoreCommand, StoreResponse}, 
};

#[cfg(feature = "rocksdb")]
pub use rocksdb_db::{RocksDbManager, RocksDbStore};

#[cfg(feature = "sqlite")]
pub use sqlite_db::{SqliteCollection, SqliteManager};
