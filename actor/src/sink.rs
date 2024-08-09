// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

use crate::Event;

use tokio::sync::broadcast::{Receiver as EventReceiver, error::RecvError};

use tracing::{error, debug};

pub struct Sink<E: Event> {
    subscriber: Box<dyn Subscriber<E>>,
    event_receiver: EventReceiver<E>,
}

impl<E: Event> Sink<E> {
    pub fn new(event_receiver: EventReceiver<E>, subscriber: impl Subscriber<E>) -> Self {
        Sink {
            subscriber: Box::new(subscriber),
            event_receiver,
        }
    }

    pub async fn run(&mut self) {
        loop {
            match self.event_receiver.recv().await {
                Ok(event) => {   
                    debug!("Received event: {:?}. Notify to the subscribers.", event);         
                    self.subscriber.notify(event);
                },
                Err(error) => {
                    error!("Error receiving event: {:?}", error);
                    match error {
                        RecvError::Closed => break,
                        RecvError::Lagged(_) => {
                            // If the receiver is lagging, we should try to catch up
                            // by processing the events that are still in the channel.
                            continue;
                        },
                    }
                }
            }
        }
    }
}

pub trait Subscriber<E: Event>: Send + Sync + 'static{
    fn notify(&self, event: E);
}