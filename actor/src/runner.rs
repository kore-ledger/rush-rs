// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor Internal Execution System
//!
//! This module provides the core runtime execution infrastructure for actors within the actor system.
//! The `ActorRunner` is the fundamental component responsible for managing the complete lifecycle
//! of individual actors, from creation and initialization through message processing to termination
//! and cleanup.
//!
//! The runner serves as the execution engine that orchestrates all aspects of actor behavior,
//! including message handling, event publishing, error management, supervision strategy application,
//! and integration with the broader actor system hierarchy.
//!
//! # Core Architecture
//!
//! ## Execution Model
//!
//! Each actor is executed within its own dedicated `ActorRunner` instance, which manages:
//!
//! - **Lifecycle Management**: Handles actor state transitions through Created → Started → Running → Stopped/Failed → Terminated
//! - **Message Processing**: Receives and dispatches messages from the actor's mailbox using a select-based event loop
//! - **Event Publishing**: Manages event broadcasting to subscribers through dedicated event channels
//! - **Error Handling**: Processes and propagates errors according to supervision policies
//! - **Parent-Child Communication**: Facilitates communication between parent and child actors in the hierarchy
//! - **Resource Management**: Ensures proper cleanup of channels, references, and system resources
//!
//! ## Threading and Concurrency
//!
//! The runner operates within Tokio's asynchronous runtime, using a single-threaded execution model
//! per actor instance. This design ensures:
//!
//! - **Isolation**: Each actor runs independently without sharing mutable state
//! - **Performance**: No locking overhead for actor-local operations
//! - **Determinism**: Message processing occurs sequentially within each actor
//! - **Scalability**: Thousands of actors can run concurrently on a single system
//!
//! ## Message Processing Loop
//!
//! The heart of the runner is an asynchronous select loop that concurrently handles:
//!
//! 1. **Stop Signals**: System shutdown and actor termination requests
//! 2. **Child Errors**: Error notifications from child actors requiring supervision decisions
//! 3. **Internal Actions**: Events, errors, and failures propagated within the actor hierarchy
//! 4. **External Messages**: User-defined messages sent to the actor from other actors
//!
//! ## Supervision Integration
//!
//! The runner implements the supervision system by:
//!
//! - **Applying Strategies**: Executes retry strategies with configurable backoff policies
//! - **Error Propagation**: Forwards failures to parent actors when supervision is required
//! - **State Management**: Maintains actor state consistency during restart operations
//! - **Recovery Coordination**: Orchestrates actor recovery while preserving system invariants
//!
//! # Usage Patterns
//!
//! ## Basic Actor Execution
//!
//! ```rust
//! use rush_actor::*;
//!
//! // Create runner and actor reference
//! let (mut runner, actor_ref, stop_sender) = ActorRunner::create(
//!     ActorPath::from("/user/worker"),
//!     WorkerActor::new(),
//!     None // No parent for root actors
//! );
//!
//! // Initialize and start the runner
//! let system = SystemRef::new(event_sender, cancellation_token);
//! tokio::spawn(async move {
//!     runner.init(system, stop_sender, None).await;
//! });
//!
//! // Send messages to the actor
//! actor_ref.tell(WorkMessage::Process(data)).await?;
//! ```
//!
//! ## Supervision Hierarchy
//!
//! ```rust
//! // Parent actor creates child with supervision
//! let (child_runner, child_ref, child_stop) = ActorRunner::create(
//!     ActorPath::from("/user/parent/child"),
//!     ChildActor::new(),
//!     Some(parent_error_sender) // Parent receives child errors
//! );
//! ```
//!
//! # Performance Characteristics
//!
//! The runner is optimized for:
//!
//! - **High Throughput**: Efficient message processing with minimal allocation overhead
//! - **Low Latency**: Direct message dispatch without intermediate queues
//! - **Memory Efficiency**: Bounded channel sizes prevent unbounded memory growth
//! - **Fault Isolation**: Actor failures don't impact sibling actors or system stability
//!
//! # Error Recovery
//!
//! The system implements comprehensive error handling:
//!
//! - **Startup Failures**: Configurable retry strategies with exponential backoff
//! - **Runtime Errors**: Propagation to parent actors for supervision decisions
//! - **Resource Cleanup**: Guaranteed cleanup even during abnormal termination
//! - **System Consistency**: Maintains actor hierarchy integrity during failures
//!

use crate::{
    ActorPath,
    Error,
    actor::{
        Actor, ActorContext, ActorLifecycle, ActorRef, ChildAction, ChildError,
        ChildErrorReceiver, ChildErrorSender, Handler,
    },
    //error::{error_box, ErrorBoxReceiver, ErrorHelper, SystemError},
    handler::{HandleHelper, MailboxReceiver, mailbox},
    supervision::{RetryStrategy, SupervisionStrategy},
    system::SystemRef,
};

use tokio::{
    select,
    sync::{
        broadcast::{self, Sender as EventSender},
        mpsc, oneshot,
    },
};
use tracing::{debug, error};

/// Channel sender for internal actor operations and event propagation.
///
/// This unbounded sender enables actors to send internal actions (events, errors, failures)
/// to their own runner for processing. The unbounded nature ensures that internal operations
/// never block, maintaining system responsiveness during high-load scenarios.
///
/// # Type Parameters
///
/// * `A` - The actor type that owns this sender, ensuring type safety for events and actions
///
/// # Usage
///
/// Used internally by `ActorContext` to propagate events and errors within the actor system
/// without blocking the message processing loop. The sender is cloned and shared with the
/// actor context to enable event publishing and error reporting.
pub type InnerSender<A> = mpsc::UnboundedSender<InnerAction<A>>;

