// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! Core library for the Rush framework.
//! Provides the foundational components for building actor-based applications.
//! This library includes the core actor model, message passing, and persistence layers.
//! It is designed to be modular and extensible, allowing developers to build custom actors and message types.

pub use actor::{
    Actor,  ActorRef, ActorSystem, ActorContext, ActorPath, Handler,
    Message, Event, Response, Error as ActorError,
};

pub use store::{
    Error as StoreError,
    store::PersistentActor,
    database::{Collection, DbManager},
};

#[cfg(feature = "rocksdb_db")]
pub use rocksdb_db::{RocksDbManager, RocksDbStore};

#[cfg(feature = "sqlite_db")]
pub use sqlite_db::{SqliteCollection, SqliteManager};