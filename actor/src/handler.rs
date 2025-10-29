

use crate::{
    ActorPath, Error,
    actor::{Actor, ActorContext, Handler},
};

use async_trait::async_trait;

use tokio::sync::{mpsc, oneshot};

use tracing::{debug, error};

use std::marker::PhantomData;

/// Message handler trait for processing actor messages.
/// This trait abstracts the handling of different message types,
/// allowing the actor system to process messages uniformly regardless
/// of whether they expect a response or not.
#[async_trait]
pub trait MessageHandler<A: Actor>: Send + Sync {
    /// Handles a message for the given actor.
    ///
    /// # Arguments
    ///
    /// * `actor` - Mutable reference to the actor processing the message.
    /// * `ctx` - Actor context providing access to system and actor state.
    ///
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>);
}

/// Internal actor message wrapper that encapsulates the message content,
/// sender information, and optional response channel for request-response patterns.
struct ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// The actual message to be processed by the actor.
    message: A::Message,
    /// The path of the actor that sent this message.
    sender: ActorPath,
    /// Optional response channel for request-response (ask) pattern.
    /// If Some, the handler will send the response back through this channel.
    /// If None, this is a fire-and-forget (tell) message.
    rsvp: Option<oneshot::Sender<Result<A::Response, Error>>>,
    /// Phantom data to associate the message with actor type A at compile time.
    _phantom_actor: PhantomData<A>,
}

/// Internal actor message implementation.
impl<A> ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new internal actor message from message content and optional response sender.
    ///
    /// # Arguments
    ///
    /// * `message` - The message to be delivered to the actor.
    /// * `sender` - The path of the sending actor.
    /// * `rsvp` - Optional channel to send the response back (for ask pattern).
    ///
    /// # Returns
    ///
    /// Returns a new ActorMessage ready to be sent to the actor's mailbox.
    ///
    pub fn new(
        message: A::Message,
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
/// This implementation calls the actor's handle_message method and,
/// if a response channel exists, sends the result back to the caller.
#[async_trait]
impl<A> MessageHandler<A> for ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// Handles the message by delegating to the actor's handle_message method.
    /// If this message expects a response (rsvp is Some), sends the result back.
    ///
    /// # Arguments
    ///
    /// * `actor` - The actor that will process this message.
    /// * `ctx` - The actor's execution context.
    ///
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>) {
        debug!("Handling internal message.");

        debug!("Handling message.");
        let result = actor
            .handle_message(self.sender.clone(), self.message.clone(), ctx)
            .await;

        if let Some(rsvp) = self.rsvp.take() {
            debug!("Sending back response (if any).");
            rsvp.send(result).unwrap_or_else(|_failed| {
                error!("Failed to send back response!"); // GRCOV-LINE
            }) // GRCOV-LINE
        }
    }
}

/// Boxed message handler for type-erased message handling.
/// This allows different message types to be stored in the same mailbox.
pub type BoxedMessageHandler<A> = Box<dyn MessageHandler<A>>;

/// Mailbox receiver side for consuming messages from the actor's queue.
/// The actor's main loop will receive messages through this channel.
pub type MailboxReceiver<A> = mpsc::UnboundedReceiver<BoxedMessageHandler<A>>;

/// Mailbox sender side for sending messages to an actor's queue.
/// Multiple references can share the same sender to communicate with an actor.
pub type MailboxSender<A> = mpsc::UnboundedSender<BoxedMessageHandler<A>>;

/// Complete mailbox tuple containing both sender and receiver sides.
/// Created during actor initialization and split for use by different components.
pub type Mailbox<A> = (MailboxSender<A>, MailboxReceiver<A>);

/// Creates a new unbounded mailbox for an actor.
/// The mailbox uses an unbounded channel to prevent message send operations
/// from blocking, delegating backpressure management to the application level.
///
/// # Returns
///
/// Returns a tuple of (sender, receiver) for the actor's mailbox.
///
pub fn mailbox<A>() -> Mailbox<A> {
    mpsc::unbounded_channel()
}

/// Handle helper for sending messages to an actor.
/// This is an internal abstraction that wraps the mailbox sender
/// and provides typed message sending methods (tell and ask).
pub struct HandleHelper<A> {
    /// The underlying mailbox sender for this actor.
    sender: MailboxSender<A>,
}

impl<A> HandleHelper<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new handle helper from a mailbox sender.
    ///
    /// # Arguments
    ///
    /// * `sender` - The mailbox sender to wrap.
    ///
    /// # Returns
    ///
    /// Returns a new HandleHelper instance.
    ///
    pub(crate) fn new(sender: MailboxSender<A>) -> Self {
        debug!("Creating new handle reference.");
        Self { sender }
    }

    /// Sends a message to the actor without expecting a response (fire-and-forget).
    /// This is the "tell" pattern in actor terminology.
    ///
    /// # Arguments
    ///
    /// * `sender` - The path of the actor sending the message.
    /// * `message` - The message to send.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the message was queued successfully.
    ///
    /// # Errors
    ///
    /// Returns Error::Send if the actor's mailbox is closed or full.
    ///
    pub(crate) async fn tell(
        &self,
        sender: ActorPath,
        message: A::Message,
    ) -> Result<(), Error> {
        debug!("Telling message to actor from handle reference.");
        let msg = ActorMessage::new(message, sender, None);
        if let Err(error) = self.sender.send(Box::new(msg)) {
            debug!("Failed to tell message! {}", error.to_string()); // GRCOV-START
            Err(Error::Send(error.to_string()))
        } else {
            // GRCOV-END
            debug!("Message sent successfully.");
            Ok(())
        }
    }

    /// Sends a message to the actor and waits for a response (request-response).
    /// This is the "ask" pattern in actor terminology.
    ///
    /// # Arguments
    ///
    /// * `sender` - The path of the actor sending the message.
    /// * `message` - The message to send.
    ///
    /// # Returns
    ///
    /// Returns the actor's response if successful.
    ///
    /// # Errors
    ///
    /// Returns Error::Send if the message couldn't be sent or if
    /// the response channel was closed before receiving a response.
    ///
    pub(crate) async fn ask(
        &self,
        sender: ActorPath,
        message: A::Message,
    ) -> Result<A::Response, Error> {
        debug!("Asking message to actor from handle reference.");
        let (response_sender, response_receiver) = oneshot::channel();
        let msg = ActorMessage::new(message, sender, Some(response_sender));
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

    /// Waits for the sender to be closed.
    /// This method will block until all senders are dropped.
    ///
    pub async fn close(&self) {
        self.sender.closed().await;
    }

    /// Checks if the sender is closed.
    ///
    /// # Returns
    ///
    /// Returns true if the actor's mailbox is closed and cannot receive more messages.
    ///
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
