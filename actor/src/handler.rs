// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

use crate::{
    actor::{Actor, ActorContext, Handler},
    ActorPath, Error,
};

use async_trait::async_trait;

use tokio::sync::{mpsc, oneshot};

use tracing::{debug, error};

use std::marker::PhantomData;

/// Message handler trait for actors messages.
#[async_trait]
pub trait MessageHandler<A: Actor>: Send + Sync {
    /// Handles the message.
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>);
}

/// Internal actor message.
struct ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    message: Option<A::Message>,
    sender: ActorPath,
    rsvp: Option<oneshot::Sender<Result<A::Response, Error>>>,
    _phantom_actor: PhantomData<A>,
}

/// Internal actor message implementation.
impl<A> ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// Creates internal actor message from message and optional reponse sender.
    pub fn new(
        message: Option<A::Message>,
        sender: ActorPath,
        rsvp: Option<oneshot::Sender<Result<A::Response, Error>>>,
    ) -> Self {
        debug!("Creating new internal actor message.");
        Self {
            message,
            sender,
            rsvp,
            _phantom_actor: PhantomData,
        }
    }
}

/// Message handler implementation for internal actor message.
#[async_trait]
impl<A> MessageHandler<A> for ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>) {
        debug!("Handling internal message.");
        if let Some(message) = &self.message {
            debug!("Handling message.");
            let result = actor
                .handle_message(self.sender.clone(), message.clone(), ctx)
                .await;

            if let Some(rsvp) = self.rsvp.take() {
                debug!("Sending back response (if any).");
                rsvp.send(result).unwrap_or_else(|_failed| {
                    error!("Failed to send back response!"); // GRCOV-LINE
                }) // GRCOV-LINE
            }
        } else {
            debug!("Stopping actor.");
            // TODO: Manage pre_stop error
            if actor.pre_stop(ctx).await.is_err() {
                error!("Failed to stop actor!"); // GRCOV-LINE
                let _ = ctx.emit_fail(Error::Stop).await; // GRCOV-LINE
            }
            ctx.stop().await;
        }
    }
}

/// Boxed message handler.
pub type BoxedMessageHandler<A> = Box<dyn MessageHandler<A>>;

/// Mailbo receiver.
pub type MailboxReceiver<A> = mpsc::UnboundedReceiver<BoxedMessageHandler<A>>;

/// Mailbox sender.
pub type MailboxSender<A> = mpsc::UnboundedSender<BoxedMessageHandler<A>>;

/// Mailbox.
pub type Mailbox<A> = (MailboxSender<A>, MailboxReceiver<A>);

/// Mailbox factory.
pub fn mailbox<A>() -> Mailbox<A> {
    mpsc::unbounded_channel()
}

/// Handle reference to send messages
pub struct HandleHelper<A> {
    sender: MailboxSender<A>,
}

impl<A> HandleHelper<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new handle reference.
    pub(crate) fn new(sender: MailboxSender<A>) -> Self {
        debug!("Creating new handle reference.");
        Self { sender }
    }

    /// Tell messasge to the actor.
    pub(crate) async fn tell(
        &self,
        sender: ActorPath,
        message: A::Message,
    ) -> Result<(), Error> {
        debug!("Telling message to actor from handle reference.");
        let msg = ActorMessage::new(Some(message), sender, None);
        if let Err(error) = self.sender.send(Box::new(msg)) {
            error!("Failed to tell message! {}", error.to_string()); // GRCOV-START
            Err(Error::Send(error.to_string()))
        } else {
            // GRCOV-END
            debug!("Message sent successfully.");
            Ok(())
        }
    }

    /// Ask message to the actor.
    pub(crate) async fn ask(
        &self,
        sender: ActorPath,
        message: A::Message,
    ) -> Result<A::Response, Error> {
        debug!("Asking message to actor from handle reference.");
        let (response_sender, response_receiver) = oneshot::channel();
        let msg =
            ActorMessage::new(Some(message), sender, Some(response_sender));
        if let Err(error) = self.sender.send(Box::new(msg)) {
            error!("Failed to ask message! {}", error.to_string()); // GRCOV-START
            Err(Error::Send(error.to_string()))
        } else {
            // GRCOV-END
            response_receiver
                .await
                .map_err(|error| Error::Send(error.to_string()))? // GRCOV-LINE
        }
    }

    /// Stop the actor.
    pub(crate) async fn stop(&self, sender: ActorPath) {
        debug!("Stopping actor from handle reference.");
        let msg: ActorMessage<A> = ActorMessage::new(None, sender, None);
        if let Err(error) = self.sender.send(Box::new(msg)) {
            error!("Failed to stop actor! {}", error.to_string()); // GRCOV-LINE
        } // GRCOV-LINE
    }

    /// Closes the sender.
    pub async fn close(&self) {
        self.sender.closed().await;
    }

    /// True if the sender is closed.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

impl<A> Clone for HandleHelper<A> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_mailbox() {
        let (sender, receiver) = mailbox::<()>();
        assert_eq!(sender.is_closed(), false);
        assert_eq!(receiver.is_closed(), false);
    }
}
