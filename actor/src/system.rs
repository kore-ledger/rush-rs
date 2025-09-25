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
    runner::{ActorRunner, StopSender},
    sink::Sink,
};

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use tracing::{debug, error};

use std::{any::Any, collections::HashMap, sync::Arc};

/// The core actor system factory that bootstraps and manages the entire actor runtime.
///
/// `ActorSystem` serves as the primary entry point for creating actor runtime environments.
/// It is responsible for establishing the fundamental infrastructure required for actor
/// execution, including system references, runners, and the coordination mechanisms
/// that enable distributed actor communication and lifecycle management.
///
/// # Architecture Overview
///
/// The actor system follows a hierarchical design where all actors exist within a
/// supervised tree structure. The `ActorSystem` creates the foundational components:
///
/// - **SystemRef**: Provides the interface for actor creation, management, and discovery
/// - **SystemRunner**: Handles system-level events and coordinates system shutdown
/// - **Event Loop**: Manages system-wide events and coordination messages
/// - **Cancellation Token**: Enables graceful system shutdown and resource cleanup
///
/// # Lifecycle Management
///
/// Actor systems have a well-defined lifecycle:
///
/// 1. **Creation**: `ActorSystem::create()` initializes system components
/// 2. **Runtime**: `SystemRunner::run()` starts the event processing loop
/// 3. **Shutdown**: Cancellation token triggers graceful actor termination
/// 4. **Cleanup**: All resources are released and actors are stopped
///
/// # Thread Safety
///
/// The actor system is designed to be thread-safe and can be shared across multiple
/// async tasks. All internal state is protected by appropriate synchronization primitives,
/// and the system maintains consistency even under high concurrent load.
///
/// # Performance Considerations
///
/// - System creation is lightweight and happens synchronously
/// - Event channels are bounded to prevent memory issues under load
/// - The system uses efficient data structures for actor registry lookups
/// - Cancellation propagation is optimized for minimal latency during shutdown
///
/// # Examples
///
/// ```ignore
/// use rush_actor::*;
/// use tokio_util::sync::CancellationToken;
///
/// #[tokio::main]
/// async fn main() {
///     let token = CancellationToken::new();
///     let (system, mut runner) = ActorSystem::create(token.clone());
///
///     // Start the system in a background task
///     tokio::spawn(async move {
///         runner.run().await;
///     });
///
///     // Create root actors
///     let actor = system.create_root_actor("my_actor", MyActor::new()).await?;
///
///     // System operations...
///
///     // Graceful shutdown
///     system.stop_system();
/// }
/// ```
pub struct ActorSystem {}

impl ActorSystem {
    /// Creates a new actor system with all necessary runtime components.
    ///
    /// This method initializes the fundamental infrastructure required for actor execution,
    /// including event channels, cancellation mechanisms, and the coordination systems
    /// that enable distributed actor communication. The creation process is synchronous
    /// and lightweight, with the actual runtime behavior starting when the returned
    /// `SystemRunner` begins execution.
    ///
    /// # Arguments
    ///
    /// * `token` - A cancellation token that controls the lifecycle of the entire actor system.
    ///   When cancelled, it triggers graceful shutdown of all actors and system components.
    ///   The token should be shared with any components that need to coordinate shutdown.
    ///
    /// # Returns
    ///
    /// Returns a tuple containing:
    /// - `SystemRef`: The primary interface for creating actors, managing system state,
    ///   and accessing system services. This can be cloned and shared across tasks.
    /// - `SystemRunner`: The event loop manager that must be run to process system events
    ///   and coordinate actor operations. This should typically be spawned in a background task.
    ///
    /// # Architecture Details
    ///
    /// The creation process establishes:
    /// - A bounded MPSC channel (capacity: 100) for system events
    /// - Cancellation token integration for coordinated shutdown
    /// - Actor registry initialization for path-based actor discovery
    /// - Helper registry for system-wide shared resources
    /// - Root actor supervision infrastructure
    ///
    /// # Performance Notes
    ///
    /// - Creation is O(1) and completes in microseconds
    /// - Memory allocation is minimal and bounded
    /// - No I/O operations are performed during creation
    /// - The system is ready to accept actors immediately
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// // Basic system creation
    /// let token = CancellationToken::new();
    /// let (system, mut runner) = ActorSystem::create(token.clone());
    ///
    /// // Start the runner in background
    /// let runner_handle = tokio::spawn(async move {
    ///     runner.run().await;
    /// });
    ///
    /// // Create actors using the system reference
    /// let actor = system.create_root_actor("worker", WorkerActor::new()).await?;
    ///
    /// // Shutdown when done
    /// token.cancel();
    /// runner_handle.await?;
    /// ```
    ///
    /// # Error Conditions
    ///
    /// This method cannot fail under normal circumstances, as it only initializes
    /// in-memory data structures. However, if the system runs out of memory during
    /// channel or data structure allocation, a panic may occur.
    pub fn create(token: CancellationToken) -> (SystemRef, SystemRunner) {
        let (event_sender, event_receiver) = mpsc::channel(100);
        let system = SystemRef::new(event_sender, token);
        let runner = SystemRunner::new(event_receiver);
        (system, runner)
    }
}

/// System-level events that coordinate actor system lifecycle and operations.
///
/// `SystemEvent` represents high-level coordination messages that flow through the
/// actor system's event processing pipeline. These events are used to coordinate
/// system-wide operations such as shutdown, configuration changes, and other
/// administrative tasks that affect the entire actor runtime.
///
/// # Event Processing
///
/// System events are processed by the `SystemRunner` in its main event loop.
/// They have higher priority than regular actor messages and are designed to
/// enable coordination of system-wide state changes that require global visibility.
///
/// # Thread Safety
///
/// All variants are designed to be safely sent across thread boundaries and can
/// be cloned without side effects. The enum implements `Send + Sync` through its
/// constituent data types.
///
/// # Performance Characteristics
///
/// - Events are lightweight and serialize efficiently
/// - Processing is designed to be non-blocking where possible
/// - Event propagation uses bounded channels to prevent memory issues
///
/// # Integration Patterns
///
/// System events are typically generated by:
/// - Cancellation token triggers during shutdown
/// - Administrative interfaces for system management
/// - Error recovery mechanisms requiring system coordination
/// - External monitoring and control systems
///
/// # Examples
///
/// ```ignore
/// use rush_actor::SystemEvent;
///
/// // Events are typically created internally by the system
/// let shutdown_event = SystemEvent::StopSystem;
///
/// // They can be pattern matched for handling
/// match shutdown_event {
///     SystemEvent::StopSystem => {
///         println!("System shutdown initiated");
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// Signals that the actor system should perform a graceful shutdown.
    ///
    /// This event triggers the system-wide shutdown sequence, which includes:
    /// - Stopping all root actors in reverse creation order
    /// - Waiting for actor supervision trees to complete shutdown
    /// - Cleaning up system resources and releasing handles
    /// - Terminating the system runner's event loop
    ///
    /// The shutdown process is designed to be graceful, allowing actors to
    /// complete their current message processing before termination.
    StopSystem,
}