/// Channel receiver for internal actor operations and event propagation.
///
/// This unbounded receiver processes internal actions sent by the actor through its context.
/// Internal actions include event publishing, error reporting, and failure notifications that
/// require special handling within the actor's execution loop.
///
/// # Type Parameters
///
/// * `A` - The actor type that owns this receiver, ensuring type safety for events and actions
///
/// # Processing
///
/// Received actions are processed in the main actor loop with high priority, ensuring
/// that events are published promptly and errors are handled according to supervision policies.
pub type InnerReceiver<A> = mpsc::UnboundedReceiver<InnerAction<A>>;

/// Channel receiver for actor stop signals and graceful shutdown coordination.
///
/// This receiver listens for stop signals that can be sent by parent actors, the system,
/// or external shutdown triggers. Each message optionally contains a oneshot sender
/// that allows the requester to wait for shutdown confirmation.
///
/// # Protocol
///
/// - `Some(Some(sender))` - Stop with confirmation callback
/// - `Some(None)` - Stop without confirmation
/// - `None` - Channel closed, indicating system shutdown
///
/// # Graceful Shutdown
///
/// When a stop signal is received, the actor:
/// 1. Calls `pre_stop()` hook for cleanup
/// 2. Stops all child actors recursively
/// 3. Removes itself from the system registry
/// 4. Sends confirmation if requested
/// 5. Transitions to stopped state
pub type StopReceiver = mpsc::Receiver<Option<oneshot::Sender<()>>>;

/// Channel sender for actor stop signals and graceful shutdown coordination.
///
/// This sender enables controlled shutdown of actors from external sources such as
/// parent actors, system shutdown procedures, or administrative commands. The bounded
/// channel (default capacity: 100) provides backpressure protection against shutdown
/// signal flooding while ensuring reliable delivery.
///
/// # Protocol
///
/// - Send `Some(Some(sender))` to stop with confirmation
/// - Send `Some(None)` to stop without confirmation
/// - Drop the sender to signal system shutdown
///
/// # Use Cases
///
/// - **Parent Supervision**: Parent actors can stop misbehaving children
/// - **System Shutdown**: Orderly shutdown of all actors in the system
/// - **Administrative Control**: External shutdown commands for maintenance
/// - **Resource Management**: Controlled cleanup during system reconfiguration
pub type StopSender = mpsc::Sender<Option<oneshot::Sender<()>>>;

/// Core execution engine for individual actors within the actor system.
///
/// The `ActorRunner` is responsible for the complete lifecycle management and execution
/// of a single actor instance. It orchestrates all aspects of actor operation including
/// message processing, event publishing, error handling, and supervision integration.
/// Each actor in the system has exactly one associated runner that manages its execution.
///
/// # Architecture Overview
///
/// The runner implements a single-threaded, event-driven execution model using Tokio's
/// asynchronous runtime. It operates a main event loop that concurrently handles:
///
/// - **Message Processing**: Receives and dispatches messages from the actor's mailbox
/// - **Event Broadcasting**: Publishes actor events to system subscribers
/// - **Error Management**: Processes errors and failures according to supervision policies
/// - **Lifecycle Control**: Manages transitions between actor states (Created → Started → Running → Terminated)
/// - **Parent-Child Communication**: Facilitates supervision and error propagation in actor hierarchies
///
/// # State Management
///
/// The runner maintains actor state through the `ActorLifecycle` enum, ensuring proper
/// state transitions and cleanup during normal operation and failure scenarios. State
/// changes trigger appropriate lifecycle hooks (`pre_start`, `pre_stop`, `post_stop`)
/// and supervision actions.
///
/// # Thread Safety
///
/// While the runner itself is not `Send` or `Sync` (as it manages mutable actor state),
/// it coordinates with thread-safe channels and references to enable safe concurrent
/// operation within the actor system.
///
/// # Type Parameters
///
/// * `A` - The actor type being executed, which must implement `Actor` and `Handler<A>`
///
/// # Examples
///
/// ```rust
/// use rush_actor::*;
///
/// // Create a new actor runner
/// let (mut runner, actor_ref, stop_sender) = ActorRunner::create(
///     ActorPath::from("/user/worker"),
///     WorkerActor::new(),
///     None, // No parent supervision
/// );
///
/// // Start the runner in a background task
/// tokio::spawn(async move {
///     runner.init(system, stop_sender, None).await;
/// });
/// ```
pub(crate) struct ActorRunner<A: Actor> {
    /// Hierarchical path identifying this actor within the system.
    ///
    /// The path serves as the unique identifier for the actor and defines its position
    /// in the system hierarchy. It's used for routing messages, logging, monitoring,
    /// and system administration. Paths follow the format `/system/user/child/grandchild`.
    path: ActorPath,

    /// The actor instance being executed by this runner.
    ///
    /// This is the user-defined actor implementation that contains the business logic
    /// and state. The runner delegates message handling, lifecycle events, and error
    /// processing to this instance while providing the execution context and system
    /// integration.
    actor: A,

    /// Current lifecycle state of the actor.
    ///
    /// Tracks the actor's progression through its lifecycle states: Created, Started,
    /// Restarted, Stopped, Failed, and Terminated. State transitions trigger appropriate
    /// lifecycle hooks and determine the runner's behavior in the main execution loop.
    lifecycle: ActorLifecycle,

    /// Message receiver from the actor's mailbox.
    ///
    /// Receives messages sent to this actor from other actors in the system. Messages
    /// are processed sequentially in the order received, ensuring deterministic behavior
    /// within each actor while allowing concurrent execution across different actors.
    receiver: MailboxReceiver<A>,

    /// Event broadcasting sender for publishing actor events.
    ///
    /// Enables the actor to publish events to multiple subscribers throughout the system.
    /// Events are broadcast immediately when published, allowing for real-time notification
    /// of actor state changes, business events, or system conditions.
    event_sender: EventSender<A::Event>,

