// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor system
//!
//! The `system` module provides the `ActorSystem` type. The `ActorSystem` type is the responsible for
//! creating and managing actors.
//!

use crate::{
    Actor, ActorPath, ActorRef, Error, Event, Handler,
    actor::ChildErrorSender,
    runner::{ActorRunner, ChildSender},
    sink::Sink,
};

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

use std::{any::Any, collections::HashMap, sync::Arc};

/// Actor system.
///
pub struct ActorSystem {}

/// Default implementation for `ActorSystem`.
impl ActorSystem {
    /// Create a new actor system.
    ///
    /// # Returns
    ///
    /// Returns a tuple with the system reference and the system runner.
    pub fn create(
        token: Option<CancellationToken>,
    ) -> (SystemRef, SystemRunner) {
        let token = if let Some(token) = token {
            token
        } else {
            CancellationToken::new()
        };

        let (event_sender, event_receiver) = mpsc::channel(100);
        let system = SystemRef::new(event_sender);
        let runner = SystemRunner::new(token, event_receiver);
        (system, runner)
    }
}

/// System event.
///
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// Stop the actor system.
    StopSystem,
}

/// System reference.
///
#[derive(Clone)]
pub struct SystemRef {
    /// The actors running in this actor system.
    actors:
        Arc<RwLock<HashMap<ActorPath, Box<dyn Any + Send + Sync + 'static>>>>,

    /// The helpers for this actor system.
    helpers: Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync + 'static>>>>,

    /// The root actor sender.
    senders: Arc<RwLock<Vec<ChildSender>>>,

    /// The event sender.
    event_sender: mpsc::Sender<SystemEvent>,
}

impl SystemRef {
    /// Create system reference.
    pub fn new(event_sender: mpsc::Sender<SystemEvent>) -> Self {
        SystemRef {
            actors: Arc::new(RwLock::new(HashMap::new())),
            helpers: Arc::new(RwLock::new(HashMap::new())),
            senders: Arc::new(RwLock::new(Vec::new())),
            event_sender,
        }
    }

    /// Retrieves an actor running in this actor system. If actor does not exist, a None
    /// is returned instead.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor to retrieve.
    ///
    /// # Returns
    ///
    /// Returns the actor reference.
    ///
    pub async fn get_actor<A>(&self, path: &ActorPath) -> Option<ActorRef<A>>
    where
        A: Actor + Handler<A>,
    {
        let actors = self.actors.read().await;
        actors
            .get(path)
            .and_then(|any| any.downcast_ref::<ActorRef<A>>().cloned())
    }

    /// Creates an actor in this actor system with the given path and actor type.
    /// If the actor already exists, an error is returned.
    pub(crate) async fn create_actor_path<A>(
        &self,
        path: ActorPath,
        actor: A,
        error_helper: Option<ChildErrorSender>,
    ) -> Result<(ActorRef<A>, ChildSender), Error>
    where
        A: Actor + Handler<A>,
    {
        // Check if the actor already exists.
        {
            let actors = self.actors.read().await;
            if actors.contains_key(&path) {
                error!("Actor '{}' already exists!", &path);
                return Err(Error::Exists(path));
            }
        }
        // Create the actor runner and init it.
        let system = self.clone();
        let (mut runner, actor_ref, child_sender) =
            ActorRunner::create(path.clone(), actor, error_helper);

        // Store the actor reference.
        let any = Box::new(actor_ref.clone());
        {
            let mut actors = self.actors.write().await;
            actors.insert(path.clone(), any);
        }
        let (sender, receiver) = oneshot::channel::<bool>();

        tokio::spawn(async move {
            runner.init(system, Some(sender)).await;
        });

        if receiver.await.map_err(|e| Error::Start(e.to_string()))? {
            Ok((actor_ref, child_sender))
        } else {
            Err(Error::Start(format!("Runner can not init {}", path)))
        }
    }

    /// Launches a new top level actor on th is actor system at the '/user'
    /// actor path. If another actor with the same name already exists,
    /// an `Err(Error::Exists(ActorPath))` is returned instead.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the actor to create.
    /// * `actor` - The type with `Actor` trait to create.
    /// * `error_helper` - The error helper actor (`None` it it is root actor).
    ///
    /// # Returns
    ///
    /// Returns the actor reference.
    ///
    /// # Error
    ///
    /// Returns an error if the actor already exists.
    ///
    pub async fn create_root_actor<A>(
        &self,
        name: &str,
        actor: A,
    ) -> Result<ActorRef<A>, Error>
    where
        A: Actor + Handler<A>,
    {
        let path = ActorPath::from("/user") / name;
        let (actor_ref, sender) =
            self.create_actor_path::<A>(path, actor, None).await?;
        let mut senders = self.senders.write().await;
        senders.push(sender);
        Ok(actor_ref)
    }