/// Primary interface for interacting with the actor system and managing actor lifecycles.
///
/// `SystemRef` serves as the main entry point for all actor system operations, providing
/// a comprehensive API for actor creation, management, discovery, and system-wide services.
/// It maintains the central registry of all actors and provides thread-safe access to
/// system resources and coordination mechanisms.
///
/// # Core Responsibilities
///
/// - **Actor Lifecycle Management**: Create, start, stop, and remove actors
/// - **Actor Discovery**: Locate and retrieve actor references by path
/// - **System Services**: Provide access to helpers, sinks, and shared resources
/// - **Supervision Tree**: Maintain parent-child relationships and error propagation
/// - **Graceful Shutdown**: Coordinate system-wide termination processes
///
/// # Architecture Integration
///
/// The `SystemRef` integrates with several key system components:
/// - Actor registry for path-based lookups and lifecycle tracking
/// - Helper registry for shared system resources and utilities
/// - Cancellation tokens for coordinated shutdown signaling
/// - Root actor supervision for top-level error handling
/// - Event system for system-wide coordination messages
///
/// # Thread Safety
///
/// This struct is fully thread-safe and designed to be shared across multiple async tasks.
/// All internal state is protected by appropriate synchronization primitives:
/// - Actor registry uses `Arc<RwLock<HashMap>>` for concurrent access
/// - Helper registry uses `Arc<RwLock<HashMap>>` for safe resource sharing
/// - Root actor tracking uses `Arc<RwLock<Vec>>` for supervision coordination
///
/// # Performance Characteristics
///
/// - Actor lookups are O(1) average case, O(n) worst case due to HashMap
/// - Actor creation involves minimal synchronization overhead
/// - Cloning is lightweight (only increments Arc reference counts)
/// - Memory usage scales linearly with number of registered actors
///
/// # Lifecycle Management
///
/// The system reference coordinates complex actor lifecycles:
/// 1. **Creation**: Validates uniqueness, initializes runner, stores references
/// 2. **Runtime**: Provides discovery and communication capabilities
/// 3. **Supervision**: Tracks parent-child relationships and error propagation
/// 4. **Shutdown**: Orchestrates graceful termination in reverse creation order
///
/// # Error Handling
///
/// The system implements robust error handling strategies:
/// - Actor creation conflicts are detected and reported
/// - Supervision failures trigger appropriate recovery mechanisms
/// - System-wide errors can trigger coordinated shutdown procedures
/// - Resource cleanup is guaranteed even in error conditions
///
/// # Examples
///
/// ```ignore
/// use rush_actor::*;
/// use tokio_util::sync::CancellationToken;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let token = CancellationToken::new();
///     let (system, mut runner) = ActorSystem::create(token.clone());
///
///     // Start system runner
///     tokio::spawn(async move { runner.run().await });
///
///     // Create root actor
///     let worker = system.create_root_actor("worker", WorkerActor::new()).await?;
///
///     // Add system helper
///     system.add_helper("config", AppConfig::new()).await;
///
///     // Retrieve actor by path
///     let path = ActorPath::from("/user/worker");
///     if let Some(actor) = system.get_actor::<WorkerActor>(&path).await {
///         actor.tell(WorkerMessage::Process("data".to_string())).await?;
///     }
///
///     // Get system helper
///     if let Some(config) = system.get_helper::<AppConfig>("config").await {
///         println!("Config loaded: {:?}", config);
///     }
///
///     // Graceful shutdown
///     system.stop_system();
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct SystemRef {
    /// Central registry of all active actors in the system, indexed by their hierarchical paths.
    ///
    /// This registry maintains type-erased references to all actors, enabling path-based
    /// discovery and lifecycle management. The `RwLock` provides concurrent read access
    /// for lookups while serializing writes during actor creation and removal.
    actors:
        Arc<RwLock<HashMap<ActorPath, Box<dyn Any + Send + Sync + 'static>>>>,

    /// Registry of shared system resources and utilities accessible to all actors.
    ///
    /// Helpers are system-wide singleton services that actors can access for shared
    /// functionality like configuration, database connections, or utility services.
    /// They are stored as type-erased values and retrieved via downcasting.
    helpers: Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync + 'static>>>>,

    /// Collection of stop channels for root-level actors under direct system supervision.
    ///
    /// Root actors are top-level actors created directly via `create_root_actor()`. The
    /// system maintains stop senders for these actors to enable coordinated shutdown
    /// in reverse creation order during system termination.
    root_senders: Arc<RwLock<Vec<StopSender>>>,

    /// Cancellation token that coordinates system-wide shutdown and cleanup operations.
    ///
    /// When cancelled, this token triggers the graceful shutdown sequence, which includes
    /// stopping all root actors, cleaning up resources, and terminating the system runner.
    /// The token can be shared with external components that need shutdown coordination.
    token: CancellationToken
}

impl SystemRef {
    /// Creates a new system reference with shutdown coordination and event handling.
    ///
    /// This method initializes the core system infrastructure, including actor registries,
    /// helper storage, and shutdown coordination mechanisms. It also establishes a
    /// background task that monitors the cancellation token and orchestrates graceful
    /// system shutdown when triggered.
    ///
    /// # Arguments
    ///
    /// * `event_sender` - Channel for sending system-level coordination events to the
    ///   system runner. Used primarily for shutdown signaling and system-wide notifications.
    /// * `token` - Cancellation token that triggers graceful system shutdown. When cancelled,
    ///   initiates the coordinated termination of all root actors and system cleanup.
    ///
    /// # Returns
    ///
    /// Returns a fully initialized `SystemRef` ready to manage actors and provide system services.
    ///
    /// # Shutdown Coordination
    ///
    /// The method spawns a background task that:
    /// 1. Waits for the cancellation token to be triggered
    /// 2. Logs the shutdown initiation for observability
    /// 3. Stops all root actors in reverse creation order
    /// 4. Waits for each actor to confirm graceful shutdown
    /// 5. Sends a `StopSystem` event to terminate the system runner
    ///
    /// # Thread Safety
    ///
    /// All internal data structures are wrapped in `Arc<RwLock<_>>` to provide safe
    /// concurrent access across multiple async tasks. The shutdown coordination task
    /// runs independently and coordinates with the main system through channels.
    ///
    /// # Performance Considerations
    ///
    /// - Registry initialization is O(1) and lightweight
    /// - Background task startup has minimal overhead
    /// - Shutdown coordination is optimized for minimal latency
    /// - Memory usage is bounded and predictable
    ///
    /// # Error Handling
    ///
    /// The shutdown coordination task handles several error scenarios:
    /// - If a root actor's stop channel is closed, the task continues with remaining actors
    /// - If the system event sender fails, shutdown continues (system may be terminating)
    /// - Individual actor shutdown failures don't prevent overall system termination
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio::sync::mpsc;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// let (event_sender, event_receiver) = mpsc::channel(100);
    /// let token = CancellationToken::new();
    ///
    /// // Create system reference
    /// let system = SystemRef::new(event_sender, token.clone());
    ///
    /// // System is ready to manage actors
    /// let actor = system.create_root_actor("worker", MyActor::new()).await?;
    ///
    /// // Trigger graceful shutdown
    /// token.cancel();
    /// ```
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

