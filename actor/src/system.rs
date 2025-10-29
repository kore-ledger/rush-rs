

//! # Actor system
//!
//! The `system` module provides the `ActorSystem` type. The `ActorSystem` type is the responsible for
//! creating and managing actors.
//!

use crate::{
    Actor, ActorPath, ActorRef, Error, Event, Handler,
    actor::ChildErrorSender,
    runner::{ActorRunner, StopSender},
    sink::Sink,
};

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

use std::{any::Any, collections::HashMap, sync::Arc};

/// Actor system factory.
/// This is the main entry point for creating an actor system instance.
/// The ActorSystem provides factory methods for initializing the system
/// infrastructure including the SystemRef and SystemRunner.
///
pub struct ActorSystem {}

/// Default implementation for `ActorSystem`.
impl ActorSystem {
    /// Creates a new actor system with its reference and runner.
    /// The actor system manages the lifecycle of all actors and provides
    /// the infrastructure for actor communication and supervision.
    ///
    /// # Arguments
    ///
    /// * `token` - CancellationToken for coordinated shutdown of the entire system.
    ///
    /// # Returns
    ///
    /// Returns a tuple containing:
    /// - `SystemRef` - Cloneable reference for creating and managing actors.
    /// - `SystemRunner` - Event loop that must be run to process system events.
    ///
    /// # Example
    ///
    /// ```
    /// use tokio_util::sync::CancellationToken;
    /// use actor::ActorSystem;
    ///
    /// let token = CancellationToken::new();
    /// let (system, mut runner) = ActorSystem::create(token.clone());
    ///
    /// // Spawn the system runner
    /// tokio::spawn(async move {
    ///     runner.run().await;
    /// });
    ///
    /// // Use system to create actors
    /// // ...
    ///
    /// // Shutdown when done
    /// token.cancel();
    /// ```
    ///
    pub fn create(token: CancellationToken) -> (SystemRef, SystemRunner) {
        let (event_sender, event_receiver) = mpsc::channel(100);
        let system = SystemRef::new(event_sender, token);
        let runner = SystemRunner::new(event_receiver);
        (system, runner)
    }
}

/// System-level events for coordinating actor system lifecycle.
/// These events are used internally to manage system-wide operations.
///
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// Signals that the actor system should stop gracefully.
    /// This triggers shutdown of all root actors and system cleanup.
    StopSystem,
}

/// Cloneable reference to the actor system.
/// The SystemRef provides methods for creating, retrieving, and managing actors.
/// Multiple SystemRef instances can be cloned and used concurrently across
/// different parts of the application.
///
/// # Thread Safety
///
/// SystemRef is thread-safe and can be cloned and shared across tasks.
/// All operations use internal locks for safe concurrent access.
///
#[derive(Clone)]
pub struct SystemRef {
    /// Registry of all actors in the system, indexed by their paths.
    /// Uses type erasure (Any) to store heterogeneous actor types.
    actors:
        Arc<RwLock<HashMap<ActorPath, Box<dyn Any + Send + Sync + 'static>>>>,

    /// Registry of helper objects that can be shared across actors.
    /// Helpers can be any type (database connections, configurations, etc.).
    helpers: Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync + 'static>>>>,

    /// Stop senders for root-level actors to enable coordinated shutdown.
    root_senders: Arc<RwLock<Vec<StopSender>>>,

    /// Cancellation token for system-wide shutdown coordination.
    token: CancellationToken
}

