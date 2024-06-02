// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Errors module
//!

use crate::ActorPath;


use serde::{Deserialize, Serialize};
use thiserror::Error;


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
    /// Error that does not compromise the operation of the system.
    #[error("Error: {0}")]
    Functional(String),
}

/* 
/// System error type.
/// This type is used to send errors to the parent actor, when an error occurs in a child actor.
/// Distinguish between errors that can be dealt with at the level of the parent actor or failures
/// that require supervision.
///
#[derive(Debug)]
pub(crate) enum SystemError {
    /// Error.
    Error(Error),
    /// Fail.
    Fail(Error, Sender<ChildAction>),
}

impl Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemError::Error(error) => write!(f, "Error: {}", error),
            SystemError::Fail(error, _) => write!(f, "Fail: {}", error),
        }
    }
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
    error: SystemError,
    /// The response sender.
    sender: Option<Sender<ChildAction>>,
    /// Phantom data.
    _phantom_actor: PhantomData<A>,
}

/// Internal error message implementation.
impl<A: Actor> ErrorMessage<A> {
    /// Creates internal error message from error.
    pub fn new(
        error: SystemError,
        sender: Option<Sender<ChildAction>>,
    ) -> Self {
        Self {
            error,
            sender,
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
        debug!("Handling error in actor.");
        match &self.error {
            SystemError::Error(error) => {
                actor.on_child_error(error.clone(), ctx).await;
            }
            SystemError::Fail(error, sender) => {
                let action = actor.on_child_fault(error.clone(), ctx).await;
            }
        }
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
    pub(crate) async fn send(
        &mut self,
        error: SystemError,
    ) -> Result<(), Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{actor::DummyActor, runner::InnerEvent};

    #[tokio::test]
    async fn test_error_helper() {
        let (sender, mut receiver) = error_box::<DummyActor>();
        let (inner_sender, _) =
            mpsc::unbounded_channel::<InnerEvent<DummyActor>>();
        let mut error_helper = ErrorHelper::new(sender);
        let error = Error::Send("Error".to_string());
        //let inner_event: InnerEvent<DummyActor> = InnerEvent::Error(error.clone());
        error_helper
            .send(SystemError::Error(error.clone()))
            .await
            .unwrap();
        let error_handler = receiver.recv().await;
        assert!(error_handler.is_some());
        let mut error_handler = error_handler.unwrap();
        let mut actor = DummyActor;
        let mut ctx = ActorContext::dummy(error_helper, inner_sender);
        error_handler.handle(&mut actor, &mut ctx).await;
    }
}
*/