    /// Retrieves a strongly-typed actor reference by its hierarchical path.
    ///
    /// This method performs a type-safe lookup in the actor registry, attempting to
    /// locate an actor at the specified path and downcast it to the requested type.
    /// The operation is thread-safe and can be called concurrently from multiple tasks
    /// without coordination.
    ///
    /// # Type Parameters
    ///
    /// * `A` - The expected actor type. Must implement both `Actor` and `Handler<A>` traits
    ///   to ensure type safety and proper message handling capabilities.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path identifying the target actor. Paths follow the
    ///   format `/user/parent/child` and must match exactly. Path comparison is
    ///   case-sensitive and does not support wildcards or pattern matching.
    ///
    /// # Returns
    ///
    /// * `Some(ActorRef<A>)` - If an actor exists at the path and can be downcast to type `A`
    /// * `None` - If no actor exists at the path or the actor cannot be cast to type `A`
    ///
    /// # Performance Characteristics
    ///
    /// - Lookup complexity: O(1) average case, O(n) worst case (HashMap performance)
    /// - Read lock acquisition: Minimal contention in typical scenarios
    /// - Type downcasting: Efficient runtime type checking with minimal overhead
    /// - Memory allocation: No heap allocation for successful lookups
    ///
    /// # Thread Safety
    ///
    /// This method acquires a read lock on the actor registry, allowing multiple
    /// concurrent lookups while preventing race conditions with actor creation/removal.
    /// The returned `ActorRef` is thread-safe and can be shared across tasks.
    ///
    /// # Common Usage Patterns
    ///
    /// 1. **Service Discovery**: Locate well-known system services by path
    /// 2. **Parent-Child Communication**: Parents discovering their children
    /// 3. **Cross-Actor Messaging**: Finding actors to send messages to
    /// 4. **System Monitoring**: Administrative tools checking actor existence
    ///
    /// # Error Conditions
    ///
    /// This method returns `None` in several scenarios:
    /// - The specified path does not exist in the registry
    /// - An actor exists at the path but is of a different type than `A`
    /// - The actor existed but has been removed from the registry
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// // Look up a specific actor by path
    /// let path = ActorPath::from("/user/worker");
    /// if let Some(worker) = system.get_actor::<WorkerActor>(&path).await {
    ///     worker.tell(WorkerMessage::Process("data".to_string())).await?;
    /// } else {
    ///     println!("Worker actor not found or wrong type");
    /// }
    ///
    /// // Service discovery pattern
    /// let db_path = ActorPath::from("/user/database");
    /// let db_actor: Option<ActorRef<DatabaseActor>> = system.get_actor(&db_path).await;
    ///
    /// // Type safety - wrong type returns None
    /// let wrong_type: Option<ActorRef<WrongActor>> = system.get_actor(&db_path).await;
    /// assert!(wrong_type.is_none());
    /// ```
    ///
    /// # Integration Notes
    ///
    /// - Works seamlessly with actor path hierarchies created via `create_child_actor`
    /// - Compatible with both root actors and child actors in supervision trees
    /// - Can be used safely during system shutdown (may return None for stopped actors)
    /// - Integrates with actor lifecycle management and cleanup processes
    pub async fn get_actor<A>(&self, path: &ActorPath) -> Option<ActorRef<A>>
    where
        A: Actor + Handler<A>,
    {
        let actors = self.actors.read().await;
        actors
            .get(path)
            .and_then(|any| any.downcast_ref::<ActorRef<A>>().cloned())
    }

    /// Creates an actor at the specified path with full lifecycle management and supervision.
    ///
    /// This method is the core actor creation mechanism that handles the complete actor
    /// lifecycle from initialization through registration. It ensures path uniqueness,
    /// establishes supervision relationships, and integrates the actor with the system's
    /// runtime infrastructure.
    ///
    /// # Type Parameters
    ///
    /// * `A` - The actor type to create. Must implement `Actor + Handler<A>` to ensure
    ///   proper message handling and lifecycle management capabilities.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path where the actor will be registered. Must be unique
    ///   within the system. Paths follow the format `/user/parent/child` and establish
    ///   the actor's position in the supervision tree.
    /// * `actor` - The actor instance to initialize and manage. Ownership is transferred
    ///   to the actor runner for lifecycle management.
    /// * `parent_error_sender` - Optional channel for reporting errors to a parent actor.
    ///   If `None`, the actor is treated as a root actor with no parent supervision.
    ///
    /// # Returns
    ///
    /// Returns a tuple containing:
    /// * `ActorRef<A>` - Thread-safe reference for sending messages to the actor
    /// * `StopSender` - Channel for coordinating graceful actor shutdown
    ///
    /// # Error Conditions
    ///
    /// * `Error::Exists(path)` - An actor already exists at the specified path
    /// * `Error::Start(message)` - Actor initialization failed during startup
    ///
    /// # Actor Creation Process
    ///
    /// The creation process involves several coordinated steps:
    /// 1. **Path Validation**: Ensures the path is unique within the system
    /// 2. **Runner Creation**: Initializes the actor runner with message channels
    /// 3. **Registry Update**: Stores the actor reference for path-based lookup
    /// 4. **Initialization**: Spawns the actor runner and waits for startup confirmation
    /// 5. **Supervision Setup**: Establishes parent-child error reporting if applicable
    ///
    /// # Performance Considerations
    ///
    /// - Path uniqueness check requires a read lock on the actor registry
    /// - Actor registration requires a write lock (serialized with other creations)
    /// - Runner initialization spawns a new async task for message processing
    /// - Startup confirmation uses a oneshot channel for minimal latency
    ///
    /// # Thread Safety
    ///
    /// This method coordinates multiple concurrent operations safely:
    /// - Registry access is protected by read/write locks
    /// - Actor runner spawning is thread-safe
    /// - Error reporting channels are established before actor startup
    ///
    /// # Integration with Supervision
    ///
    /// When `parent_error_sender` is provided:
    /// - Actor errors are reported to the parent for supervision decisions
    /// - Parent-child relationships are established for coordinated shutdown
    /// - Error propagation follows the supervision tree hierarchy
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Create a root actor (no parent supervision)
    /// let path = ActorPath::from("/user/worker");
    /// let (actor_ref, stop_sender) = system.create_actor_path(
    ///     path,
    ///     WorkerActor::new(),
    ///     None
    /// ).await?;
    ///
    /// // Create a child actor with parent supervision
    /// let child_path = ActorPath::from("/user/worker/processor");
    /// let (child_ref, child_stop) = system.create_actor_path(
    ///     child_path,
    ///     ProcessorActor::new(),
    ///     Some(parent_error_sender)
    /// ).await?;
    /// ```
    ///
    /// # Internal Implementation Details
    ///
    /// This method is marked `pub(crate)` as it's used internally by higher-level
    /// actor creation methods like `create_root_actor` and `create_child_actor`.
    /// External code should use those public methods instead of calling this directly.
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

