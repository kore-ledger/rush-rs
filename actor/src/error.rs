// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Errors module
//!

use thiserror::Error;

use crate::path::ActorPath;

/// Error type for the actor system.
#[derive(Debug, Error, PartialEq)]
pub enum Error {
    /// An error occurred while sending a message to an actor.
    #[error("An error occurred while sending a message to actor: {0}.")]
    Send(String),
    /// An error occurred while receiving a message from an actor.
    #[error("An error occurred while receiving a message from {0}:{1} actor.")]
    Receive(ActorPath, String),
    /// An error occurred while creating an actor.
    #[error("An error occurred while creating an actor.")]
    Create,
    /// An error occurred while retrieving an actor.
    #[error("Actor {0} exist.")]
    Exists(ActorPath),
    /// An error occurred while stopping an actor.
    #[error("An error occurred while stopping an actor.")]
    Stop,
    /// An error occurred while starting the actor system.
    #[error("An error occurred while starting the actor system.")]
    Start,
    /// An error occurred while sending an envent to event bus.
    #[error("An error occurred while sending an event to event bus.")]
    SendEvent,
    /// Create store error.
    #[error("Can't create store: {0}")]
    CreateStore(String),
    /// Get data error.
    #[error("Get error: {0}")]
    Get(String),
    /// Entry not found error.
    #[error("Entry not found.")]
    EntryNotFound,
    /// Store  Error.
    #[error("Store error: {0}")]
    Store(String),
}