    /// Receiver for stop signals from parent actors or system shutdown.
    ///
    /// Listens for graceful shutdown requests that can originate from parent actors
    /// during supervision actions, system-wide shutdown procedures, or administrative
    /// commands. Stop signals trigger controlled cleanup and proper resource release.
    stop_receiver: StopReceiver,

    /// Error sender for communication with child actors.
    ///
    /// Provides the communication channel that child actors use to report errors
    /// and failures to this parent actor. This sender is cloned and given to child
    /// actors during their creation, establishing the supervision relationship.
    error_sender: ChildErrorSender,

    /// Optional error sender for communication with parent actor.
    ///
    /// When this actor has a parent (i.e., it's not a root actor), this sender
    /// enables reporting errors and failures to the parent for supervision decisions.
    /// Root actors have `None` here and handle their own failures according to
    /// their configured supervision strategy.
    parent_sender: Option<ChildErrorSender>,

    /// Error receiver for handling child actor failures.
    ///
    /// Receives error notifications from child actors that require supervision
    /// decisions. The actor's supervision policy determines whether child failures
    /// result in restart attempts, escalation to parent, or child termination.
    error_receiver: ChildErrorReceiver,

    /// Internal action sender for event and error propagation.
    ///
    /// Used by the actor context to send internal actions (events, errors, failures)
    /// back to the runner for processing. This enables the actor to publish events
    /// and report errors without blocking message processing.
    inner_sender: InnerSender<A>,

    /// Internal action receiver for processing actor-generated actions.
    ///
    /// Processes internal actions sent by the actor through its context, including
    /// event publishing requests, error reports, and failure notifications that
    /// require special handling within the execution loop.
    inner_receiver: InnerReceiver<A>,

    /// Flag indicating whether a stop signal has been received.
    ///
    /// Used to control the main execution loop and prevent processing of new messages
    /// after a stop signal has been received. Ensures graceful shutdown by allowing
    /// current operations to complete while preventing new work from starting.
    stop_signal: bool,
}