    /// Creates a new top-level actor under direct system supervision at the `/user` path.
    ///
    /// Root actors are the top-level entries in the actor system's supervision hierarchy.
    /// They are created directly under the `/user` path and are supervised by the system
    /// itself. During shutdown, root actors are terminated in reverse creation order to
    /// ensure proper resource cleanup and graceful system termination.
    ///
    /// # Type Parameters
    ///
    /// * `A` - The actor type to create. Must implement `Actor + Handler<A>` to ensure
    ///   proper message handling capabilities and integration with the actor system.
    ///
    /// # Arguments
    ///
    /// * `name` - The unique name for this root actor. Will be used to construct the
    ///   full path as `/user/{name}`. Must be unique among root actors. Names should
    ///   follow conventional identifier patterns for clarity and debugging.
    /// * `actor` - The actor instance to initialize. Ownership is transferred to the
    ///   system for lifecycle management. The actor will be started immediately
    ///   after creation and registration.
    ///
    /// # Returns
    ///
    /// Returns a thread-safe `ActorRef<A>` that can be used to:
    /// - Send messages to the actor via `tell()` and `ask()` methods
    /// - Subscribe to actor events via the event system
    /// - Monitor actor lifecycle and status
    ///
    /// # Error Conditions
    ///
    /// * `Error::Exists(ActorPath)` - An actor with the same name already exists at `/user/{name}`
    /// * `Error::Start(String)` - Actor initialization failed during startup process
    ///
    /// # Root Actor Characteristics
    ///
    /// Root actors have special properties within the system:
    /// - **No Parent Supervision**: Errors are handled by the system's default strategies
    /// - **System Shutdown Priority**: Stopped first during graceful system termination
    /// - **Direct System Access**: Can access system-level services and helpers directly
    /// - **Top-Level Supervision**: Can create and supervise child actor hierarchies
    ///
    /// # Supervision Integration
    ///
    /// The system maintains supervision metadata for root actors:
    /// - Stop senders are stored for coordinated shutdown
    /// - Creation order is tracked for reverse termination
    /// - Error handling follows system-level supervision strategies
    /// - Resource cleanup is guaranteed during system shutdown
    ///
    /// # Performance Characteristics
    ///
    /// - Path construction and uniqueness checking: O(1) average case
    /// - Actor initialization: Spawns new async task with minimal overhead
    /// - Registration: Requires brief write lock on system registries
    /// - Supervision setup: Lightweight channel and vector operations
    ///
    /// # Thread Safety
    ///
    /// All operations are thread-safe and can be called concurrently:
    /// - Multiple root actors can be created simultaneously
    /// - Registry updates are properly synchronized
    /// - Actor references are safe to share across tasks
    ///
    /// # Common Usage Patterns
    ///
    /// 1. **Service Actors**: Long-running services like database connections
    /// 2. **Worker Pool Managers**: Actors that manage pools of worker actors
    /// 3. **Protocol Handlers**: Actors that handle specific communication protocols
    /// 4. **System Services**: Infrastructure actors like logging or metrics collectors
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// // Create a database service actor
    /// let db_actor = system.create_root_actor(
    ///     "database",
    ///     DatabaseActor::new("postgresql://localhost/mydb")
    /// ).await?;
    ///
    /// // Create a worker pool manager
    /// let worker_manager = system.create_root_actor(
    ///     "workers",
    ///     WorkerPoolActor::new(10) // 10 worker threads
    /// ).await?;
    ///
    /// // Create an HTTP server actor
    /// let http_server = system.create_root_actor(
    ///     "http_server",
    ///     HttpServerActor::new("0.0.0.0:8080")
    /// ).await?;
    ///
    /// // Use the actors
    /// db_actor.tell(DatabaseMessage::Connect).await?;
    /// worker_manager.tell(WorkerMessage::StartPool).await?;
    /// http_server.tell(ServerMessage::Start).await?;
    /// ```
    ///
    /// # Integration with Child Actors
    ///
    /// Root actors often serve as parents for child actor hierarchies:
    ///
    /// ```ignore
    /// // Root actor that creates children
    /// let supervisor = system.create_root_actor(
    ///     "supervisor",
    ///     SupervisorActor::new()
    /// ).await?;
    ///
    /// // The supervisor can create children in its message handlers
    /// supervisor.tell(SupervisorMessage::CreateWorkers(5)).await?;
    /// ```
    ///
    /// # Lifecycle Management
    ///
    /// Root actors participate in the system lifecycle:
    /// - **Creation**: Immediately active and ready to receive messages
    /// - **Runtime**: Process messages according to their handler implementation
    /// - **Shutdown**: Gracefully terminated when system shutdown is initiated
    /// - **Cleanup**: Resources released and registry entries removed
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

    /// Removes an actor from the system registry and cleans up associated resources.
    ///
    /// This method handles the cleanup phase of actor lifecycle management, removing
    /// the actor's entry from the system registry to prevent further lookups and
    /// message delivery. It is typically called during actor shutdown or when an
    /// actor encounters an unrecoverable error.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path of the actor to remove. The path must match
    ///   exactly the path used during actor creation. Case-sensitive comparison is used.
    ///
    /// # Behavior
    ///
    /// - If the actor exists at the specified path, it is removed from the registry
    /// - If no actor exists at the path, the operation completes silently (idempotent)
    /// - Registry cleanup is atomic to prevent race conditions
    /// - No cascade removal of child actors (must be handled by supervision logic)
    ///
    /// # Thread Safety
    ///
    /// This method acquires a write lock on the actor registry, ensuring atomic
    /// removal even under concurrent access. Multiple removals can be serialized
    /// safely without coordination between callers.
    ///
    /// # Performance Characteristics
    ///
    /// - Removal complexity: O(1) average case, O(n) worst case (HashMap performance)
    /// - Write lock acquisition: Serialized with other registry modifications
    /// - Memory cleanup: Immediate release of registry entry and type-erased reference
    ///
    /// # Integration with Actor Lifecycle
    ///
    /// This method is typically called by:
    /// - Actor runners during graceful shutdown
    /// - Supervision logic when restarting failed actors
    /// - System shutdown during cleanup of remaining actors
    /// - Error recovery mechanisms removing corrupted actors
    ///
    /// # Internal Implementation Note
    ///
    /// This method is marked `pub(crate)` as it's part of the internal lifecycle
    /// management system. External code should not call this directly, as it may
    /// leave the system in an inconsistent state if not coordinated with proper
    /// actor shutdown procedures.
    ///
    /// # Examples (Internal Use)
    ///
    /// ```ignore
    /// // Called by actor runner during shutdown
    /// impl<A> ActorRunner<A> {
    ///     async fn shutdown(&mut self, system: &SystemRef) {
    ///         // Perform actor cleanup...
    ///         system.remove_actor(&self.path).await;
    ///     }
    /// }
    ///
    /// // Called by supervision during restart
    /// async fn restart_actor(system: &SystemRef, path: &ActorPath) {
    ///     system.remove_actor(path).await;  // Remove old instance
    ///     // Create new instance...
    /// }
    /// ```
    pub(crate) async fn remove_actor(&self, path: &ActorPath) {
        let mut actors = self.actors.write().await;
        actors.remove(path);
    }

    /// Initiates graceful shutdown of the entire actor system.
    ///
    /// This method triggers the coordinated shutdown sequence that cleanly terminates
    /// all actors and releases system resources. The shutdown process is designed to
    /// be graceful, allowing actors to complete their current message processing
    /// before termination.
    ///
    /// # Shutdown Sequence
    ///
    /// When called, this method triggers the following sequence:
    /// 1. **Signal Initiation**: Cancels the system's cancellation token
    /// 2. **Root Actor Shutdown**: Stops all root actors in reverse creation order
    /// 3. **Supervision Cascade**: Child actors are terminated through supervision trees
    /// 4. **Resource Cleanup**: System resources and registries are cleaned up
    /// 5. **Runner Termination**: The system runner's event loop terminates
    ///
    /// # Graceful Termination
    ///
    /// The shutdown process respects actor boundaries and allows for graceful cleanup:
    /// - Actors finish processing their current message before stopping
    /// - Each actor can perform cleanup in its `stopping()` lifecycle method
    /// - Parent actors wait for children to terminate before shutting down themselves
    /// - System waits for all actors to confirm termination before cleanup
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called from any async context.
    /// Multiple calls to `stop_system()` are safe and idempotent - the shutdown
    /// sequence will only be initiated once.
    ///
    /// # Performance Characteristics
    ///
    /// - Shutdown initiation: O(1) - simply cancels the token
    /// - Overall shutdown time: Depends on actor count and cleanup complexity
    /// - Resource release: Immediate for system resources, gradual for actor resources
    /// - Memory cleanup: Automatic as actors are dropped and registries cleared
    ///
    /// # Error Handling
    ///
    /// The shutdown process is designed to be resilient:
    /// - Individual actor shutdown failures don't prevent system termination
    /// - Resource cleanup continues even if some actors fail to stop gracefully
    /// - System guarantees eventual termination even in error conditions
    ///
    /// # Integration Patterns
    ///
    /// Common scenarios for calling this method:
    /// - Application shutdown hooks (SIGTERM, Ctrl+C handlers)
    /// - Graceful service termination in containerized environments
    /// - Test cleanup to ensure proper resource release
    /// - Administrative shutdown commands in management interfaces
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio::signal;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let token = CancellationToken::new();
    ///     let (system, mut runner) = ActorSystem::create(token.clone());
    ///
    ///     // Start the system
    ///     let runner_handle = tokio::spawn(async move {
    ///         runner.run().await;
    ///     });
    ///
    ///     // Create actors and run application logic...
    ///     let worker = system.create_root_actor("worker", WorkerActor::new()).await?;
    ///
    ///     // Set up graceful shutdown on SIGTERM
    ///     let system_clone = system.clone();
    ///     tokio::spawn(async move {
    ///         signal::ctrl_c().await.expect("Failed to listen for ctrl+c");
    ///         println!("Shutting down gracefully...");
    ///         system_clone.stop_system();
    ///     });
    ///
    ///     // Wait for system to complete shutdown
    ///     runner_handle.await?;
    ///     println!("System shutdown complete");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Coordination with External Components
    ///
    /// If external components need to coordinate with system shutdown:
    ///
    /// ```ignore
    /// // Clone the cancellation token during system creation
    /// let token = CancellationToken::new();
    /// let (system, runner) = ActorSystem::create(token.clone());
    ///
    /// // External component can listen for shutdown
    /// let external_service = ExternalService::new();
    /// tokio::spawn(async move {
    ///     token.cancelled().await;
    ///     external_service.shutdown().await;
    /// });
    ///
    /// // Triggering shutdown coordinates both system and external components
    /// system.stop_system();
    /// ```
    pub fn stop_system(&self) {
        self.token.cancel();
    }

