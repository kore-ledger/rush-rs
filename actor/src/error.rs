// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Errors module
//!

use crate::{Actor, ActorContext, ActorPath, Handler};

use tokio::sync::mpsc;

use async_trait::async_trait;

use thiserror::Error;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use std::marker::PhantomData;

// TODO: We should improve the error handling in the actor system.

/// Error type for the actor system.
#[derive(Clone, Debug, Error, PartialEq, Serialize, Deserialize)]
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
    /// An error occurred while sending an error to parent actor.
    #[error("An error occurred while sending an error to parent actor.")]
    SendError,
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

/// Error handler trait for actors errors.
#[async_trait]
pub trait ErrorHandler<A: Actor>: Send + Sync {
    /// Handles the error.
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>);
}

/// Internal error message.
struct ErrorMessage<A: Actor> {
    /// The error.
    error: Error,
    /// Phantom data.
    _phantom_actor: PhantomData<A>,
} 

/// Internal error message implementation.
impl<A: Actor> ErrorMessage<A> {
    /// Creates internal error message from error.
    pub fn new(error: Error) -> Self {
        Self {
            error,
            _phantom_actor: PhantomData,
        }
    }
}

/// Error handler implementation for internal error message.
#[async_trait]
impl<A> ErrorHandler<A> for ErrorMessage<A> 
where
    A: Actor + Handler<A>,    
{
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>) {
        actor.handle_child_error(self.error.clone(), ctx).await;
    }
}

/// Boxed error handler type.
pub type BoxedErrorHandler<A> = Box<dyn ErrorHandler<A>>;

/// Error box receiver.
pub type ErrorBoxReceiver<A> = mpsc::UnboundedReceiver<BoxedErrorHandler<A>>;

/// Mailbox sender.
pub type ErrorBoxSender<A> = mpsc::UnboundedSender<BoxedErrorHandler<A>>;

/// Mailbox.
pub type ErrorBox<A> = (ErrorBoxSender<A>, ErrorBoxReceiver<A>);

/// Mailbox factory.
pub fn error_box<A>() -> ErrorBox<A> {
    mpsc::unbounded_channel()
}

/// Handle reference to send messages
pub struct ErrorHelper<A> {
    sender: ErrorBoxSender<A>,
}

impl<A> ErrorHelper<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new handle reference.
    pub(crate) fn new(sender: ErrorBoxSender<A>) -> Self {
        debug!("Creating new error handle reference.");
        Self { sender }
    }

    /// Sends an error to the actor.
    pub async fn send(&mut self, error: Error) -> Result<(), Error> {
        debug!("Sending error to parent actor.");
        let error = Box::new(ErrorMessage::new(error));
        if self.sender.send(error).is_err() {
            error!("Failed to send error to parent actor!");
            return Err(Error::Send("Error".to_string()));
        }
        Ok(())
        
    }

    /// True if the sender is closed.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

impl<A> Clone for ErrorHelper<A> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

