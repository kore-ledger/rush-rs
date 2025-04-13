// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Simplified actor model
//!
//! This crate provides a simplified actor model implementation. It is based on the
//! [actor model](https://en.wikipedia.org/wiki/Actor_model) as described by Carl Hewitt in 1973.
//!
//! The actor model is a model of concurrent computation that treats "actors" as the universal
//! primitives of concurrent computation. In response to a message that it receives, an actor can:
//!
//! - make local decisions
//! - update its private state
//! - create more actors
//! - send more messages
//! - determine how to respond to the next message received
//!
//! Actors may modify their own private state, but can only affect each other indirectly through
//! messaging (no actor can access the state of another actor directly).
//!

mod actor;
mod error;
mod handler;
mod path;
mod retries;
mod runner;
mod sink;
mod supervision;
mod system;

pub use actor::{
    Actor, ActorContext, ActorRef, ChildAction, Event, Handler, Message,
    Response,
};
pub use error::Error;
pub use path::ActorPath;

pub use sink::{Sink, Subscriber};

pub use retries::{RetryActor, RetryMessage};
pub use supervision::{
    CustomIntervalStrategy, FixedIntervalStrategy, NoIntervalStrategy,
    RetryStrategy, Strategy, SupervisionStrategy,
};
pub use system::{ActorSystem, SystemEvent, SystemRef, SystemRunner};