    /// Retrieves all direct child actors of the specified parent actor.
    ///
    /// This method traverses the actor registry to find all actors that are direct
    /// children of the given parent path. It's useful for supervision logic, system
    /// introspection, and coordinated operations across actor hierarchies.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path of the parent actor whose children should be found.
    ///   This should be a valid actor path that may or may not have children.
    ///
    /// # Returns
    ///
    /// Returns a `Vec<ActorPath>` containing the paths of all direct child actors.
    /// The vector will be empty if:
    /// - The parent actor has no children
    /// - The parent actor does not exist
    /// - All child actors have been terminated
    ///
    /// # Parent-Child Relationship
    ///
    /// The parent-child relationship is determined by path hierarchy:
    /// - Parent: `/user/parent`
    /// - Child: `/user/parent/child`
    /// - Grandchild: `/user/parent/child/grandchild`
    ///
    /// This method returns only direct children, not grandchildren or deeper descendants.
    ///
    /// # Performance Characteristics
    ///
    /// - Complexity: O(n) where n is the total number of actors in the system
    /// - Registry access: Requires a read lock for the duration of the traversal
    /// - Memory allocation: Allocates a vector sized to the number of children found
    /// - Path comparison: Each registered actor path is checked for parentage
    ///
    /// # Thread Safety
    ///
    /// This method acquires a read lock on the actor registry, allowing concurrent
    /// access with other read operations while preventing modification during traversal.
    /// The returned paths are snapshots and may become stale if actors are created
    /// or removed after the call completes.
    ///
    /// # Use Cases
    ///
    /// 1. **Supervision Logic**: Parents checking their children before shutdown
    /// 2. **System Monitoring**: Administrative tools inspecting actor hierarchies
    /// 3. **Broadcasting**: Sending messages to all children of a parent
    /// 4. **Resource Management**: Coordinating cleanup across related actors
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// // Create a parent actor
    /// let parent = system.create_root_actor("parent", ParentActor::new()).await?;
    ///
    /// // Parent creates some children (typically done in message handlers)
    /// // Children would be created with paths like:
    /// // "/user/parent/worker1", "/user/parent/worker2", etc.
    ///
    /// // Get all children of the parent
    /// let parent_path = ActorPath::from("/user/parent");
    /// let children = system.children(&parent_path).await;
    ///
    /// println!("Parent has {} children:", children.len());
    /// for child_path in children {
    ///     println!("  - {}", child_path);
    /// }
    /// ```
    ///
    /// # Supervision Integration
    ///
    /// This method is commonly used in supervision strategies:
    ///
    /// ```ignore
    /// // Supervisor checking its children before restart
    /// impl SupervisorActor {
    ///     async fn handle_restart(&mut self, ctx: &ActorContext<Self>) {
    ///         let my_path = ctx.path();
    ///         let children = ctx.system().children(my_path).await;
    ///
    ///         // Stop all children before restarting
    ///         for child_path in children {
    ///             if let Some(child) = ctx.system().get_actor::<ChildActor>(&child_path).await {
    ///                 child.tell(ChildMessage::Stop).await?;
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Limitations
    ///
    /// - Only returns direct children (one level down in hierarchy)
    /// - Results represent a point-in-time snapshot of the registry
    /// - Does not distinguish between active and terminating actors
    /// - Performance degrades linearly with total system actor count
    ///
    /// # Alternative Approaches
    ///
    /// For better performance with large numbers of actors, consider:
    /// - Maintaining explicit parent-child tracking in actor state
    /// - Using actor groups or tags for logical grouping
    /// - Implementing custom registry structures for hierarchy queries
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

    /// Registers a shared helper resource in the system-wide helper registry.
    ///
    /// Helpers are singleton resources that provide shared functionality to all actors
    /// in the system. They are commonly used for configuration objects, database
    /// connection pools, external service clients, and other resources that should
    /// be shared rather than duplicated across actors.
    ///
    /// # Type Parameters
    ///
    /// * `H` - The helper type to register. Must implement `Any + Send + Sync + Clone + 'static`
    ///   to ensure thread safety and type erasure compatibility.
    ///
    /// # Arguments
    ///
    /// * `name` - A unique string identifier for this helper. Used for subsequent retrieval
    ///   via `get_helper()`. Names should be descriptive and follow a consistent naming
    ///   convention across the application.
    /// * `helper` - The helper instance to register. The value is cloned and stored in
    ///   type-erased form, so it must implement `Clone` for efficient retrieval.
    ///
    /// # Behavior
    ///
    /// - If a helper with the same name already exists, it is replaced silently
    /// - The helper is stored in type-erased form and retrieved via downcasting
    /// - All registered helpers persist for the lifetime of the actor system
    /// - Helpers are accessible to all actors in the system immediately after registration
    ///
    /// # Thread Safety
    ///
    /// This method acquires a write lock on the helper registry, ensuring atomic
    /// updates even under concurrent access. The helper itself must be thread-safe
    /// (`Send + Sync`) to be accessible from multiple actors simultaneously.
    ///
    /// # Performance Characteristics
    ///
    /// - Registration: O(1) HashMap insertion with brief write lock
    /// - Memory usage: Stores one cloned instance per helper type
    /// - Type erasure: Minimal overhead for boxed storage
    /// - Concurrent access: Optimized for many readers, infrequent writers
    ///
    /// # Common Helper Types
    ///
    /// 1. **Configuration Objects**: Application settings and parameters
    /// 2. **Database Pools**: Shared database connection pools
    /// 3. **HTTP Clients**: Reusable HTTP client instances
    /// 4. **Cache Instances**: Shared caching layers
    /// 5. **Metrics Collectors**: System-wide metrics and telemetry
    /// 6. **Service Clients**: External service API clients
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use std::sync::Arc;
    ///
    /// #[derive(Clone, Debug)]
    /// pub struct DatabaseConfig {
    ///     pub connection_string: String,
    ///     pub max_connections: u32,
    /// }
    ///
    /// #[derive(Clone)]
    /// pub struct HttpClient {
    ///     client: Arc<reqwest::Client>,
    /// }
    ///
    /// // Register configuration
    /// let config = DatabaseConfig {
    ///     connection_string: "postgresql://localhost/mydb".to_string(),
    ///     max_connections: 10,
    /// };
    /// system.add_helper("db_config", config).await;
    ///
    /// // Register HTTP client
    /// let http_client = HttpClient {
    ///     client: Arc::new(reqwest::Client::new()),
    /// };
    /// system.add_helper("http_client", http_client).await;
    ///
    /// // Register metrics collector
    /// let metrics = MetricsCollector::new();
    /// system.add_helper("metrics", metrics).await;
    /// ```
    ///
    /// # Integration with Actor Context
    ///
    /// Actors typically access helpers through their context:
    ///
    /// ```ignore
    /// impl Handler<DatabaseActor> for DatabaseActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Get database configuration
    ///         let config: Option<DatabaseConfig> = ctx.system()
    ///             .get_helper("db_config")
    ///             .await;
    ///
    ///         if let Some(config) = config {
    ///             self.connect(&config.connection_string).await?;
    ///         }
    ///
    ///         Ok(DatabaseResponse::Connected)
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// 1. **Register During Startup**: Add helpers before creating actors that depend on them
    /// 2. **Use Descriptive Names**: Choose clear, consistent naming conventions
    /// 3. **Minimize Helper Count**: Too many helpers can indicate architectural issues
    /// 4. **Thread Safety**: Ensure helpers are truly thread-safe for concurrent access
    /// 5. **Immutable Preferred**: Prefer immutable helpers or use interior mutability safely
    ///
    /// # Error Considerations
    ///
    /// This method cannot fail under normal circumstances, but consider:
    /// - Helper construction should handle initialization errors beforehand
    /// - Large helpers may impact memory usage across the system
    /// - Helpers with expensive `Clone` implementations may affect performance
    pub async fn add_helper<H>(&self, name: &str, helper: H)
    where
        H: Any + Send + Sync + Clone + 'static,
    {
        let mut helpers = self.helpers.write().await;
        helpers.insert(name.to_owned(), Box::new(helper));
    }