impl<A> ActorRunner<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor runner, actor reference, and associated communication channels.
    ///
    /// This factory method initializes all the communication infrastructure required for
    /// actor execution, including message channels, event broadcasting, error reporting,
    /// and lifecycle management. It returns a complete actor execution environment ready
    /// for initialization and startup.
    ///
    /// # Architecture Setup
    ///
    /// The method establishes several key communication channels:
    ///
    /// - **Message Mailbox**: Unbounded channel for receiving messages from other actors
    /// - **Stop Channel**: Bounded channel (capacity: 100) for graceful shutdown coordination
    /// - **Error Channel**: Unbounded channel for child-to-parent error reporting in supervision
    /// - **Event Channel**: Broadcast channel (capacity: 10,000) for event publishing to subscribers
    /// - **Internal Channel**: Unbounded channel for actor context communication with runner
    ///
    /// # Supervision Integration
    ///
    /// When `parent_sender` is provided, it establishes a supervision relationship where
    /// this actor can report failures to its parent for supervision decisions. Root actors
    /// (with `parent_sender = None`) handle their own failures using their configured
    /// supervision strategy.
    ///
    /// # Channel Sizing Strategy
    ///
    /// Channel capacities are chosen based on expected usage patterns:
    /// - **Unbounded channels** for internal operations that should never block
    /// - **Bounded stop channel** to prevent shutdown signal flooding
    /// - **Large event channel** to handle burst event publishing scenarios
    ///
    /// # Arguments
    ///
    /// * `path` - Hierarchical path identifying the actor within the system
    /// * `actor` - The actor instance to be executed by this runner
    /// * `parent_sender` - Optional channel for reporting errors to parent actor
    ///                     (None for root actors, Some for child actors)
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// * `ActorRunner<A>` - The execution engine for this actor
    /// * `ActorRef<A>` - External reference for sending messages to this actor
    /// * `StopSender` - Channel for sending stop signals to this actor
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_actor::*;
    ///
    /// // Create a root actor (no supervision)
    /// let (runner, actor_ref, stop_sender) = ActorRunner::create(
    ///     ActorPath::from("/user/root"),
    ///     MyActor::new(),
    ///     None
    /// );
    ///
    /// // Create a child actor with supervision
    /// let (child_runner, child_ref, child_stop) = ActorRunner::create(
    ///     ActorPath::from("/user/parent/child"),
    ///     ChildActor::new(),
    ///     Some(parent_error_sender)
    /// );
    /// ```
    pub(crate) fn create(
        path: ActorPath,
        actor: A,
        parent_sender: Option<ChildErrorSender>,
    ) -> (Self, ActorRef<A>, StopSender) {
        debug!("Creating new actor runner.");
        let (sender, receiver) = mailbox();
        let (stop_sender, stop_receiver) = mpsc::channel(100);
        let (error_sender, error_receiver) = mpsc::unbounded_channel();
        let (event_sender, event_receiver) = broadcast::channel(10000);
        let (inner_sender, inner_receiver) = mpsc::unbounded_channel();
        let helper = HandleHelper::new(sender);

        //let error_helper = ErrorHelper::new(error_sender);
        let actor_ref = ActorRef::new(
            path.clone(),
            helper,
            stop_sender.clone(),
            event_receiver,
        );
        let runner: ActorRunner<A> = ActorRunner {
            path,
            actor,
            lifecycle: ActorLifecycle::Created,
            receiver,
            stop_receiver,
            event_sender,
            error_sender,
            parent_sender,
            error_receiver,
            inner_sender,
            inner_receiver,
            stop_signal: false,
        };
        (runner, actor_ref, stop_sender)
    }

    /// Initializes and runs the complete actor lifecycle from creation to termination.
    ///
    /// This is the main entry point for actor execution that orchestrates the entire actor
    /// lifecycle through a state machine. It handles actor startup, supervision strategies,
    /// error recovery, and graceful shutdown while maintaining system consistency and
    /// hierarchy integrity.
    ///
    /// # Lifecycle State Machine
    ///
    /// The actor progresses through these states:
    ///
    /// 1. **Created** → Calls `pre_start()`, transitions to Started on success, Failed on error
    /// 2. **Started** → Enters main execution loop, transitions to Failed on runtime errors
    /// 3. **Restarted** → Applies supervision strategy, may transition to Started or Stopped
    /// 4. **Failed** → Decides restart or termination based on parent supervision
    /// 5. **Stopped** → Calls `pre_stop()` and `post_stop()`, transitions to Terminated
    /// 6. **Terminated** → Final state, removes actor from system and cleans up resources
    ///
    /// # Supervision Integration
    ///
    /// During startup failures, the method applies the actor's configured supervision strategy:
    ///
    /// - **Stop Strategy**: Immediately terminates the failed actor
    /// - **Retry Strategy**: Attempts restart with configurable backoff policies
    /// - **Parent Strategy**: Defers decision to parent actor for child actors
    ///
    /// # Error Handling
    ///
    /// The method implements comprehensive error handling:
    ///
    /// - **Startup Failures**: Managed through supervision strategies with retry limits
    /// - **Runtime Errors**: Propagated to parent for supervision decisions
    /// - **Cleanup Failures**: Logged but don't prevent termination
    /// - **System Consistency**: Always removes actor from system registry on termination
    ///
    /// # Shutdown Coordination
    ///
    /// The optional `sender` parameter enables startup confirmation:
    /// - Sends `true` when actor successfully starts and enters running state
    /// - Sends `false` when actor fails to start or terminates before running
    /// - Used for synchronous actor creation and testing scenarios
    ///
    /// # Arguments
    ///
    /// * `system` - Reference to the actor system for global operations and registration
    /// * `stop_sender` - Channel for stopping child actors during cleanup
    /// * `sender` - Optional oneshot channel for startup confirmation notifications
    ///
    /// # Runtime Behavior
    ///
    /// Once started successfully, the actor enters its main execution loop (`run`) where
    /// it processes messages, handles child errors, and responds to system events until
    /// a stop condition is met. The loop ensures message ordering guarantees and provides
    /// fair scheduling between different types of operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rush_actor::*;
    ///
    /// // Basic actor initialization
    /// let system = SystemRef::new(event_sender, cancellation_token);
    /// tokio::spawn(async move {
    ///     runner.init(system, stop_sender, None).await;
    /// });
    ///
    /// // Initialization with startup confirmation
    /// let (startup_tx, startup_rx) = oneshot::channel();
    /// tokio::spawn(async move {
    ///     runner.init(system, stop_sender, Some(startup_tx)).await;
    /// });
    /// let started_successfully = startup_rx.await.unwrap();
    /// ```
    pub(crate) async fn init(
        &mut self,
        system: SystemRef,
        stop_sender: StopSender,
        mut sender: Option<oneshot::Sender<bool>>,
    ) {
        debug!("Initializing actor {} runner.", &self.path);

        // Create the actor context.
        debug!("Creating actor {} context.", &self.path);
        let mut ctx: ActorContext<A> = ActorContext::new(
            stop_sender,
            self.path.clone(),
            system.clone(),
            self.error_sender.clone(),
            self.inner_sender.clone(),
        );

        // Main loop of the actor.
        let mut retries = 0;
        loop {
            match self.lifecycle {
                // State: CREATED
                ActorLifecycle::Created => {
                    debug!("Actor {} is created.", &self.path);
                    // Pre-start hook.
                    match self.actor.pre_start(&mut ctx).await {
                        Ok(_) => {
                            debug!(
                                "Actor '{}' has started successfully.",
                                &self.path
                            );
                            self.lifecycle = ActorLifecycle::Started;
                        }
                        Err(err) => {
                            error!(
                                "Actor {} failed to start: {:?}",
                                &self.path, err
                            );
                            ctx.set_error(err);
                            self.lifecycle = ActorLifecycle::Failed;
                        }
                    }
                }
                // State: STARTED
                ActorLifecycle::Started => {
                    debug!("Actor {} is started.", &self.path);
                    if let Some(sender) = sender.take() {
                        sender.send(true).unwrap_or_else(|err| {
                            error!("Failed to send signal: {:?}", err);
                        });
                    }
                    self.run(&mut ctx).await;
                    if ctx.error().is_some() {
                        self.lifecycle = ActorLifecycle::Failed;
                    }
                }
                // State: RESTARTED
                ActorLifecycle::Restarted => {
                    // Apply supervision strategy.
                    self.apply_supervision_strategy(
                        A::supervision_strategy(),
                        &mut ctx,
                        &mut retries,
                    )
                    .await;
                }
                // State: STOPPED
                ActorLifecycle::Stopped => {
                    debug!("Actor {} is stopped.", &self.path);
                    // Post stop hook.
                    if self.actor.post_stop(&mut ctx).await.is_err() {
                        error!("Actor '{}' failed to stop!", &self.path);
                    }
                    self.lifecycle = ActorLifecycle::Terminated;
                }
                // State: FAILED
                ActorLifecycle::Failed => {
                    debug!("Actor {} is faulty.", &self.path);
                    if self.parent_sender.is_none() {
                        self.lifecycle = ActorLifecycle::Restarted;
                    } else {
                        // TODO: Parent supervision should determine the action here.
                        self.lifecycle = ActorLifecycle::Terminated;
                    }
                }
                // State: TERMINATED
                ActorLifecycle::Terminated => {
                    debug!("Actor {} is terminated.", &self.path);
                    ctx.system().remove_actor(&self.path.clone()).await;
                    if let Some(sender) = sender.take() {
                        sender.send(false).unwrap_or_else(|err| {
                            error!("Failed to send signal: {:?}", err);
                        });
                    }
                    break;
                }
            }
        }
        self.receiver.close();
    }

    /// Executes the main actor event loop for message processing and system coordination.
    ///
    /// This method implements the core execution engine of the actor system, running a
    /// select-based event loop that concurrently handles multiple types of operations
    /// while maintaining deterministic message processing within each actor. The loop
    /// continues until a stop condition is met or the actor fails.
    ///
    /// # Event Loop Architecture
    ///
    /// The loop uses Tokio's `select!` macro to concurrently wait on multiple channels,
    /// processing whichever event becomes available first. This provides:
    ///
    /// - **Fair Scheduling**: No single event type can monopolize the actor
    /// - **Responsive Shutdown**: Stop signals are processed immediately
    /// - **Error Prioritization**: Child errors are handled before regular messages
    /// - **Event Processing**: Internal actions are processed with high priority
    ///
    /// # Event Types and Priority
    ///
    /// Events are processed in this priority order (highest to lowest):
    ///
    /// 1. **Stop Signals**: Immediate graceful shutdown initiation
    /// 2. **Child Errors**: Error notifications from supervised child actors
    /// 3. **Internal Actions**: Event publishing, error reporting, and failure notifications
    /// 4. **External Messages**: User-defined messages from other actors
    ///
    /// # Shutdown Procedure
    ///
    /// When a stop signal is received, the actor follows a coordinated shutdown:
    ///
    /// 1. **Pre-Stop Hook**: Calls `actor.pre_stop()` for cleanup preparation
    /// 2. **Child Shutdown**: Stops all child actors recursively
    /// 3. **System Removal**: Removes itself from the system actor registry
    /// 4. **Confirmation**: Sends shutdown confirmation if requested
    /// 5. **State Transition**: Transitions to Stopped lifecycle state
    ///
    /// # Error Handling
    ///
    /// The loop handles different types of errors appropriately:
    ///
    /// - **Child Errors**: Delegated to actor's supervision policy
    /// - **Child Faults**: Require supervision decisions (stop/restart/delegate)
    /// - **Message Processing Errors**: Handled by individual message handlers
    /// - **Channel Closures**: Trigger graceful shutdown when senders are dropped
    ///
    /// # Flow Control
    ///
    /// The `stop_signal` flag prevents processing of new messages after shutdown
    /// initiation while allowing current operations to complete gracefully. This
    /// ensures system consistency and proper resource cleanup.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Actor context providing access to system services and lifecycle management
    ///
    /// # Performance Characteristics
    ///
    /// - **Zero-Copy Message Passing**: Messages are moved directly to handlers
    /// - **Bounded Memory Usage**: Channels have configured capacity limits
    /// - **Minimal Allocation**: Reuses data structures where possible
    /// - **CPU Efficiency**: No busy-waiting or polling
    ///
    /// # Examples
    ///
    /// The run loop is typically called automatically by `init()`, but can be used
    /// directly for custom lifecycle management:
    ///
    /// ```rust
    /// // Custom runner usage (advanced)
    /// let mut ctx = ActorContext::new(/* ... */);
    /// runner.run(&mut ctx).await;
    /// ```
    pub(crate) async fn run(&mut self, ctx: &mut ActorContext<A>) {
        debug!("Running actor {}.", &self.path);

        loop {
            select! {
                stop = self.stop_receiver.recv() => {
                    debug!("Stopping actor.");
                    if self.actor.pre_stop(ctx).await.is_err() {
                        error!("Failed to stop actor!");
                        let _ = ctx.emit_fail(Error::Stop).await;
                    }

                    ctx.stop_childs().await;
                    ctx.remove_actor().await;

                    if let Some(Some(stop)) = stop {
                        let _ = stop.send(());
                    }

                    if let ActorLifecycle::Started =  self.lifecycle {
                        self.lifecycle = ActorLifecycle::Stopped;
                    }
                    break;
                }
                // Handle error from `ErrorBoxReceiver`.
                error =  self.error_receiver.recv(), if !self.stop_signal => {
                    if let Some(error) = error {
                        match error {
                            ChildError::Error { error } => self.actor.on_child_error(error, ctx).await,
                            ChildError::Fault { error, sender } => {
                                let action = self.actor.on_child_fault(error, ctx).await;
                                if sender.send(action).is_err() {
                                    error!("Can not send action to child!");
                                }
                            },
                        }
                    } else {
                        ctx.stop(None).await;
                        self.stop_signal = true;
                    }
                }
                // Handle inner event from `inner_receiver`.
                recv = self.inner_receiver.recv(), if !self.stop_signal => {
                    if let Some(event) = recv {
                        self.inner_handle(event, ctx).await;
                    } else {
                        ctx.stop(None).await;
                        self.stop_signal = true;
                    }
                }
                // Gets message handler from mailbox receiver and push it to the messages queue.
                msg = self.receiver.recv(), if !self.stop_signal => {
                    if let Some(mut msg) = msg {
                        msg.handle(&mut self.actor, ctx).await;
                    } else {
                        ctx.stop(None).await;
                        self.stop_signal = true;
                    }
                }
            }
        }
    }

    /// Processes internal actions generated by the actor through its context.
    ///
    /// This method handles three types of internal actions that actors can generate
    /// during their execution: event publishing, error reporting, and failure notifications.
    /// These actions are processed with high priority in the main event loop to ensure
    /// timely system coordination and error handling.
    ///
    /// # Action Types
    ///
    /// ## Event Publishing (`InnerAction::Event`)
    /// Publishes actor events to all subscribers using the broadcast channel. Events
    /// are used for system monitoring, business logic coordination, and real-time
    /// notifications throughout the actor system.
    ///
    /// ## Error Reporting (`InnerAction::Error`)
    /// Reports non-fatal errors to the parent actor for logging and monitoring.
    /// These errors don't trigger supervision actions but provide visibility into
    /// actor health and system behavior.
    ///
    /// ## Failure Notification (`InnerAction::Fail`)
    /// Reports fatal failures that require supervision decisions. When a failure
    /// is reported, the parent actor's supervision policy determines whether to
    /// restart, stop, or delegate the decision further up the hierarchy.
    ///
    /// # Supervision Protocol
    ///
    /// For failure notifications, the method implements a request-response protocol
    /// with the parent actor:
    ///
    /// 1. **Send Fault**: Reports failure with a oneshot response channel
    /// 2. **Wait for Decision**: Blocks until parent makes supervision decision
    /// 3. **Apply Action**: Executes the supervision action (Stop/Restart/Delegate)
    /// 4. **State Transition**: Updates actor lifecycle state accordingly
    ///
    /// # Error Handling
    ///
    /// Communication failures with parent actors are logged but don't prevent
    /// the action from being processed. This ensures system resilience when
    /// parent actors are unavailable or have failed.
    ///
    /// # Arguments
    ///
    /// * `event` - The internal action to be processed
    /// * `ctx` - Actor context for accessing system services and state management
    ///
    /// # Examples
    ///
    /// Internal actions are typically generated through the actor context:
    ///
    /// ```rust
    /// // Publishing an event (generates InnerAction::Event)
    /// ctx.publish_event(MyEvent::DataProcessed).await?;
    ///
    /// // Reporting an error (generates InnerAction::Error)
    /// ctx.emit_error(Error::Functional("Processing warning".to_string())).await?;
    ///
    /// // Reporting a failure (generates InnerAction::Fail)
    /// ctx.emit_fail(Error::FunctionalFail("Critical error".to_string())).await?;
    /// ```
    async fn inner_handle(
        &mut self,
        event: InnerAction<A>,
        ctx: &mut ActorContext<A>,
    ) {
        match event {
            InnerAction::Event(event) => {
                match self.event_sender.send(event.clone()) {
                    Ok(size) => {
                        debug!(
                            "Event sent successfully to {} subscribers.",
                            size
                        );
                    }
                    Err(_err) => {
                        error!("Failed to send event");
                    }
                }
            }
            InnerAction::Error(error) => {
                if let Some(parent_helper) = self.parent_sender.as_mut() {
                    // Send error to parent.
                    parent_helper
                        .send(ChildError::Error { error })
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send error to parent actor: {:?}",
                                err
                            );
                        });
                }
            }
            InnerAction::Fail(error) => {
                // If the actor has a parent, send the fail to the parent.
                if let Some(parent_helper) = self.parent_sender.as_mut() {
                    let (action_sender, action_receiver) = oneshot::channel();
                    //self.action_receiver = Some(action_receiver);
                    parent_helper
                        .send(ChildError::Fault {
                            error,
                            sender: action_sender,
                        })
                        .unwrap_or_else(|err| {
                            error!(
                                "Failed to send fail to parent actor: {:?}",
                                err
                            );
                        });
                    // Sets the state from action.
                    if let Ok(action) = action_receiver.await {
                        // Clean error.
                        ctx.clean_error();
                        match action {
                            ChildAction::Stop => {}
                            ChildAction::Restart | ChildAction::Delegate => {
                                self.lifecycle = ActorLifecycle::Restarted;
                            }
                        }
                    }
                }
                ctx.stop(None).await;
                self.stop_signal = true;
            }
        }
    }

    /// Applies the configured supervision strategy when an actor fails during startup or restart.
    ///
    /// This method implements the supervision system's core decision-making logic,
    /// determining how to handle actor failures based on the configured strategy.
    /// It manages retry attempts, backoff delays, and ultimately decides whether
    /// to continue attempting recovery or terminate the actor.
    ///
    /// # Supervision Strategies
    ///
    /// ## Stop Strategy (`SupervisionStrategy::Stop`)
    /// Immediately terminates the failed actor without any retry attempts. This
    /// strategy is appropriate for actors where failures indicate permanent issues
    /// that cannot be resolved through restart.
    ///
    /// ## Retry Strategy (`SupervisionStrategy::Retry`)
    /// Attempts to restart the failed actor according to a configured retry policy.
    /// The strategy includes:
    /// - **Maximum Retry Count**: Limits total restart attempts
    /// - **Backoff Policy**: Configurable delay between retry attempts
    /// - **Failure Limit**: Stops retrying when limit is exceeded
    ///
    /// # Retry Management
    ///
    /// The method tracks retry attempts and applies backoff delays:
    ///
    /// 1. **Check Retry Limit**: Ensures retry count hasn't exceeded maximum
    /// 2. **Apply Backoff**: Waits for strategy-defined delay before retry
    /// 3. **Increment Counter**: Tracks total retry attempts
    /// 4. **Attempt Restart**: Calls actor's `pre_restart()` and reset lifecycle
    /// 5. **Handle Result**: Either return to normal operation or continue retrying
    ///
    /// # Backoff Policies
    ///
    /// Retry strategies can implement various backoff patterns:
    /// - **No Interval**: Immediate retry without delay
    /// - **Fixed Interval**: Constant delay between attempts
    /// - **Exponential Backoff**: Increasing delays to reduce system load
    /// - **Custom Patterns**: User-defined delay sequences
    ///
    /// # State Transitions
    ///
    /// The method manages lifecycle state transitions:
    /// - **Successful Restart**: `Restarted` → `Started` (retry count reset)
    /// - **Restart Failure**: Continues in `Restarted` state (increments retry count)
    /// - **Retry Exhausted**: `Restarted` → `Stopped` (permanent failure)
    ///
    /// # Arguments
    ///
    /// * `strategy` - The supervision strategy to apply from the actor's configuration
    /// * `ctx` - Actor context for performing restart operations and state management
    /// * `retries` - Mutable reference to the current retry count for this failure episode
    ///
    /// # Examples
    ///
    /// Supervision strategies are typically configured in the actor implementation:
    ///
    /// ```rust
    /// use rush_actor::supervision::*;
    /// use std::time::Duration;
    ///
    /// impl Actor for MyActor {
    ///     fn supervision_strategy() -> SupervisionStrategy {
    ///         // Retry up to 3 times with 1-second delays
    ///         SupervisionStrategy::Retry(
    ///             Strategy::FixedInterval(
    ///                 FixedIntervalStrategy::new(3, Duration::from_secs(1))
    ///             )
    ///         )
    ///     }
    /// }
    /// ```
    async fn apply_supervision_strategy(
        &mut self,
        strategy: SupervisionStrategy,
        ctx: &mut ActorContext<A>,
        retries: &mut usize,
    ) {
        match strategy {
            SupervisionStrategy::Stop => {
                error!("Actor '{}' failed to start!", &self.path);
                self.lifecycle = ActorLifecycle::Stopped;
            }
            SupervisionStrategy::Retry(mut retry_strategy) => {
                debug!(
                    "Restarting actor with retry strategy: {:?}",
                    &retry_strategy
                );
                if *retries < retry_strategy.max_retries() {
                    debug!("retries: {}", &retries);
                    if let Some(duration) = retry_strategy.next_backoff() {
                        debug!("Backoff for {:?}", &duration);
                        tokio::time::sleep(duration).await;
                    }
                    *retries += 1;
                    let error = ctx.error();
                    match ctx.restart(&mut self.actor, error.as_ref()).await {
                        Ok(_) => {
                            self.lifecycle = ActorLifecycle::Started;
                            *retries = 0;
                        }
                        Err(err) => {
                            ctx.set_error(err);
                        }
                    }
                } else {
                    self.lifecycle = ActorLifecycle::Stopped;
                }
            }
        }
    }
}

