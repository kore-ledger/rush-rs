// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! # Actor system
//!
//! The `system` module provides the `ActorSystem` type. The `ActorSystem` type is the responsible for
//! creating and managing actors.
//!

use crate::{
    actor::DummyActor, error::ErrorHelper, runner::ActorRunner, Actor,
    ActorPath, ActorRef, Error, Handler,
};

use tokio::sync::RwLock;

use tracing::{debug, error};

use std::{any::Any, collections::HashMap, sync::Arc};

/// Actor system.
///
#[derive(Clone, Default)]
pub struct ActorSystem {
    actors:
        Arc<RwLock<HashMap<ActorPath, Box<dyn Any + Send + Sync + 'static>>>>,
}

impl ActorSystem {
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
    pub(crate) async fn create_actor_path<A, P>(
        &self,
        path: ActorPath,
        actor: A,
        error_helper: Option<ErrorHelper<P>>,
    ) -> Result<ActorRef<A>, Error>
    where
        A: Actor + Handler<A>,
        P: Actor + Handler<P>,
    {
        // Check if the actor already exists.
        let mut actors = self.actors.write().await;
        if actors.contains_key(&path) {
            error!("Actor '{}' already exists!", &path);
            return Err(Error::Exists(path));
        }

        // Create the actor runner and init it.
        let system = self.clone();
        let (mut runner, actor_ref) =
            ActorRunner::create(path, actor, error_helper);
        tokio::spawn(async move {
            runner.init(system).await;
        });

        // Store the actor reference.
        let path = actor_ref.path().clone();
        let any = Box::new(actor_ref.clone());
        actors.insert(path, any);

        Ok(actor_ref)
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
        self.create_actor_path::<A, DummyActor>(path, actor, None)
            .await
    }

    /// Retrieve or create a new actor on this actor system if it does not exist yet.
    pub(crate) async fn get_or_create_actor_path<A, P, F>(
        &self,
        path: &ActorPath,
        error_helper: Option<ErrorHelper<P>>,
        actor_fn: F,
    ) -> Result<ActorRef<A>, Error>
    where
        A: Actor + Handler<A>,
        P: Actor + Handler<P>,
        F: FnOnce() -> A,
    {
        let actors = self.actors.read().await;
        match self.get_actor(path).await {
            Some(actor) => Ok(actor),
            None => {
                drop(actors);
                self.create_actor_path(path.clone(), actor_fn(), error_helper)
                    .await
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
        self.get_or_create_actor_path::<A, DummyActor, F>(&path, None, actor_fn)
            .await
    }

    /// Stops the actor on this actor system. All its children will also be stopped.
    /// If the actor does not exist, nothing happens.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor to stop.
    ///
    pub async fn stop_actor(&self, path: &ActorPath) {
        debug!("Stopping actor '{}'...", &path);
        let mut paths: Vec<ActorPath> = vec![path.clone()];
        paths.extend(self.children(path).await);
        paths.sort_unstable();
        paths.reverse();
        let mut actors = self.actors.write().await;
        for path in &paths {
            actors.remove(path);
        }
    }

    /// Get the actor's children.
    async fn children(&self, path: &ActorPath) -> Vec<ActorPath> {
        let actors = self.actors.read().await;
        let mut children = vec![];
        for actor in actors.keys() {
            if actor.is_child_of(path) {
                children.push(actor.clone());
            }
        }
        children
    }
}