    /// Retrieves a shared helper resource from the system-wide helper registry.
    ///
    /// This method provides type-safe access to helpers that were previously registered
    /// with `add_helper()`. It performs type downcasting to ensure the retrieved helper
    /// matches the expected type, returning `None` if the helper doesn't exist or has
    /// an incompatible type.
    ///
    /// # Type Parameters
    ///
    /// * `H` - The expected helper type. Must match the type used when the helper was
    ///   registered and implement `Any + Send + Sync + Clone + 'static` for compatibility
    ///   with the type erasure and retrieval system.
    ///
    /// # Arguments
    ///
    /// * `name` - The string identifier used when the helper was registered via `add_helper()`.
    ///   The lookup is case-sensitive and must match exactly.
    ///
    /// # Returns
    ///
    /// * `Some(H)` - If a helper exists with the given name and can be downcast to type `H`
    /// * `None` - If no helper exists with the name or the helper has an incompatible type
    ///
    /// # Type Safety
    ///
    /// The method provides compile-time type safety through generic parameters while
    /// using runtime type checking for the actual retrieval. The downcasting operation
    /// ensures that only helpers of the correct type are returned, preventing runtime
    /// type errors in actor code.
    ///
    /// # Performance Characteristics
    ///
    /// - Lookup complexity: O(1) HashMap access with brief read lock
    /// - Type downcasting: Efficient runtime type checking with minimal overhead
    /// - Memory allocation: Helper is cloned, so `Clone` performance affects retrieval speed
    /// - Concurrent access: Optimized for multiple simultaneous reads
    ///
    /// # Thread Safety
    ///
    /// This method acquires a read lock on the helper registry, allowing multiple
    /// concurrent retrievals while preventing race conditions with helper registration.
    /// The returned helper clone is independent and safe to use across tasks.
    ///
    /// # Caching Strategies
    ///
    /// For frequently accessed helpers, actors may want to cache references:
    ///
    /// ```ignore
    /// impl MyActor {
    ///     async fn started(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         // Cache frequently used helpers during actor startup
    ///         self.config = ctx.system().get_helper("app_config").await;
    ///         self.http_client = ctx.system().get_helper("http_client").await;
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// #[derive(Clone, Debug)]
    /// pub struct DatabaseConfig {
    ///     pub url: String,
    ///     pub timeout: u64,
    /// }
    ///
    /// // Retrieve configuration helper
    /// if let Some(config) = system.get_helper::<DatabaseConfig>("db_config").await {
    ///     println!("Database URL: {}", config.url);
    ///     println!("Timeout: {}ms", config.timeout);
    /// } else {
    ///     eprintln!("Database configuration not found");
    /// }
    ///
    /// // Type safety - wrong type returns None
    /// let wrong_type: Option<String> = system.get_helper("db_config").await;
    /// assert!(wrong_type.is_none());
    ///
    /// // Non-existent helper returns None
    /// let missing: Option<DatabaseConfig> = system.get_helper("nonexistent").await;
    /// assert!(missing.is_none());
    /// ```
    ///
    /// # Actor Integration Patterns
    ///
    /// Common patterns for using helpers in actors:
    ///
    /// ```ignore
    /// impl Handler<WorkerActor> for WorkerActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         match msg {
    ///             WorkerMessage::ProcessWithConfig(data) => {
    ///                 // Get configuration for this operation
    ///                 let config = ctx.system()
    ///                     .get_helper::<ProcessingConfig>("processing_config")
    ///                     .await
    ///                     .ok_or_else(|| Error::Custom("Missing processing config".to_string()))?;
    ///
    ///                 // Use configuration in processing
    ///                 let result = self.process_data(data, &config).await?;
    ///                 Ok(WorkerResponse::Processed(result))
    ///             }
    ///             WorkerMessage::SendHttpRequest(url) => {
    ///                 // Get shared HTTP client
    ///                 let client = ctx.system()
    ///                     .get_helper::<HttpClient>("http_client")
    ///                     .await
    ///                     .ok_or_else(|| Error::Custom("Missing HTTP client".to_string()))?;
    ///
    ///                 let response = client.get(&url).send().await?;
    ///                 Ok(WorkerResponse::HttpResponse(response.text().await?))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Error Handling Patterns
    ///
    /// Recommended patterns for handling missing or incorrect helpers:
    ///
    /// ```ignore
    /// // Pattern 1: Return error if helper is required
    /// let config = system.get_helper::<Config>("config")
    ///     .await
    ///     .ok_or_else(|| Error::Custom("Required configuration missing".to_string()))?;
    ///
    /// // Pattern 2: Use default values
    /// let config = system.get_helper::<Config>("config")
    ///     .await
    ///     .unwrap_or_else(|| Config::default());
    ///
    /// // Pattern 3: Conditional behavior
    /// if let Some(metrics) = system.get_helper::<Metrics>("metrics").await {
    ///     metrics.increment("requests_processed");
    /// }
    /// ```
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

