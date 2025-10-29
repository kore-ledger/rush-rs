

//! Event sink and subscriber pattern implementation.
//!
//! This module provides a sink/subscriber pattern for processing actor events.
//! Sinks run in separate tasks and receive events from actors via broadcast channels,
//! allowing for event-driven architectures and observability patterns.

use crate::Event;

use async_trait::async_trait;
use tokio::sync::broadcast::{Receiver as EventReceiver, error::RecvError};

use tracing::debug;

/// A sink that receives events from an actor and notifies a subscriber.
/// The Sink runs in its own task and processes events asynchronously,
/// implementing the Observer pattern for actor events.
///
/// # Type Parameters
///
/// * `E` - The event type that this sink will process.
///
pub struct Sink<E: Event> {
    /// The subscriber that will be notified of events.
    subscriber: Box<dyn Subscriber<E>>,
    /// The broadcast receiver for actor events.
    event_receiver: EventReceiver<E>,
}

impl<E: Event> Sink<E> {
    /// Creates a new Sink with the given event receiver and subscriber.
    ///
    /// # Arguments
    ///
    /// * `event_receiver` - Broadcast receiver subscribed to an actor's event channel.
    /// * `subscriber` - Implementation of the Subscriber trait that will process events.
    ///
    /// # Returns
    ///
    /// Returns a new Sink instance ready to be run.
    ///
    pub fn new(
        event_receiver: EventReceiver<E>,
        subscriber: impl Subscriber<E>,
    ) -> Self {
        Sink {
            subscriber: Box::new(subscriber),
            event_receiver,
        }
    }

    /// Runs the sink's event processing loop.
    /// This method will block and continuously process events until the
    /// event channel is closed. Should be spawned in a separate task.
    ///
    /// # Behavior
    ///
    /// - Receives events from the broadcast channel.
    /// - Notifies the subscriber of each event.
    /// - Handles lagged events by catching up (skips missed events).
    /// - Exits when the event channel is closed.
    ///
    pub async fn run(&mut self) {
        loop {
            match self.event_receiver.recv().await {
                Ok(event) => {
                    debug!(
                        "Received event: {:?}. Notify to the subscriber.",
                        event
                    );
                    self.subscriber.notify(event).await;
                }
                Err(error) => {
                    match error {
                        RecvError::Closed => break,
                        RecvError::Lagged(_) => {
                            // If the receiver is lagging, we should try to catch up
                            // by processing the events that are still in the channel.
                            continue;
                        }
                    }
                }
            }
        }
    }
}

/// Trait for types that can receive and process actor events.
/// Implement this trait to define custom event processing logic
/// that will be invoked by a Sink for each event received.
///
/// # Type Parameters
///
/// * `E` - The event type this subscriber can process.
///
#[async_trait]
pub trait Subscriber<E: Event>: Send + Sync + 'static {
    /// Called when an event is received by the sink.
    /// This method should contain the logic for processing the event.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to process.
    ///
    async fn notify(&self, event: E);
}