/// Internal actions that actors can generate through their execution context.
///
/// This enum represents the various internal operations that actors can trigger
/// during their execution, which require special processing by the actor runner.
/// These actions are sent through a dedicated internal channel to ensure they
/// are processed with appropriate priority and don't interfere with regular
/// message processing.
///
/// # Design Rationale
///
/// Internal actions are separated from regular messages to:
///
/// - **Maintain Message Ordering**: Regular messages maintain FIFO ordering
/// - **Priority Processing**: Internal actions are processed before external messages
/// - **System Coordination**: Enable actors to interact with system services
/// - **Error Handling**: Provide structured error reporting and failure management
///
/// # Usage Context
///
/// Internal actions are generated when actors call methods on their `ActorContext`:
///
/// - `ctx.publish_event()` → `InnerAction::Event`
/// - `ctx.emit_error()` → `InnerAction::Error`
/// - `ctx.emit_fail()` → `InnerAction::Fail`
///
/// # Processing Priority
///
/// The actor runner processes internal actions with high priority in the main
/// event loop, ensuring that events are published promptly and errors are
/// handled according to supervision policies without being delayed by regular
/// message processing.
///
/// # Type Parameters
///
/// * `A` - The actor type that generates these actions, ensuring type safety
///         for event publishing and maintaining consistency with the actor's
///         event type definition
///
/// # Thread Safety
///
/// This enum implements `Clone` to enable efficient distribution across the
/// internal communication channels while maintaining ownership semantics
/// for the contained data.
#[derive(Debug, Clone)]
pub enum InnerAction<A: Actor> {
    /// Event publishing request containing an actor-generated event.
    ///
    /// When an actor calls `ctx.publish_event()`, it generates this action to
    /// request that the event be broadcast to all subscribers. The runner
    /// processes this by sending the event through the broadcast channel,
    /// enabling real-time notifications throughout the actor system.
    ///
    /// # Event Broadcasting
    ///
    /// Events are published using Tokio's broadcast channel with a large buffer
    /// (default: 10,000 messages) to handle burst scenarios. If the buffer is
    /// full, the oldest events are dropped to maintain system responsiveness.
    ///
    /// # Use Cases
    ///
    /// - **Business Events**: Domain-specific events for application logic coordination
    /// - **State Changes**: Notifications when actor state changes occur
    /// - **System Events**: Infrastructure events for monitoring and debugging
    /// - **Integration Events**: Cross-system communication and data synchronization
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Actor publishes a business event
    /// ctx.publish_event(OrderEvent::Created { order_id: 123 }).await?;
    ///
    /// // System can subscribe to these events
    /// let mut events = actor_ref.subscribe_events().await;
    /// while let Ok(event) = events.recv().await {
    ///     match event {
    ///         OrderEvent::Created { order_id } => process_new_order(order_id),
    ///     }
    /// }
    /// ```
    Event(A::Event),