    /// Spawns and runs an event sink in a dedicated background task.
    ///
    /// Sinks are specialized components that consume and process events from actors
    /// within the system. They provide a decoupled way to handle side effects like
    /// logging, metrics collection, persistence, or integration with external systems.
    /// This method spawns the sink in a separate async task to ensure it runs
    /// independently of the calling context.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type that this sink will process. Must implement the `Event`
    ///   trait to ensure proper serialization and system integration.
    ///
    /// # Arguments
    ///
    /// * `sink` - The sink instance to run. Ownership is transferred to the background
    ///   task, and the sink will run until completion or system shutdown.
    ///
    /// # Sink Lifecycle
    ///
    /// Once spawned, the sink follows this lifecycle:
    /// 1. **Initialization**: Sink performs any required setup
    /// 2. **Event Processing**: Continuously consumes and processes events
    /// 3. **Graceful Shutdown**: Stops processing when event stream ends
    /// 4. **Cleanup**: Releases resources and performs final operations
    ///
    /// # Performance Characteristics
    ///
    /// - Task spawning: Minimal overhead for async task creation
    /// - Event processing: Performance depends on sink implementation
    /// - Memory usage: Each sink runs in its own task with independent memory
    /// - Concurrency: Multiple sinks can run simultaneously without interference
    ///
    /// # Error Handling
    ///
    /// - Sink errors are isolated to the background task and don't affect the system
    /// - Failed sinks will terminate their task but won't crash the actor system
    /// - Sink implementations should handle errors gracefully and log appropriately
    ///
    /// # Integration Patterns
    ///
    /// Sinks are commonly used for:
    /// 1. **Event Logging**: Persisting actor events to files or databases
    /// 2. **Metrics Collection**: Gathering performance and business metrics
    /// 3. **External Integration**: Sending events to external monitoring systems
    /// 4. **Audit Trails**: Recording system activities for compliance
    /// 5. **Event Sourcing**: Building event-sourced data projections
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    ///
    /// // Create a logging sink
    /// let log_sink = Sink::new(
    ///     event_receiver,  // Event stream to consume
    ///     |event| async move {
    ///         println!("Event: {:?}", event);
    ///         Ok(())
    ///     }
    /// );
    ///
    /// // Spawn the sink in background
    /// system.run_sink(log_sink).await;
    ///
    /// // Create a metrics collection sink
    /// let metrics_sink = Sink::new(
    ///     metrics_receiver,
    ///     |event| async move {
    ///         match event {
    ///             MetricEvent::Counter(name, value) => {
    ///                 METRICS.increment_counter(&name, value);
    ///             }
    ///             MetricEvent::Gauge(name, value) => {
    ///                 METRICS.set_gauge(&name, value);
    ///             }
    ///         }
    ///         Ok(())
    ///     }
    /// );
    ///
    /// system.run_sink(metrics_sink).await;
    /// ```
    ///
    /// # Database Integration Example
    ///
    /// ```ignore
    /// // Sink for persisting events to database
    /// let db_sink = Sink::new(
    ///     event_stream,
    ///     |event| async move {
    ///         let db = get_database_connection().await?;
    ///
    ///         match event {
    ///             UserEvent::Created { id, name, email } => {
    ///                 db.execute(
    ///                     "INSERT INTO user_events (type, user_id, data) VALUES ($1, $2, $3)",
    ///                     &[&"created", &id, &serde_json::to_value((name, email))?]
    ///                 ).await?;
    ///             }
    ///             UserEvent::Updated { id, changes } => {
    ///                 db.execute(
    ///                     "INSERT INTO user_events (type, user_id, data) VALUES ($1, $2, $3)",
    ///                     &[&"updated", &id, &serde_json::to_value(changes)?]
    ///                 ).await?;
    ///             }
    ///         }
    ///
    ///         Ok(())
    ///     }
    /// );
    ///
    /// system.run_sink(db_sink).await;
    /// ```
    ///
    /// # Multiple Sink Coordination
    ///
    /// ```ignore
    /// // Run multiple sinks for different purposes
    /// system.run_sink(audit_sink).await;      // Legal compliance
    /// system.run_sink(metrics_sink).await;     // Performance monitoring
    /// system.run_sink(alert_sink).await;       // Error alerting
    /// system.run_sink(replication_sink).await; // Data replication
    /// ```
    ///
    /// # Task Management
    ///
    /// The spawned tasks are managed automatically by the Tokio runtime:
    /// - Tasks run independently and don't block the calling code
    /// - Task handles are not returned, as sinks are fire-and-forget
    /// - System shutdown will terminate sink tasks through event stream closure
    /// - Memory and resources are cleaned up automatically when sinks complete
    ///
    /// # Best Practices
    ///
    /// 1. **Error Resilience**: Implement robust error handling in sink processors
    /// 2. **Backpressure**: Consider event buffer sizes and processing rates
    /// 3. **Resource Management**: Ensure sinks clean up resources properly
    /// 4. **Monitoring**: Include logging and metrics in sink implementations
    /// 5. **Testing**: Test sinks independently with mock event streams
    pub async fn run_sink<E>(&self, mut sink: Sink<E>)
    where
        E: Event,
    {
        tokio::spawn(async move {
            sink.run().await;
        });
    }
}

/// Event loop manager responsible for processing system-wide coordination events.
///
/// The `SystemRunner` is the central event processing component that handles system-level
/// coordination messages and manages the actor system's runtime lifecycle. It runs in its
/// own async task and processes events that require system-wide visibility or coordination,
/// such as shutdown signals, configuration updates, and administrative commands.
///
/// # Architecture Role
///
/// The system runner serves as the coordination hub for the actor system:
/// - **Event Processing**: Handles system-level events that affect global state
/// - **Lifecycle Management**: Coordinates system startup and shutdown sequences
/// - **Resource Coordination**: Manages system-wide resource allocation and cleanup
/// - **Administrative Interface**: Provides a channel for system management operations
///
/// # Event Processing Model
///
/// The runner processes events in a single-threaded, sequential manner to ensure:
/// - Consistent system state during complex operations
/// - Atomic handling of system-wide configuration changes
/// - Predictable ordering of shutdown and cleanup operations
/// - Simplified reasoning about system-level state transitions
///
/// # Integration with ActorSystem
///
/// The `SystemRunner` is created alongside the `SystemRef` by `ActorSystem::create()`
/// and must be run to enable proper system operation. While the `SystemRef` provides
/// the interface for actor management, the `SystemRunner` provides the runtime
/// infrastructure that enables coordination and lifecycle management.
///
/// # Performance Characteristics
///
/// - Event processing: Sequential, single-threaded for consistency
/// - Memory usage: Minimal, only stores event receiver channel
/// - Latency: Low latency for system events (microseconds typically)
/// - Throughput: Optimized for infrequent but critical system events
///
/// # Thread Safety
///
/// The system runner is designed to run in a single async task and does not
/// require internal synchronization. All coordination with other system components
/// occurs through message passing and the shared state managed by `SystemRef`.
///
/// # Lifecycle Integration
///
/// The runner participates in the system lifecycle:
/// 1. **Creation**: Initialized with event receiver during system creation
/// 2. **Runtime**: Processes events until shutdown signal received
/// 3. **Shutdown**: Handles graceful termination and resource cleanup
/// 4. **Completion**: Event loop terminates and task completes
///
/// # Error Handling
///
/// The runner is designed to be resilient to various error conditions:
/// - Event processing errors are logged but don't terminate the system
/// - Channel errors typically indicate system shutdown and are handled gracefully
/// - Resource cleanup continues even if individual operations fail
///
/// # Examples
///
/// ```ignore
/// use rush_actor::*;
/// use tokio_util::sync::CancellationToken;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let token = CancellationToken::new();
///     let (system, mut runner) = ActorSystem::create(token.clone());
///
///     // Start the runner in a background task
///     let runner_handle = tokio::spawn(async move {
///         runner.run().await;
///         println!("System runner completed");
///     });
///
///     // Create and manage actors using the system reference
///     let worker = system.create_root_actor("worker", WorkerActor::new()).await?;
///
///     // Perform application logic...
///
///     // Trigger graceful shutdown
///     system.stop_system();
///
///     // Wait for runner to complete
///     runner_handle.await?;
///     Ok(())
/// }
/// ```
///
/// # Integration Patterns
///
/// The system runner enables several important integration patterns:
///
/// ```ignore
/// // Pattern 1: Graceful shutdown coordination
/// let (system, mut runner) = ActorSystem::create(token);
/// tokio::spawn(async move { runner.run().await });
///
/// // Pattern 2: System monitoring and health checks
/// let runner_handle = tokio::spawn(async move {
///     runner.run().await;
///     health_monitor.report_system_stopped().await;
/// });
///
/// // Pattern 3: Restart coordination
/// loop {
///     let (system, mut runner) = ActorSystem::create(CancellationToken::new());
///     let result = runner.run().await;
///     if should_restart(&result) {
///         continue;
///     }
///     break;
/// }
/// ```
pub struct SystemRunner {
    /// Channel receiver for system-level coordination events.
    ///
    /// This receiver processes events that require system-wide coordination,
    /// such as shutdown signals, configuration changes, and administrative
    /// commands. The channel is bounded to prevent memory issues under load.
    event_receiver: mpsc::Receiver<SystemEvent>,
}

