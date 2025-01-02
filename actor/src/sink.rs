// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

use crate::Event;

use async_trait::async_trait;
use tokio::sync::broadcast::{error::RecvError, Receiver as EventReceiver};

use tracing::debug;

pub struct Sink<E: Event> {
    subscriber: Box<dyn Subscriber<E>>,
    event_receiver: EventReceiver<E>,
}

impl<E: Event> Sink<E> {
    pub fn new(
        event_receiver: EventReceiver<E>,
        subscriber: impl Subscriber<E>,
    ) -> Self {
        Sink {
            subscriber: Box::new(subscriber),
            event_receiver,
        }
    }

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

#[async_trait]
pub trait Subscriber<E: Event>: Send + Sync + 'static {
    async fn notify(&self, event: E);
}