impl SystemRef {
    /// Creates a new system reference with shutdown coordination.
    /// This method sets up the actor registry and spawns a task that
    /// listens for cancellation signals to coordinate system shutdown.
    ///
    /// # Arguments
    ///
    /// * `event_sender` - Channel for sending system events to the runner.
    /// * `token` - Cancellation token that triggers system shutdown when cancelled.
    ///
    /// # Returns
    ///
    /// Returns a new SystemRef instance.
    ///
    /// # Behavior
    ///
    /// Spawns a background task that:
    /// - Waits for the cancellation token to be cancelled.
    /// - Stops all root actors in order, waiting for each to confirm.
    /// - Sends a StopSystem event to the runner to complete shutdown.
    ///
    pub fn new(
        event_sender: mpsc::Sender<SystemEvent>,
        token: CancellationToken,
    ) -> Self {
        let root_senders = Arc::new(RwLock::new(Vec::<StopSender>::new()));
        let root_sender_clone = root_senders.clone();
        let token_clone = token.clone();

        tokio::spawn(async move {
            token_clone.cancelled().await;
            debug!("Stopping actor system...");
            let mut root_senders = root_sender_clone.write().await;
            while let Some(sender) = root_senders.pop() {
                let (stop_sender, stop_receiver) = oneshot::channel();
                if sender.send(Some(stop_sender)).await.is_err() {
                    return;
                } else {
                    let _ = stop_receiver.await;
                };
            }

            let _ = event_sender.send(SystemEvent::StopSystem).await;
        });

        SystemRef {
            actors: Arc::new(RwLock::new(HashMap::new())),
            helpers: Arc::new(RwLock::new(HashMap::new())),
            token,
            root_senders,
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
        parent_error_sender: Option<ChildErrorSender>,
    ) -> Result<(ActorRef<A>, StopSender), Error>
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
        let (mut runner, actor_ref, stop_sender) =
            ActorRunner::create(path.clone(), actor, parent_error_sender);

        // Store the actor reference.
        let any = Box::new(actor_ref.clone());
        {
            let mut actors = self.actors.write().await;
            actors.insert(path.clone(), any);
        }
        let (sender, receiver) = oneshot::channel::<bool>();

        let stop_sender_clone = stop_sender.clone();
        tokio::spawn(async move {
            runner.init(system, stop_sender_clone, Some(sender)).await;
        });

        if receiver.await.map_err(|e| Error::Start(e.to_string()))? {
            Ok((actor_ref, stop_sender))
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
        let (actor_ref, stop_sender, ..) =
            self.create_actor_path::<A>(path, actor, None).await?;
        let mut senders = self.root_senders.write().await;
        senders.push(stop_sender);
        Ok(actor_ref)
    }

    /// Remove an actor from this actor system.
    /// If the actor does not exist, nothing happens.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the actor to remove.
    ///
    pub(crate) async fn remove_actor(&self, path: &ActorPath) {
        let mut actors = self.actors.write().await;
        actors.remove(path);
    }

    /// Initiates graceful shutdown of the entire actor system.
    /// This cancels the system's cancellation token, which triggers
    /// the shutdown sequence for all root actors.
    ///
    /// # Behavior
    ///
    /// - Cancels the system token.
    /// - Triggers the shutdown task spawned in `new()`.
    /// - Root actors are stopped in reverse order of creation.
    /// - System runner receives StopSystem event when complete.
    ///
    pub fn stop_system(&self) {
        self.token.cancel();
    }

    /// Retrieves all direct children of the specified actor.
    /// This scans the actor registry for actors whose parent matches
    /// the given path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path of the parent actor.
    ///
    /// # Returns
    ///
    /// Returns a vector of ActorPath for all direct children.
    ///
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

    /// Adds a helper object to the actor system.
    /// Helpers are shared objects (like database pools, configurations, etc.)
    /// that actors can retrieve by name. This enables dependency injection
    /// for actors without tight coupling.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique identifier for this helper.
    /// * `helper` - The helper object to store (must be Clone + Send + Sync).
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct DatabasePool { /* ... */ }
    ///
    /// let db_pool = DatabasePool::new();
    /// system.add_helper("db_pool", db_pool).await;
    /// ```
    ///
    pub async fn add_helper<H>(&self, name: &str, helper: H)
    where
        H: Any + Send + Sync + Clone + 'static,
    {
        let mut helpers = self.helpers.write().await;
        helpers.insert(name.to_owned(), Box::new(helper));
    }

    /// Retrieves a helper object from the actor system.
    /// Actors can use this to access shared resources like database
    /// connections, configuration, or other services.
    ///
    /// # Arguments
    ///
    /// * `name` - The identifier of the helper to retrieve.
    ///
    /// # Returns
    ///
    /// Returns Some(helper) if found and type matches, None otherwise.
    ///
    /// # Example
    ///
    /// ```
    /// let db_pool: Option<DatabasePool> = system.get_helper("db_pool").await;
    /// ```
    ///
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

    /// Spawns a sink to run in a separate task.
    /// Sinks process events emitted by actors and run independently
    /// in their own tasks for concurrent event processing.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink to run (contains subscriber and event receiver).
    ///
    /// # Example
    ///
    /// ```
    /// let sink = Sink::new(actor_ref.subscribe(), MySubscriber);
    /// system.run_sink(sink).await;
    /// ```
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

/// System runner that processes system-wide events.
/// The SystemRunner must be spawned in a task and run to process
/// system lifecycle events like shutdown notifications.
///
pub struct SystemRunner {
    /// Receiver for system-wide events.
    event_receiver: mpsc::Receiver<SystemEvent>,
}

impl SystemRunner {
    /// Creates a new system runner with the given event receiver.
    ///
    /// # Arguments
    ///
    /// * `event_receiver` - Channel receiver for SystemEvent messages.
    ///
    /// # Returns
    ///
    /// Returns a new SystemRunner instance.
    ///
    pub(crate) fn new(event_receiver: mpsc::Receiver<SystemEvent>) -> Self {
        Self { event_receiver }
    }

    /// Runs the system event loop.
    /// This method blocks and processes system events until a StopSystem
    /// event is received. Must be spawned in a separate task.
    ///
    /// # Behavior
    ///
    /// - Waits for system events on the event channel.
    /// - Processes StopSystem event by logging and exiting the loop.
    /// - Returns when the system is stopped.
    ///
    /// # Example
    ///
    /// ```
    /// let (system, mut runner) = ActorSystem::create(token);
    ///
    /// tokio::spawn(async move {
    ///     runner.run().await;
    ///     println!("System runner completed");
    /// });
    /// ```
    ///
    pub async fn run(&mut self) {
        debug!("Running actor system...");
            tokio::select! {
                Some(event) = self.event_receiver.recv() => {
                    match event {
                        SystemEvent::StopSystem => {
                            debug!("Actor system stopped.");
                    }
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
        let token = CancellationToken::new();
        let (_system, mut runner) = ActorSystem::create(token.clone());

        tokio::spawn(async move {
            runner.run().await;
        });
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        assert!(logs_contain("Running actor system..."));
        token.cancel();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        assert!(logs_contain("Stopping actor system..."));
        assert!(logs_contain("Actor system stopped."));
    }

    #[tokio::test]
    async fn test_helpers() {
        let (system, _) = ActorSystem::create(CancellationToken::new());
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
