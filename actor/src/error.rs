// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Errors module
//!

use crate::ActorPath;

use thiserror::Error;
// GRCOV-START
// TODO: We should improve the error handling in the actor system.

/// Error type for the actor system.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum Error {
    /// An error occurred while sending a message to an actor.
    #[error("An error occurred while sending a message to actor: {0}.")]
    Send(String),
    /// An error occurred while receiving a response from an actor.
    #[error(
        "Actor {0} returned a response that was not expected, expected response: {1}"
    )]
    UnexpectedResponse(ActorPath, String),
    /// An error occurred while creating an actor.
    #[error("An error occurred while creating an actor: {0}/{1}.")]
    Create(ActorPath, String),
    /// An error occurred while retrieving an actor.
    #[error("Actor {0} exist.")]
    Exists(ActorPath),
    /// Actor not found error.
    #[error("Actor {0} not found.")]
    NotFound(ActorPath),
    /// An error occurred while stopping an actor.
    #[error("An error occurred while stopping an actor.")]
    Stop,
    /// An error occurred while starting the actor system.
    #[error("An error occurred while starting the actor system: {0}")]
    Start(String),
    /// An error occurred while sending an envent to event bus.
    #[error("An error occurred while sending an event to event bus: {0}")]
    SendEvent(String),
    /// Create store error.
    #[error("Can't create store: {0}")]
    CreateStore(String),
    /// Get data error.
    #[error("Get error: {0}")]
    Get(String),
    /// Entry not found error.
    #[error("Entry not found: {0}")]
    EntryNotFound(String),
    /// Store  Error.
    #[error("Store error: {0}")]
    Store(String),
    /// Error that does not compromise the operation of the system.
    #[error("Error: {0}")]
    Functional(String),
    /// Error that does compromise the operation of the system.
    #[error("Error: {0}")]
    FunctionalFail(String),
    /// An error that affects the state. Contains the valid state.
    #[error("State error: {0}")]
    State(String),
    /// The maximum number of retries has been reached.
    #[error("The maximum number of retries has been reached.")]
    ReTry,
    /// Can not get a helper.
    #[error("Helper {0} could not be accessed.")]
    NotHelper(String),
}
// GRCOV-END