    /// Non-fatal error notification for monitoring and logging.
    ///
    /// This action reports errors that don't require supervision decisions but
    /// should be communicated to parent actors for logging, monitoring, or
    /// statistical purposes. These errors don't trigger actor restart or
    /// termination but provide visibility into actor health.
    ///
    /// # Error Propagation
    ///
    /// When processed, this action sends the error to the parent actor (if any)
    /// through the supervision channel. Parent actors receive these as
    /// `ChildError::Error` notifications and can handle them through their
    /// `on_child_error()` method.
    ///
    /// # Use Cases
    ///
    /// - **Warning Conditions**: Recoverable issues that don't affect functionality
    /// - **Performance Issues**: Slow operations or resource constraints
    /// - **Data Validation**: Input validation failures that can be ignored
    /// - **External Dependencies**: Temporary failures of external services
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Report a non-fatal error during processing
    /// if let Err(e) = external_api_call().await {
    ///     ctx.emit_error(Error::Functional(format!("API warning: {}", e))).await?;
    ///     // Continue processing with fallback logic
    /// }
    /// ```
    Error(Error),

    /// Fatal failure notification requiring supervision decision.
    ///
    /// This action reports failures that require intervention from the supervision
    /// system. When processed, it initiates a request-response protocol with the
    /// parent actor to determine the appropriate supervision action (restart,
    /// stop, or delegate to higher-level supervisor).
    ///
    /// # Supervision Protocol
    ///
    /// The failure triggers a synchronous supervision decision:
    ///
    /// 1. **Send Fault**: Reports failure to parent with response channel
    /// 2. **Wait Decision**: Blocks until parent actor responds
    /// 3. **Apply Action**: Executes the supervision decision
    /// 4. **State Update**: Transitions actor lifecycle accordingly
    ///
    /// # Supervision Actions
    ///
    /// Parent actors can respond with:
    /// - **Stop**: Terminate the failed actor permanently
    /// - **Restart**: Attempt to restart the actor with clean state
    /// - **Delegate**: Escalate decision to the next supervisor level
    ///
    /// # Use Cases
    ///
    /// - **Critical Errors**: Failures that compromise actor functionality
    /// - **Resource Exhaustion**: Out of memory or other resource failures
    /// - **State Corruption**: Inconsistent or invalid actor state detected
    /// - **Dependency Failures**: Critical external dependencies unavailable
    ///
    /// # Examples
    ///
    /// ```rust
    /// // Report a critical failure that requires supervision
    /// match critical_operation().await {
    ///     Err(CriticalError::StateCorrupted) => {
    ///         ctx.emit_fail(Error::FunctionalFail("State corrupted".to_string())).await?;
    ///         return; // Actor will be supervised appropriately
    ///     }
    ///     Ok(result) => process_result(result),
    /// }
    /// ```
    Fail(Error),
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::{
        Error,
        actor::{Actor, ActorContext, Event, Handler, Message},
        supervision::{FixedIntervalStrategy, Strategy, SupervisionStrategy},
        system::SystemRef,
    };
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    use tokio_util::sync::CancellationToken;
    use tracing_test::traced_test;

