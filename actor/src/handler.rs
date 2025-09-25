// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

use crate::{
    ActorPath, Error,
    actor::{Actor, ActorContext, Handler},
};

use async_trait::async_trait;

use tokio::sync::{mpsc, oneshot};

use tracing::{debug, error};

use std::marker::PhantomData;

/// Message handler trait for processing actor messages.
///
/// This trait defines the interface for handling messages sent to actors.
/// It provides the core message processing capability for the actor system.
///
/// # Type Parameters
///
/// * `A` - The actor type that this handler processes messages for
///
/// # Thread Safety
///
/// This trait requires `Send + Sync` to ensure handlers can be safely shared
/// across threads in the concurrent actor system.
#[async_trait]
pub trait MessageHandler<A: Actor>: Send + Sync {
    /// Handles an incoming message for the specified actor.
    ///
    /// This method is called when a message needs to be processed by an actor.
    /// The handler receives mutable access to the actor, allowing it to modify
    /// the actor's state as needed.
    ///
    /// # Parameters
    ///
    /// * `actor` - Mutable reference to the actor that will process the message
    /// * `ctx` - Mutable reference to the actor's execution context, providing
    ///           access to system services like creating child actors, sending
    ///           messages, and publishing events
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use async_trait::async_trait;
    ///
    /// struct MyHandler;
    ///
    /// #[async_trait]
    /// impl MessageHandler<MyActor> for MyHandler {
    ///     async fn handle(&mut self, actor: &mut MyActor, ctx: &mut ActorContext<MyActor>) {
    ///         // Process the message and update actor state
    ///         actor.process_message();
    ///
    ///         // Use context for actor system operations
    ///         ctx.publish_event(MyEvent::MessageProcessed).await.ok();
    ///     }
    /// }
    /// ```
    async fn handle(&mut self, actor: &mut A, ctx: &mut ActorContext<A>);
}

/// Internal actor message container.
///
/// This structure encapsulates a message sent to an actor along with metadata
/// required for processing and response handling. It's used internally by the
/// actor system to manage message delivery and response coordination.
///
/// # Type Parameters
///
/// * `A` - The target actor type that will receive and process this message
///
/// # Fields
///
/// * `message` - The actual message payload to be processed by the actor
/// * `sender` - Path identifying the actor that sent this message (for logging/tracing)
/// * `rsvp` - Optional response sender for ask-pattern messages that expect a reply
/// * `_phantom_actor` - Phantom data to ensure proper type safety at compile time
struct ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// The message payload that the actor will process
    message: A::Message,
    /// Path of the actor that sent this message
    sender: ActorPath,
    /// Optional channel sender for returning responses (used in ask pattern)
    rsvp: Option<oneshot::Sender<Result<A::Response, Error>>>,
    /// Phantom data to maintain type safety
    _phantom_actor: PhantomData<A>,
}

/// Internal actor message implementation.
impl<A> ActorMessage<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new internal actor message.
    ///
    /// This constructor builds an actor message container that will be used
    /// internally by the actor system for message delivery and processing.
    ///
    /// # Parameters
    ///
    /// * `message` - The message payload that the target actor should process
    /// * `sender` - The path of the actor sending this message (used for tracing)
    /// * `rsvp` - Optional response channel sender for ask-pattern communication.
    ///            If provided, the actor will send its response through this channel.
    ///            If `None`, this is a tell-pattern message with no expected response.
    ///
    /// # Returns
    ///
    /// Returns a new `ActorMessage` instance ready for delivery to the target actor.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Create a tell message (no response expected)
    /// let tell_msg = ActorMessage::new(
    ///     MyMessage::DoSomething,
    ///     ActorPath::from("/user/sender"),
    ///     None
    /// );
    ///
    /// // Create an ask message (response expected)
    /// let (tx, rx) = oneshot::channel();
    /// let ask_msg = ActorMessage::new(
    ///     MyMessage::Calculate(42),
    ///     ActorPath::from("/user/sender"),
    ///     Some(tx)
    /// );
    /// ```
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
#[async_trait]
impl<A> MessageHandler<A> for ActorMessage<A>
where
    A: Actor + Handler<A>,
{
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

    /// Ask message to the actor.
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
        assert!(!sender.is_closed());
        assert!(!receiver.is_closed());
    }
}