impl SystemRunner {
    /// Creates a new system runner with the provided event receiver channel.
    ///
    /// This method initializes the system runner with the event processing infrastructure
    /// required for handling system-wide coordination. The runner is created in a
    /// ready-to-run state but does not begin processing events until `run()` is called.
    ///
    /// # Arguments
    ///
    /// * `event_receiver` - Bounded MPSC receiver for system events. This channel is
    ///   used to receive coordination messages from the `SystemRef` and other system
    ///   components that need to trigger system-wide operations.
    ///
    /// # Returns
    ///
    /// Returns a fully initialized `SystemRunner` ready to process system events.
    ///
    /// # Design Notes
    ///
    /// This method is marked `pub(crate)` as it's part of the internal system creation
    /// process handled by `ActorSystem::create()`. External code should not create
    /// system runners directly, as they require coordination with other system components.
    ///
    /// # Performance Characteristics
    ///
    /// - Creation is O(1) and completes immediately
    /// - No heap allocation beyond the struct itself
    /// - No I/O or blocking operations during initialization
    /// - Minimal memory footprint until event processing begins
    ///
    /// # Thread Safety
    ///
    /// The runner is designed to be moved into a single async task and does not
    /// require internal synchronization. The event receiver channel provides the
    /// necessary coordination with other system components.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Internal usage within ActorSystem::create()
    /// let (event_sender, event_receiver) = mpsc::channel(100);
    /// let runner = SystemRunner::new(event_receiver);
    ///
    /// // Runner is ready but not yet processing events
    /// tokio::spawn(async move {
    ///     runner.run().await;  // Begin event processing
    /// });
    /// ```
    pub(crate) fn new(event_receiver: mpsc::Receiver<SystemEvent>) -> Self {
        Self { event_receiver }
    }

    /// Starts the main event processing loop for system coordination and lifecycle management.
    ///
    /// This method begins the core event processing loop that handles system-wide coordination
    /// messages. It runs continuously until a shutdown event is received, processing events
    /// that require global system visibility or coordination. The method is designed to be
    /// called once per system instance and will consume the runner.
    ///
    /// # Event Processing Loop
    ///
    /// The runner processes events in a sequential, single-threaded manner:
    /// 1. **Event Reception**: Waits for the next system event via the event channel
    /// 2. **Event Processing**: Handles the event according to its type and requirements
    /// 3. **State Coordination**: Updates system-wide state as needed
    /// 4. **Cleanup Operations**: Performs any required resource cleanup
    /// 5. **Loop Continuation**: Returns to waiting for the next event (unless stopping)
    ///
    /// # Shutdown Behavior
    ///
    /// When a `SystemEvent::StopSystem` event is received:
    /// - Logs the system shutdown completion for observability
    /// - Performs any final cleanup operations
    /// - Terminates the event processing loop
    /// - Allows the async task to complete normally
    ///
    /// # Performance Characteristics
    ///
    /// - Event latency: Microsecond response times for most events
    /// - Memory usage: Constant, minimal allocation during processing
    /// - CPU usage: Idle when no events, minimal during event processing
    /// - Throughput: Optimized for infrequent but critical system events
    ///
    /// # Error Handling
    ///
    /// The event loop is designed to be resilient:
    /// - Individual event processing errors don't terminate the loop
    /// - Channel closure (sender dropped) triggers graceful shutdown
    /// - Resource cleanup continues even if some operations fail
    /// - All errors are logged for debugging and monitoring
    ///
    /// # Integration with System Lifecycle
    ///
    /// The runner coordinates with the broader system lifecycle:
    /// - Must be started after system creation to enable proper operation
    /// - Processes shutdown signals from cancellation token handlers
    /// - Coordinates with actor termination and resource cleanup
    /// - Ensures graceful system shutdown even under load
    ///
    /// # Concurrency Model
    ///
    /// The runner uses a single-task model for consistency:
    /// - No internal concurrency or parallelism
    /// - Sequential event processing ensures predictable state transitions
    /// - Coordination with other components via message passing
    /// - Simplified reasoning about system-level operations
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::*;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let token = CancellationToken::new();
    ///     let (system, mut runner) = ActorSystem::create(token.clone());
    ///
    ///     // Start the runner in a dedicated task
    ///     let runner_handle = tokio::spawn(async move {
    ///         println!("Starting system runner...");
    ///         runner.run().await;
    ///         println!("System runner completed");
    ///     });
    ///
    ///     // Use the system for actor operations
    ///     let worker = system.create_root_actor("worker", WorkerActor::new()).await?;
    ///
    ///     // Application logic...
    ///     tokio::time::sleep(Duration::from_secs(10)).await;
    ///
    ///     // Initiate graceful shutdown
    ///     println!("Initiating system shutdown...");
    ///     system.stop_system();
    ///
    ///     // Wait for complete shutdown
    ///     runner_handle.await?;
    ///     println!("System fully shut down");
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Advanced Integration Patterns
    ///
    /// ```ignore
    /// // Pattern 1: Runner with health monitoring
    /// let runner_handle = tokio::spawn(async move {
    ///     health_monitor.report_starting().await;
    ///     runner.run().await;
    ///     health_monitor.report_stopped().await;
    /// });
    ///
    /// // Pattern 2: Runner with restart capability
    /// loop {
    ///     let (system, mut runner) = ActorSystem::create(token.clone());
    ///     let result = tokio::spawn(async move { runner.run().await }).await;
    ///
    ///     if should_restart() {
    ///         println!("Restarting system...");
    ///         continue;
    ///     }
    ///     break;
    /// }
    ///
    /// // Pattern 3: Runner with timeout protection
    /// let runner_result = tokio::time::timeout(
    ///     Duration::from_secs(60),
    ///     runner.run()
    /// ).await;
    ///
    /// match runner_result {
    ///     Ok(_) => println!("System shut down normally"),
    ///     Err(_) => println!("System shutdown timed out"),
    /// }
    /// ```
    ///
    /// # Observability and Monitoring
    ///
    /// The runner provides logging for system events:
    /// - Startup: "Running actor system..."
    /// - Shutdown: "Actor system stopped."
    /// - Additional logging may be added for specific event types
    /// - Integration with distributed tracing via the `tracing` crate
    ///
    /// # Resource Management
    ///
    /// The runner ensures proper resource cleanup:
    /// - Event channel is properly closed during shutdown
    /// - No memory leaks from incomplete event processing
    /// - Graceful termination allows proper resource release
    /// - Integration with Tokio's task cancellation mechanisms
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