    /// Retrieve or create a new actor on this actor system if it does not exist yet.
    pub(crate) async fn get_or_create_actor_path<A, F>(
        &self,
        path: &ActorPath,
        error_helper: Option<ChildErrorSender>,
        actor_fn: F,
    ) -> Result<(ActorRef<A>, Option<ChildSender>), Error>
    where
        A: Actor + Handler<A>,
        F: FnOnce() -> A,
    {
        let actors = self.actors.read().await;
        match self.get_actor(path).await {
            Some(actor) => Ok((actor, None)),
            None => {
                drop(actors);
                let (actor_ref, sender) = self
                    .create_actor_path(path.clone(), actor_fn(), error_helper)
                    .await?;
                Ok((actor_ref, Some(sender)))
            }
        }
    }

    /// Retrieve or create a new actor on this actor system if it does not exist yet.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the actor to retrieve or create.
    /// * `actor_fn` - The function to create the actor if it does not exist.   
    ///
    /// # Returns
    ///
    /// Returns the actor reference.
    ///
    /// # Error
    ///
    /// Returns an error if the actor cannot be created.
    ///
    pub async fn get_or_create_actor<A, F>(
        &self,
        name: &str,
        actor_fn: F,
    ) -> Result<ActorRef<A>, Error>
    where
        A: Actor + Handler<A>,
        F: FnOnce() -> A,
    {
        let path = ActorPath::from("/user") / name;
        let (actor_ref, sender) = self
            .get_or_create_actor_path::<A, F>(&path, None, actor_fn)
            .await?;
        if let Some(sender) = sender {
            let mut senders = self.senders.write().await;
            senders.push(sender);
        }
        Ok(actor_ref)
    }

    /// Remove an actor from this actor system.
    /// If the actor does not exist, nothing happens.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor to remove.
    ///
    pub async fn remove_actor(&self, path: &ActorPath) {
        let mut actors = self.actors.write().await;
        actors.remove(path);
    }

    /// Send a system event.
    pub async fn send_event(&self, event: SystemEvent) {
        if let Err(error) = self.event_sender.send(event).await {
            error!("Failed to send event! {}", error.to_string());
        }
    }

    /// Get the actor's children.
    pub async fn children(&self, path: &ActorPath) -> Vec<ActorPath> {
        let actors = self.actors.read().await;
        let mut children = vec![];
        for actor in actors.keys() {
            if actor.is_child_of(path) {
                children.push(actor.clone());
            }
        }
        children
    }

    /// Add a helper to the actor system.
    pub async fn add_helper<H>(&self, name: &str, helper: H)
    where
        H: Any + Send + Sync + Clone + 'static,
    {
        let mut helpers = self.helpers.write().await;
        helpers.insert(name.to_owned(), Box::new(helper));
    }

    /// Get a helper from the actor system.
    /// If the helper does not exist, a None is returned.
    pub async fn get_helper<H>(&self, name: &str) -> Option<H>
    where
        H: Any + Send + Sync + Clone + 'static,
    {
        let helpers = self.helpers.read().await;
        helpers
            .get(name)
            .and_then(|any| any.downcast_ref::<H>())
            .cloned()
    }

    /// Run a sink. The sink will be run in a separate task.
    ///
    pub async fn run_sink<E>(&self, mut sink: Sink<E>)
    where
        E: Event,
    {
        tokio::spawn(async move {
            sink.run().await;
        });
    }
}

/// System runner.
pub struct SystemRunner {
    /// The cancellation token for the actor system.
    token: CancellationToken,

    /// The event receiver.
    event_receiver: mpsc::Receiver<SystemEvent>,
}

impl SystemRunner {
    /// Create a new system runner.
    pub(crate) fn new(
        token: CancellationToken,
        event_receiver: mpsc::Receiver<SystemEvent>,
    ) -> Self {
        Self {
            token,
            event_receiver,
        }
    }

    /// Run the actor system.
    pub async fn run(&mut self) {
        debug!("Running actor system...");
        loop {
            tokio::select! {
                Some(event) = self.event_receiver.recv() => {
                    match event {
                        SystemEvent::StopSystem => {
                            debug!("Stopping actor system...");
                            self.token.cancel();
                        }
                    }
                }
                _ = self.token.cancelled() => {
                    debug!("Actor system stopped.");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_stop_actor_system() {
        let (system, mut runner) = ActorSystem::create(None);

        tokio::spawn(async move {
            runner.run().await;
        });
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        assert!(logs_contain("Running actor system..."));
        system.send_event(SystemEvent::StopSystem).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        assert!(logs_contain("Stopping actor system..."));
        assert!(logs_contain("Actor system stopped."));
    }

    #[tokio::test]
    async fn test_helpers() {
        let (system, _) = ActorSystem::create(None);
        let helper = TestHelper { value: 42 };
        system.add_helper("test", helper).await;
        let helper: Option<TestHelper> = system.get_helper("test").await;
        assert_eq!(helper, Some(TestHelper { value: 42 }));
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct TestHelper {
        pub value: i32,
    }
}