    use std::time::Duration;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestMessage(ErrorMessage);

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ErrorMessage {
        Stop,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TestEvent;

    impl Event for TestEvent {}

    #[derive(Debug, Clone)]
    pub struct TestActor {
        failed: bool,
    }

    #[async_trait]
    impl Actor for TestActor {
        type Message = TestMessage;
        type Response = ();
        type Event = TestEvent;

        fn supervision_strategy() -> SupervisionStrategy {
            SupervisionStrategy::Retry(Strategy::FixedInterval(
                FixedIntervalStrategy::new(3, Duration::from_secs(1)),
            ))
        }

        async fn pre_start(
            &mut self,
            _ctx: &mut ActorContext<Self>,
        ) -> Result<(), Error> {
            if self.failed {
                Err(Error::Start("PreStart failed".to_owned()))
            } else {
                Ok(())
            }
        }

        async fn pre_restart(
            &mut self,
            _ctx: &mut ActorContext<Self>,
            _error: Option<&Error>,
        ) -> Result<(), Error> {
            if self.failed {
                self.failed = false;
            }
            Ok(())
        }

        async fn post_stop(
            &mut self,
            _ctx: &mut ActorContext<Self>,
        ) -> Result<(), Error> {
            debug!("Post stop");
            Ok(())
        }
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: TestMessage,
            ctx: &mut ActorContext<Self>,
        ) -> Result<(), Error> {
            debug!("Handling empty message");
            match msg {
                TestMessage(ErrorMessage::Stop) => {
                    ctx.stop(None).await;
                    debug!("Actor stopped");
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_actor_root_failed() {
        let (event_sender, _) = mpsc::channel(100);

        let system = SystemRef::new(event_sender, CancellationToken::new());

        let actor = TestActor { failed: false };
        let (mut runner, actor_ref, stop_sender) =
            ActorRunner::create(ActorPath::from("/user/test"), actor, None);
        let inner_system = system.clone();

        // Init the actor runner.
        tokio::spawn(async move {
            runner.init(inner_system, stop_sender, None).await;
        });
        tokio::time::sleep(Duration::from_secs(1)).await;

        actor_ref
            .tell(TestMessage(ErrorMessage::Stop))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;

        assert!(logs_contain("Actor /user/test is terminated"));

        assert!(
            system
                .get_actor::<TestActor>(&ActorPath::from("/user/test"))
                .await
                .is_none()
        );

        let actor = TestActor { failed: true };

        let (mut runner, actor_ref, stop_sender) =
            ActorRunner::create(ActorPath::from("/user/test"), actor, None);
        let inner_system = system.clone();

        // Init the actor runner.
        tokio::spawn(async move {
            runner.init(inner_system, stop_sender, None).await;
        });

        tokio::time::sleep(Duration::from_secs(2)).await;

        assert!(logs_contain("Creating new actor runner"));
        assert!(logs_contain("Creating new handle reference"));
        assert!(logs_contain("Initializing actor /user/test runner"));
        assert!(logs_contain("Creating actor /user/test context"));
        assert!(logs_contain("Actor /user/test is created"));
        assert!(logs_contain("Actor /user/test failed to start"));
        assert!(logs_contain("Actor /user/test is faulty"));
        assert!(logs_contain("Restarting actor with retry strategy"));
        assert!(logs_contain("Actor /user/test is started"));
        assert!(logs_contain("Running actor /user/test"));

        actor_ref
            .tell(TestMessage(ErrorMessage::Stop))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(logs_contain("Actor /user/test is terminated"));

        assert!(
            system
                .get_actor::<TestActor>(&ActorPath::from("/user/test"))
                .await
                .is_none()
        );
    }
}
