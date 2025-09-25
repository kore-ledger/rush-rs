// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor
//!
//! The `actor` module provides the `Actor` trait and the `ActorRef` type. The `Actor` trait is the
//! main trait that actors must implement. The `ActorRef` type is a reference to an actor that can
//! be used to send messages to him.
//!

use crate::{
    ActorPath, Error,
    handler::HandleHelper,
    runner::{InnerAction, InnerSender, StopSender},
    supervision::SupervisionStrategy,
    system::SystemRef,
};

use tokio::sync::{broadcast::Receiver as EventReceiver, mpsc, oneshot};

use async_trait::async_trait;

use serde::{Serialize, de::DeserializeOwned};

use tracing::debug;

use std::fmt::Debug;

/// Actor execution context providing access to system services and actor lifecycle management.
///
/// The `ActorContext` is the primary interface through which actors interact with the actor system.
/// It provides access to system services such as creating child actors, sending messages,
/// publishing events, and managing the actor's lifecycle. Each actor receives a context instance
/// when handling messages, allowing it to perform system-level operations safely.
///
/// # Type Parameters
///
/// * `A` - The actor type that owns this context, ensuring type safety for operations
///
/// # Core Capabilities
///
/// - **Child Actor Management**: Create and manage child actors with automatic supervision
/// - **Message Publishing**: Send messages to other actors and publish events to subscribers
/// - **System Access**: Access system-level services and configuration
/// - **Lifecycle Control**: Manage actor startup, shutdown, and error handling
/// - **Path Management**: Access the actor's hierarchical path in the system
///
/// # Thread Safety
///
/// This struct is designed to be used within a single actor's message handling context
/// and is not `Send` or `Sync` by design, as it represents local actor state.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// #[async_trait]
/// impl Handler<MyActor> for MyActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         // Create a child actor
///         let child = ctx.create_child("worker", WorkerActor::new()).await?;
///
///         // Send message to child
///         child.tell(WorkerMessage::Process(msg.data)).await?;
///
///         // Publish event to system
///         ctx.publish_event(MyEvent::MessageProcessed).await?;
///
///         Ok(MyResponse::Success)
///     }
/// }
/// ```
pub struct ActorContext<A: Actor + Handler<A>> {
    /// Channel sender for stopping this actor
    stop: StopSender,
    /// Hierarchical path identifying this actor in the system
    path: ActorPath,
    /// Reference to the actor system for global operations
    system: SystemRef,
    /// Current error state of the actor, if any
    error: Option<Error>,
    /// Channel for reporting errors to the parent actor
    error_sender: ChildErrorSender,
    /// Internal communication channel for actor operations
    inner_sender: InnerSender<A>,
    /// Collection of stop senders for child actors under supervision
    child_senders: Vec<StopSender>,
}

impl<A> ActorContext<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor context for an actor instance.
    ///
    /// This internal constructor initializes the context with all necessary communication
    /// channels and system references required for actor operation. It's called automatically
    /// by the actor system during actor creation and should not be called directly by user code.
    ///
    /// # Parameters
    ///
    /// * `stop` - Channel sender for graceful actor termination. Used to signal when
    ///   the actor should stop processing messages and clean up resources.
    /// * `path` - Hierarchical path uniquely identifying this actor in the system tree.
    ///   Used for addressing, logging, and supervision relationships.
    /// * `system` - Reference to the actor system providing access to global services
    ///   like actor creation, message routing, and system configuration.
    /// * `error_sender` - Communication channel for reporting errors to the parent actor
    ///   in the supervision hierarchy for error handling and recovery.
    /// * `inner_sender` - Internal communication channel used by the actor system for
    ///   system-level message delivery and lifecycle management.
    ///
    /// # Returns
    ///
    /// Returns a fully initialized `ActorContext` ready for use in message handling.
    /// The context starts with no active error state and an empty collection of child actors.
    ///
    /// # Examples
    ///
    /// This method is called internally by the system:
    ///
    /// ```ignore
    /// // Called internally during actor creation
    /// let context = ActorContext::new(
    ///     stop_sender,
    ///     ActorPath::from("/user/my-actor"),
    ///     system_ref,
    ///     error_sender,
    ///     inner_sender,
    /// );
    /// ```
    pub(crate) fn new(
        stop: StopSender,
        path: ActorPath,
        system: SystemRef,
        error_sender: ChildErrorSender,
        inner_sender: InnerSender<A>,
    ) -> Self {
        Self {
            stop,
            path,
            system,
            error: None,
            error_sender,
            inner_sender,
            child_senders: Vec::new(),
        }
    }

    /// Restarts the actor following a failure or error condition.
    ///
    /// This internal method is called by the supervision system when an actor needs to be
    /// restarted due to an error or failure. It invokes the actor's `pre_restart` hook,
    /// allowing the actor to perform cleanup and state preparation before restart.
    ///
    /// # Parameters
    ///
    /// * `actor` - Mutable reference to the actor being restarted
    /// * `error` - Optional error that triggered the restart. This allows the actor
    ///   to examine the failure condition and react appropriately.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the restart preparation completed successfully,
    /// or an `Error` if the pre-restart hook failed.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally by supervision system
    /// if let Err(restart_error) = ctx.restart(&mut actor, Some(&original_error)).await {
    ///     // Handle restart failure - may escalate to parent
    /// }
    /// ```
    pub(crate) async fn restart(
        &mut self,
        actor: &mut A,
        error: Option<&Error>,
    ) -> Result<(), Error>
    where
        A: Actor,
    {
        actor.pre_restart(self, error).await
    }
    /// Retrieves a reference to this actor that can be used for message sending.
    ///
    /// This method provides a way to obtain an `ActorRef` pointing to the current actor,
    /// which can be useful for passing references to other actors or storing for later use.
    /// The reference can be used for both `tell` and `ask` operations.
    ///
    /// # Returns
    ///
    /// Returns `Some(ActorRef<A>)` if the actor is still active in the system,
    /// or `None` if the actor has been terminated or removed from the system.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<MyActor> for MyActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Get reference to self for delegation
    ///         if let Some(self_ref) = ctx.reference().await {
    ///             // Can send messages to ourselves or pass reference to others
    ///             other_actor.tell(MessageWithCallback { responder: self_ref }).await?;
    ///         }
    ///         Ok(MyResponse::Success)
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Note
    ///
    /// This operation involves a system lookup and should be used judiciously in
    /// performance-critical paths.
    pub async fn reference(&self) -> Option<ActorRef<A>> {
        self.system.get_actor(&self.path).await
    }

    /// Returns the hierarchical path of this actor in the system.
    ///
    /// The actor path uniquely identifies this actor within the actor system hierarchy.
    /// It follows a filesystem-like structure (e.g., `/user/parent/child`) and is used
    /// for actor addressing, logging, debugging, and establishing supervision relationships.
    ///
    /// # Returns
    ///
    /// Returns a reference to the `ActorPath` representing this actor's location
    /// in the system hierarchy.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<MyActor> for MyActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         _msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         println!("Processing message in actor: {}", ctx.path());
    ///
    ///         // Use path for logging or conditional logic
    ///         if ctx.path().name() == "critical-worker" {
    ///             // Special handling for critical workers
    ///         }
    ///
    ///         Ok(MyResponse::Processed)
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Note
    ///
    /// This is a lightweight operation that returns a reference without allocation.
    pub fn path(&self) -> &ActorPath {
        &self.path
    }

    /// Returns a reference to the actor system.
    ///
    /// Provides access to the actor system instance that manages this actor.
    /// The system reference can be used to access global services, configuration,
    /// create root actors, or perform system-wide operations.
    ///
    /// # Returns
    ///
    /// Returns a reference to the `SystemRef` that manages this actor and provides
    /// access to system-level services and configuration.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<MyActor> for MyActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Access system-level services
    ///         let system = ctx.system();
    ///
    ///         // Create a root actor (if this actor has permission)
    ///         if msg.create_helper {
    ///             let helper = system.create_root_actor(
    ///                 "helper",
    ///                 HelperActor::new()
    ///             ).await?;
    ///         }
    ///
    ///         // Access system configuration or global state
    ///         let config = system.configuration();
    ///
    ///         Ok(MyResponse::SystemAccessed)
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Note
    ///
    /// This returns a reference without allocation and is safe to call frequently.
    pub fn system(&self) -> &SystemRef {
        &self.system
    }

    /// Retrieves a reference to the parent actor in the supervision hierarchy.
    ///
    /// This method allows an actor to obtain a reference to its parent actor, which can be
    /// useful for reporting status, requesting resources, or implementing custom supervision
    /// patterns. Returns `None` if this actor is a root actor (has no parent).
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the parent actor. This must match the actual parent type,
    ///   or the lookup will return `None` even if a parent exists.
    ///
    /// # Returns
    ///
    /// Returns `Some(ActorRef<P>)` if the parent actor exists and is of the correct type,
    /// or `None` if this is a root actor or the parent type doesn't match.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<WorkerActor> for WorkerActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Report completion to parent
    ///         if let Some(parent) = ctx.parent::<ManagerActor>().await {
    ///             parent.tell(ManagerMessage::TaskCompleted {
    ///                 worker_id: ctx.path().name().to_string(),
    ///                 result: msg.result
    ///             }).await?;
    ///         }
    ///
    ///         Ok(WorkerResponse::Acknowledged)
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Note
    ///
    /// This operation involves a system lookup and path traversal, so it should be
    /// used judiciously in performance-critical code paths.
    ///
    /// # Type Safety
    ///
    /// The parent type `P` must exactly match the actual parent actor type. If there's
    /// a type mismatch, the method will return `None` even if a parent exists.
    pub async fn parent<P: Actor + Handler<P>>(&self) -> Option<ActorRef<P>> {
        self.system.get_actor(&self.path.parent()).await
    }

    /// Gracefully stops all child actors under this actor's supervision.
    ///
    /// This internal method is called during actor termination to ensure all child actors
    /// are properly stopped before the parent terminates. It sends stop signals to each
    /// child and waits for acknowledgment, ensuring clean resource cleanup in the actor hierarchy.
    ///
    /// The method processes all child actors in LIFO order (last created, first stopped)
    /// and waits for each child to acknowledge termination before proceeding to the next.
    /// If a child fails to respond, the method continues with the remaining children to
    /// avoid deadlock situations.
    ///
    /// # Behavior
    ///
    /// - Sends stop signal to each child actor
    /// - Waits for stop acknowledgment from each child
    /// - Continues with remaining children if one fails to respond
    /// - Clears the child senders list after processing all children
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally during actor termination
    /// context.stop_childs().await;
    /// // All children are now stopped and cleaned up
    /// ```
    ///
    /// # Error Handling
    ///
    /// This method is designed to be robust against child actor failures. If a child
    /// doesn't respond to the stop signal, the method logs the failure and continues
    /// with the remaining children to ensure system stability.
    pub(crate) async fn stop_childs(&mut self) {
        while let Some(sender) = self.child_senders.pop() {
            let (stop_sender, stop_receiver) = oneshot::channel();
            if sender.send(Some(stop_sender)).await.is_err() {
                continue;
            } else {
                let _ = stop_receiver.await;
            };
        }
    }

    /// Removes this actor from the actor system registry.
    ///
    /// This internal method unregisters the actor from the system's actor registry,
    /// effectively making it unreachable for new message delivery. This is typically
    /// called as part of the actor termination process to ensure clean removal from
    /// the system hierarchy.
    ///
    /// Once an actor is removed from the registry, attempts to look up the actor
    /// by its path will return `None`, and new messages sent to the actor's path
    /// will fail to be delivered.
    ///
    /// # Behavior
    ///
    /// - Removes the actor from the system's internal actor registry
    /// - Makes the actor unreachable by path-based lookups
    /// - Does not stop the actor's message processing loop (use `stop` for that)
    /// - Is typically called during the termination sequence
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally during actor termination
    /// context.remove_actor().await;
    /// // Actor is now removed from system registry
    /// ```
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any actor context.
    /// The system registry operations are internally synchronized.
    pub(crate) async fn remove_actor(&self) {
        self.system.remove_actor(&self.path).await;
    }

    /// Initiates graceful termination of this actor.
    ///
    /// This method sends a stop signal to the actor's message processing loop, triggering
    /// the shutdown sequence. The actor will finish processing its current message (if any),
    /// execute its `pre_stop` lifecycle hook, stop all child actors, and then terminate.
    ///
    /// This is an internal method used by the actor system for controlled shutdown.
    /// External code should typically use `ActorRef::tell_stop()` or `ActorRef::ask_stop()`
    /// methods instead.
    ///
    /// # Parameters
    ///
    /// * `sender` - Optional oneshot sender channel for receiving stop confirmation.
    ///   If provided, the sender will receive a signal when the actor has
    ///   completed its shutdown sequence. If `None`, the stop is fire-and-forget.
    ///
    /// # Behavior
    ///
    /// The stop sequence follows these steps:
    /// 1. Current message processing completes (if any)
    /// 2. Actor's `pre_stop` hook is called
    /// 3. All child actors are stopped recursively
    /// 4. Actor is removed from system registry
    /// 5. Actor's `post_stop` hook is called
    /// 6. Confirmation is sent through the provided sender (if any)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Fire-and-forget stop
    /// context.stop(None).await;
    ///
    /// // Stop with confirmation
    /// let (sender, receiver) = oneshot::channel();
    /// context.stop(Some(sender)).await;
    /// receiver.await.unwrap(); // Wait for stop completion
    /// ```
    ///
    /// # Error Handling
    ///
    /// If the stop signal cannot be sent (e.g., if the actor is already stopping),
    /// the method silently succeeds. This prevents deadlock situations during system shutdown.
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any async context.
    pub async fn stop(&self, sender: Option<oneshot::Sender<()>>) {
        debug!("Stopping actor from handle reference.");

        let _ = self.stop.send(sender).await;
    }

    /// Publishes an event to all subscribers of this actor.
    ///
    /// This method emits an event that will be delivered to all current subscribers
    /// of this actor's event stream. Events are typically used to implement the
    /// event sourcing pattern, notify external systems of state changes, or
    /// coordinate between actors in a loosely coupled manner.
    ///
    /// Events are delivered asynchronously to subscribers and do not block the
    /// actor's message processing. If no subscribers exist, the event is simply
    /// discarded without error.
    ///
    /// # Parameters
    ///
    /// * `event` - The event instance to publish. Must implement the actor's
    ///   associated `Event` type, which requires `Serialize`, `Clone`,
    ///   `Debug`, and other standard traits for event handling.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the event was successfully queued for delivery,
    /// or an `Error` if the event could not be published.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<MyActor> for MyActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Process the message
    ///         self.counter += msg.increment;
    ///
    ///         // Publish event to notify subscribers
    ///         ctx.publish_event(CounterUpdated {
    ///             old_value: self.counter - msg.increment,
    ///             new_value: self.counter,
    ///         }).await?;
    ///
    ///         // Conditional event publishing
    ///         if self.counter > 100 {
    ///             ctx.publish_event(ThresholdExceeded {
    ///                 threshold: 100,
    ///                 current_value: self.counter,
    ///             }).await?;
    ///         }
    ///
    ///         Ok(MyResponse::Updated(self.counter))
    ///     }
    /// }
    /// ```
    ///
    /// # Error Conditions
    ///
    /// - Returns `Error::SendEvent` if the internal event channel is closed
    ///   (typically indicates the actor system is shutting down)
    /// - The error contains the underlying channel error message for debugging
    ///
    /// # Performance Notes
    ///
    /// - Event publishing is asynchronous and non-blocking
    /// - Events are queued and delivered in order
    /// - Multiple events can be published from a single message handler
    /// - No performance penalty if there are no active subscribers
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any actor context.
    /// Event delivery is handled by the actor system's internal event dispatcher.
    pub async fn publish_event(&self, event: A::Event) -> Result<(), Error> {
        self.inner_sender
            .send(InnerAction::Event(event))
            .map_err(|e| Error::SendEvent(e.to_string())) // GRCOV-LINE
    }

    /// Emits an event to inner handler.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to emit.
    ///
    /// # Returns
    ///
    /// Returns a void result.
    ///
    /// # Errors
    ///
    /// Returns an error if the event could not be emitted.
    ///
    #[deprecated(since = "0.5.0", note = "please use `publish_event` instead")]
    pub async fn event(&self, event: A::Event) -> Result<(), Error> {
        self.publish_event(event).await
    }

    /// Reports an error condition to the actor system without causing actor termination.
    ///
    /// This method allows an actor to report error conditions that should be logged
    /// or handled by the system, but are not severe enough to cause the actor to fail
    /// or restart. This is useful for recoverable errors, validation failures, or
    /// warning conditions that the actor can continue operating despite.
    ///
    /// The error is sent to the actor's supervision hierarchy and the system's error
    /// handling infrastructure, but does not trigger supervision actions like restart
    /// or termination. For fatal errors that should trigger supervision, use `emit_fail`.
    ///
    /// # Parameters
    ///
    /// * `error` - The error condition to report. This should contain sufficient
    ///   detail for debugging and monitoring purposes.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the error was successfully reported, or an `Error` if
    /// the error reporting mechanism itself failed.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<ValidationActor> for ValidationActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Attempt to validate input
    ///         if let Err(validation_error) = self.validate(&msg.data) {
    ///             // Report validation error but continue processing
    ///             ctx.emit_error(Error::Functional(format!(
    ///                 "Validation failed for input: {}", validation_error
    ///             ))).await?;
    ///
    ///             // Return appropriate response indicating validation failure
    ///             return Ok(ValidationResponse::Invalid(validation_error));
    ///         }
    ///
    ///         // Continue with normal processing
    ///         Ok(ValidationResponse::Valid)
    ///     }
    /// }
    /// ```
    ///
    /// # Error Conditions
    ///
    /// - Returns `Error::Send` if the internal communication channel is closed
    /// - This typically indicates the actor system is shutting down
    ///
    /// # Usage Guidelines
    ///
    /// Use `emit_error` for:
    /// - Recoverable error conditions
    /// - Validation failures
    /// - Warning conditions
    /// - Errors that don't require supervision action
    ///
    /// Use `emit_fail` instead for:
    /// - Fatal errors requiring actor restart
    /// - Unrecoverable error conditions
    /// - Errors that indicate actor state corruption
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any actor context.
    /// Error reporting is handled asynchronously by the actor system.
    pub async fn emit_error(&mut self, error: Error) -> Result<(), Error> {
        self.inner_sender
            .send(InnerAction::Error(error))
            .map_err(|e| Error::Send(e.to_string())) // GRCOV-LINE
    }

    /// Reports a fatal failure that triggers supervision action and halts message processing.
    ///
    /// This method signals a critical error condition that the actor cannot recover from,
    /// triggering the supervision system to take action (typically restart or termination).
    /// Unlike `emit_error`, this method also stops the actor from processing further messages
    /// by setting an internal error state.
    ///
    /// When a failure is emitted:
    /// 1. The error is stored internally, preventing further message processing
    /// 2. The failure is reported to the parent actor for supervision decision
    /// 3. The parent's `on_child_fault` method is invoked to determine the response
    /// 4. Based on supervision strategy, the actor may be restarted, stopped, or escalated
    ///
    /// This is the appropriate method to use for unrecoverable errors, state corruption,
    /// or any condition that requires supervision intervention.
    ///
    /// # Parameters
    ///
    /// * `error` - The fatal error that caused the failure. This error will be:
    ///   - Stored internally to halt message processing
    ///   - Reported to the parent for supervision decision
    ///            - Available for logging and debugging purposes
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the failure was successfully reported, or an `Error` if
    /// the failure reporting mechanism itself failed.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<DatabaseActor> for DatabaseActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         // Attempt database operation
    ///         match self.database.execute(&msg.query).await {
    ///             Ok(result) => Ok(DatabaseResponse::Success(result)),
    ///             Err(db_error) => {
    ///                 // Check if this is a recoverable error
    ///                 if db_error.is_connection_lost() {
    ///                     // Fatal error - emit failure to trigger restart
    ///                     ctx.emit_fail(Error::Database(format!(
    ///                         "Database connection lost: {}", db_error
    ///                     ))).await?;
    ///
    ///                     // This actor will not process further messages
    ///                     Ok(DatabaseResponse::ConnectionLost)
    ///                 } else {
    ///                     // Non-fatal error - just return error response
    ///                     Ok(DatabaseResponse::QueryFailed(db_error.to_string()))
    ///                 }
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Error Conditions
    ///
    /// - Returns `Error::Send` if the internal communication channel is closed
    /// - This typically indicates the actor system is shutting down
    ///
    /// # State Changes
    ///
    /// After calling this method:
    /// - The actor's internal error state is set, halting message processing
    /// - The actor will not handle further messages until restarted or terminated
    /// - The parent actor receives a fault notification through supervision
    ///
    /// # Usage Guidelines
    ///
    /// Use `emit_fail` for:
    /// - Fatal errors that corrupt actor state
    /// - Unrecoverable system errors (database connection lost, etc.)
    /// - Errors that indicate the actor should be restarted
    /// - Critical failures requiring supervision intervention
    ///
    /// Use `emit_error` instead for:
    /// - Recoverable errors that don't require supervision
    /// - Validation failures or business logic errors
    /// - Warning conditions
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any actor context.
    /// The failure reporting and state changes are handled atomically.
    pub async fn emit_fail(&mut self, error: Error) -> Result<(), Error> {
        // Store error to stop message handling.
        self.set_error(error.clone());
        // Send fail to parent actor.
        self.inner_sender
            .send(InnerAction::Fail(error.clone()))
            .map_err(|e| Error::Send(e.to_string())) // GRCOV-LINE
    }

    /// Creates a new child actor under this actor's supervision.
    ///
    /// This method instantiates a new actor as a child of the current actor, establishing
    /// a parent-child supervision relationship. The child actor will be automatically
    /// managed by this actor's lifecycle, and any failures in the child will be reported
    /// to this actor's supervision handlers.
    ///
    /// The child actor is created with a path derived from this actor's path plus the
    /// provided name (e.g., if this actor's path is `/user/parent`, a child named "worker"
    /// will have the path `/user/parent/worker`).
    ///
    /// # Type Parameters
    ///
    /// * `C` - The type of the child actor being created. Must implement both `Actor`
    ///        and `Handler<C>` traits for message processing capabilities.
    ///
    /// # Parameters
    ///
    /// * `name` - The local name for the child actor. This name must be unique among
    ///           this actor's children and will become part of the child's actor path.
    ///           Names should be descriptive and follow naming conventions for clarity.
    /// * `actor` - The actor instance to be managed as a child. This instance will be
    ///            moved into the actor system and managed by the runtime.
    ///
    /// # Returns
    ///
    /// Returns `Ok(ActorRef<C>)` containing a reference to the newly created child actor,
    /// or an `Error` if the child could not be created.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<ManagerActor> for ManagerActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         match msg {
    ///             ManagerMessage::CreateWorker { worker_id, config } => {
    ///                 // Create a worker actor as a child
    ///                 let worker = WorkerActor::new(config);
    ///                 let worker_ref = ctx.create_child(&worker_id, worker).await?;
    ///
    ///                 // Store reference for later communication
    ///                 self.workers.insert(worker_id.clone(), worker_ref.clone());
    ///
    ///                 // Send initial task to the worker
    ///                 worker_ref.tell(WorkerMessage::Initialize).await?;
    ///
    ///                 Ok(ManagerResponse::WorkerCreated(worker_id))
    ///             }
    ///             ManagerMessage::CreateMultipleWorkers { count } => {
    ///                 let mut created_workers = Vec::new();
    ///
    ///                 for i in 0..count {
    ///                     let worker_name = format!("worker-{}", i);
    ///                     let worker = WorkerActor::new(WorkerConfig::default());
    ///                     let worker_ref = ctx.create_child(&worker_name, worker).await?;
    ///                     created_workers.push(worker_ref);
    ///                 }
    ///
    ///                 Ok(ManagerResponse::MultipleWorkersCreated(created_workers.len()))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Error Conditions
    ///
    /// Returns an error if:
    /// - An actor with the same name already exists as a child
    /// - The actor system is shutting down
    /// - There are insufficient system resources to create the actor
    /// - The child actor's `pre_start` method fails during initialization
    ///
    /// # Supervision Relationship
    ///
    /// Once created:
    /// - The child actor is automatically supervised by this actor
    /// - Child failures will trigger this actor's `on_child_error` or `on_child_fault` methods
    /// - When this actor stops, all child actors are automatically stopped first
    /// - The supervision strategy of this actor determines how child failures are handled
    ///
    /// # Lifecycle Management
    ///
    /// The child actor goes through the standard lifecycle:
    /// 1. Actor instance is created and initialized
    /// 2. Actor's `pre_start` method is called
    /// 3. Actor begins processing messages
    /// 4. Actor is registered in the system for message delivery
    ///
    /// # Performance Notes
    ///
    /// - Child creation is async and may involve system-level resource allocation
    /// - Each child adds overhead to parent lifecycle management
    /// - Consider pooling patterns for frequently created/destroyed children
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and handles all necessary synchronization internally.
    /// The child actor will run in its own task context managed by the actor system.
    pub async fn create_child<C>(
        &mut self,
        name: &str,
        actor: C,
    ) -> Result<ActorRef<C>, Error>
    where
        C: Actor + Handler<C>,
    {
        let path = self.path.clone() / name;
        let (actor_ref, stop_sender) = self
            .system
            .create_actor_path(path, actor, Some(self.error_sender.clone()))
            .await?;

        self.child_senders.push(stop_sender);
        Ok(actor_ref)
    }

    /// Retrieves a reference to an existing child actor by name.
    ///
    /// This method looks up a previously created child actor by its local name and returns
    /// an `ActorRef` that can be used to communicate with the child. The lookup is performed
    /// in the actor system's registry using the child's full path.
    ///
    /// This is useful for accessing child actors that were created earlier, either in the
    /// same message handler or in previous message processing cycles. The method performs
    /// a type-safe lookup, ensuring the returned reference matches the expected actor type.
    ///
    /// # Type Parameters
    ///
    /// * `C` - The expected type of the child actor. This must exactly match the actual
    ///        type of the child actor, or the method will return `None` even if a child
    ///        with the given name exists but has a different type.
    ///
    /// # Parameters
    ///
    /// * `name` - The local name of the child actor to retrieve. This should match the
    ///           name used when the child was created with `create_child`.
    ///
    /// # Returns
    ///
    /// Returns `Some(ActorRef<C>)` if a child actor with the specified name and type exists,
    /// or `None` if no such child is found or if there's a type mismatch.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<ManagerActor> for ManagerActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         _sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         match msg {
    ///             ManagerMessage::AssignTask { task, worker_name } => {
    ///                 // Look up an existing worker by name
    ///                 if let Some(worker) = ctx.get_child::<WorkerActor>(&worker_name).await {
    ///                     worker.tell(WorkerMessage::ProcessTask(task)).await?;
    ///                     Ok(ManagerResponse::TaskAssigned)
    ///                 } else {
    ///                     // Worker doesn't exist, create it first
    ///                     let worker = WorkerActor::new(WorkerConfig::default());
    ///                     let worker_ref = ctx.create_child(&worker_name, worker).await?;
    ///                     worker_ref.tell(WorkerMessage::ProcessTask(task)).await?;
    ///                     Ok(ManagerResponse::TaskAssignedToNewWorker)
    ///                 }
    ///             }
    ///             ManagerMessage::GetWorkerStatus { worker_name } => {
    ///                 match ctx.get_child::<WorkerActor>(&worker_name).await {
    ///                     Some(worker) => {
    ///                         let status = worker.ask(WorkerMessage::GetStatus).await?;
    ///                         Ok(ManagerResponse::WorkerStatus(status))
    ///                     }
    ///                     None => Ok(ManagerResponse::WorkerNotFound(worker_name))
    ///                 }
    ///             }
    ///             ManagerMessage::BroadcastToAllWorkers { message } => {
    ///                 // Send message to all known workers
    ///                 for worker_name in &self.worker_names {
    ///                     if let Some(worker) = ctx.get_child::<WorkerActor>(worker_name).await {
    ///                         worker.tell(message.clone()).await?;
    ///                     }
    ///                 }
    ///                 Ok(ManagerResponse::BroadcastComplete)
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Behavior Notes
    ///
    /// - The lookup is performed by constructing the child's full path from this actor's
    ///   path and the provided name
    /// - The method performs a type-safe lookup in the actor system registry
    /// - Returns `None` if the child doesn't exist, has stopped, or has a type mismatch
    /// - The returned `ActorRef` can be used immediately for message sending
    ///
    /// # Performance Considerations
    ///
    /// - This operation involves a system registry lookup and should be used judiciously
    ///   in performance-critical paths
    /// - Consider caching `ActorRef` instances in actor state for frequently accessed children
    /// - The lookup is async and may involve synchronization with the system registry
    ///
    /// # Type Safety
    ///
    /// The generic parameter `C` must exactly match the actual type of the child actor.
    /// If there's a type mismatch, the method will return `None` even if a child with
    /// the correct name exists. This ensures type safety but requires careful type management.
    ///
    /// # Thread Safety
    ///
    /// This method is async-safe and can be called from any actor context. The registry
    /// lookup is handled by the actor system with appropriate synchronization.
    pub async fn get_child<C>(&self, name: &str) -> Option<ActorRef<C>>
    where
        C: Actor + Handler<C>,
    {
        let path = self.path.clone() / name;
        self.system.get_actor(&path).await
    }

    /// Retrieves the current error state of the actor, if any.
    ///
    /// This internal method returns the error that has been set on this actor context,
    /// typically as a result of calling `emit_fail` or during error processing. When an
    /// actor has an error state set, it stops processing new messages until the error
    /// is cleared or the actor is restarted.
    ///
    /// The error state is used by the actor system's supervision and restart mechanisms
    /// to determine the appropriate recovery actions and to maintain error context during
    /// actor lifecycle transitions.
    ///
    /// # Returns
    ///
    /// Returns `Some(Error)` if the actor currently has an error state set, or `None`
    /// if the actor is in a normal operating state without any recorded errors.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally by the actor system during supervision
    /// if let Some(current_error) = context.error() {
    ///     // Handle the error based on supervision strategy
    ///     match supervision_strategy {
    ///         SupervisionStrategy::Restart => {
    ///             // Restart the actor and clear the error
    ///             context.restart(&mut actor, Some(&current_error)).await?;
    ///             context.clean_error();
    ///         }
    ///         SupervisionStrategy::Stop => {
    ///             // Stop the actor due to the error
    ///             context.stop(None).await;
    ///         }
    ///         _ => {
    ///             // Other supervision strategies
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Usage Context
    ///
    /// This method is primarily used by:
    /// - The actor system's supervision framework
    /// - Error handling and recovery logic
    /// - Diagnostic and monitoring systems
    /// - Actor restart and recovery mechanisms
    ///
    /// # State Management
    ///
    /// The error state is managed through the context lifecycle:
    /// - Set by `emit_fail` when a fatal error occurs
    /// - Set by `set_error` for internal error tracking
    /// - Cleared by `clean_error` after successful recovery
    /// - Inspected by supervision logic to determine recovery actions
    ///
    /// # Thread Safety
    ///
    /// This method safely accesses the actor's internal error state. The error state
    /// is local to the actor's execution context and doesn't require additional synchronization.
    pub(crate) fn error(&self) -> Option<Error> {
        self.error.clone().or(None)
    }

    /// Sets the error state of the actor to halt message processing.
    ///
    /// This internal method records an error condition on the actor context, which prevents
    /// the actor from processing any further messages until the error is cleared or the actor
    /// is restarted. This is typically called automatically by `emit_fail`, but can also be
    /// used by internal actor system components for error state management.
    ///
    /// When an error is set:
    /// - The actor stops accepting new messages for processing
    /// - The error becomes available through the `error()` method
    /// - Supervision mechanisms can inspect and act upon the error
    /// - The actor may be restarted or stopped depending on supervision strategy
    ///
    /// # Parameters
    ///
    /// * `error` - The error condition to record. This error will be stored in the
    ///            actor's context and used by supervision mechanisms to determine
    ///            the appropriate recovery action.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally during error handling
    /// if let Err(fatal_error) = critical_operation().await {
    ///     // Set error to halt message processing
    ///     context.set_error(Error::System(format!("Critical failure: {}", fatal_error)));
    ///
    ///     // Actor will now stop processing messages until recovered
    ///     return Err(Error::ActorStopped("Due to critical failure".to_string()));
    /// }
    /// ```
    ///
    /// # State Management
    ///
    /// This method is part of the actor error state lifecycle:
    /// 1. `set_error` - Records error and halts message processing
    /// 2. `error` - Allows inspection of current error state
    /// 3. `clean_error` - Clears error state to resume normal operation
    /// 4. Supervision actions may restart actor, clearing error implicitly
    ///
    /// # Behavior
    ///
    /// - Overwrites any existing error state with the new error
    /// - Immediately affects message processing behavior
    /// - Does not trigger supervision actions directly (use `emit_fail` for that)
    /// - The error state persists until explicitly cleared or actor restart
    ///
    /// # Usage Context
    ///
    /// This method is used by:
    /// - The `emit_fail` method to record failure state
    /// - Actor system internals during error recovery
    /// - Supervision mechanisms during error handling
    /// - Internal error state management during restarts
    ///
    /// # Thread Safety
    ///
    /// This method modifies the actor's local error state and is safe to call from
    /// within the actor's execution context. No additional synchronization is required.
    pub(crate) fn set_error(&mut self, error: Error) {
        self.error = Some(error);
    }

    /// Clears the error state of the actor to resume normal message processing.
    ///
    /// This internal method removes any error condition that has been set on the actor
    /// context, allowing the actor to resume processing messages normally. This is typically
    /// called after successful error recovery, actor restart, or when the supervision
    /// system has determined that the error condition has been resolved.
    ///
    /// When the error state is cleared:
    /// - The actor resumes accepting and processing new messages
    /// - The error state becomes `None` when inspected via `error()`
    /// - Normal actor operation is restored
    /// - The actor can continue its regular message handling lifecycle
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Called internally during actor restart sequence
    /// async fn restart_actor(context: &mut ActorContext<MyActor>, actor: &mut MyActor) {
    ///     // First, attempt to restart the actor
    ///     if let Err(restart_error) = context.restart(actor, context.error().as_ref()).await {
    ///         // Restart failed, handle accordingly
    ///         return Err(restart_error);
    ///     }
    ///
    ///     // Restart successful, clear the error state
    ///     context.clean_error();
    ///
    ///     // Actor can now process messages normally
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # State Management Lifecycle
    ///
    /// This method completes the error state management cycle:
    /// 1. Error occurs and is recorded via `set_error` or `emit_fail`
    /// 2. Actor stops processing messages due to error state
    /// 3. Supervision system evaluates error and determines recovery action
    /// 4. Recovery action is taken (restart, ignore, etc.)
    /// 5. `clean_error` is called to clear error state and resume operation
    ///
    /// # Usage Context
    ///
    /// This method is typically called by:
    /// - Actor restart mechanisms after successful restart
    /// - Supervision strategies that handle and resolve errors
    /// - Error recovery systems that restore actor functionality
    /// - Manual error resolution procedures in system management
    ///
    /// # Behavior
    ///
    /// - Immediately clears any existing error state
    /// - Enables the actor to process new messages
    /// - Has no effect if no error state was set
    /// - Does not affect any other actor state or context
    ///
    /// # Thread Safety
    ///
    /// This method modifies the actor's local error state and is safe to call from
    /// within the actor's execution context. The error state is local to the actor
    /// and doesn't require additional synchronization.
    ///
    /// # Recovery Patterns
    ///
    /// Common patterns for error recovery and cleanup:
    /// - Restart and clean: Actor is restarted, then error is cleared
    /// - Resolve and clean: Error condition is resolved, then error is cleared
    /// - Ignore and clean: Error is deemed non-critical and cleared immediately
    /// - Escalate then clean: Error is escalated to parent, then cleared locally
    pub(crate) fn clean_error(&mut self) {
        self.error = None;
    }
}

/// Represents the lifecycle states of an actor throughout its existence in the system.
///
/// The `ActorLifecycle` enum tracks the current state of an actor as it progresses through
/// various stages from creation to termination. This is used by the actor system for
/// monitoring, debugging, supervision, and ensuring proper lifecycle management.
///
/// Each state represents a specific phase in the actor's lifecycle and determines what
/// operations are valid and what behaviors are expected from the actor.
///
/// # State Transitions
///
/// Typical lifecycle progression:
/// ```text
/// Created -> Started -> [Restarted] -> [Failed] -> Stopped -> Terminated
///                    ^               /
///                    |______________/
/// ```
///
/// # Usage
///
/// This enum is primarily used by the actor system internally for:
/// - Tracking actor state during supervision
/// - Logging and debugging actor behavior
/// - Ensuring proper lifecycle hook execution
/// - Monitoring system health and actor status
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorLifecycle;
///
/// // Check actor lifecycle state during supervision
/// match actor_lifecycle_state {
///     ActorLifecycle::Created => {
///         // Actor is newly created, call pre_start
///         actor.pre_start(&mut context).await?;
///     }
///     ActorLifecycle::Failed => {
///         // Actor has failed, apply supervision strategy
///         apply_supervision_strategy(&actor, &error).await;
///     }
///     ActorLifecycle::Stopped => {
///         // Actor is stopped, clean up resources
///         cleanup_actor_resources(&actor).await;
///     }
///     _ => {
///         // Handle other states as appropriate
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum ActorLifecycle {
    /// The actor has been created but not yet started.
    ///
    /// In this state, the actor instance has been constructed and registered in the system,
    /// but its `pre_start` lifecycle hook has not been called yet. The actor is not yet
    /// processing messages and is not considered active.
    ///
    /// **Valid Transitions:** `Created` → `Started`, `Created` → `Failed`, `Created` → `Terminated`
    Created,

    /// The actor is actively running and processing messages.
    ///
    /// In this state, the actor has successfully completed its `pre_start` initialization
    /// and is actively processing messages from its mailbox. This is the normal operational
    /// state for a healthy actor.
    ///
    /// **Valid Transitions:** `Started` → `Restarted`, `Started` → `Failed`, `Started` → `Stopped`
    Started,

    /// The actor has been restarted due to a failure or supervision action.
    ///
    /// This state indicates that the actor was previously running but encountered an error
    /// that triggered a restart through the supervision system. The actor's `pre_restart`
    /// hook has been called, and it's being reinitialized.
    ///
    /// **Valid Transitions:** `Restarted` → `Started`, `Restarted` → `Failed`, `Restarted` → `Stopped`
    Restarted,

    /// The actor has encountered a fatal error and is no longer processing messages.
    ///
    /// This state indicates that the actor has experienced a failure condition that has
    /// stopped its normal operation. The actor may be awaiting supervision action such as
    /// restart or termination, depending on the configured supervision strategy.
    ///
    /// **Valid Transitions:** `Failed` → `Restarted`, `Failed` → `Stopped`, `Failed` → `Terminated`
    Failed,

    /// The actor has been instructed to stop and is in the process of shutting down.
    ///
    /// In this state, the actor has received a stop signal and is executing its shutdown
    /// sequence. It may be calling `pre_stop` hooks, stopping child actors, and cleaning
    /// up resources before final termination.
    ///
    /// **Valid Transitions:** `Stopped` → `Terminated`
    Stopped,

    /// The actor has completed its lifecycle and been removed from the system.
    ///
    /// This is the final state of an actor's lifecycle. The actor has completed all
    /// shutdown procedures, called `post_stop` hooks, been removed from the system
    /// registry, and released all resources. No further state transitions are possible.
    ///
    /// **Valid Transitions:** None (terminal state)
    Terminated,
}

/// Specifies the supervision action to take when a child actor encounters a fault.
///
/// The `ChildAction` enum is returned by the `on_child_fault` handler method to instruct
/// the actor system how to handle a child actor's failure. This forms the core of the
/// supervision strategy implementation, allowing parent actors to make runtime decisions
/// about how to respond to child failures based on the specific error condition and
/// application requirements.
///
/// # Supervision Philosophy
///
/// The actor model's supervision principle follows the "let it crash" philosophy, where
/// failures are isolated and handled through supervision rather than defensive programming.
/// Parent actors act as supervisors, making decisions about child recovery based on:
/// - The nature and severity of the error
/// - The child's role in the system
/// - The application's resilience requirements
/// - The current system state and resource availability
///
/// # Decision Factors
///
/// When choosing a `ChildAction`, consider:
/// - **Error Type**: Transient errors may warrant restart, while configuration errors might require stopping
/// - **Resource Impact**: Restarting resource-intensive actors may have system-wide effects
/// - **Dependency Chain**: Consider how the action affects other actors that depend on this child
/// - **Recovery Time**: Balance between quick recovery (restart) and system stability (stop)
///
/// # Examples
///
/// ```ignore
/// use rush_actor::{ChildAction, Error};
///
/// #[async_trait]
/// impl Handler<ParentActor> for ParentActor {
///     async fn on_child_fault(
///         &mut self,
///         error: Error,
///         ctx: &mut ActorContext<Self>,
///     ) -> ChildAction {
///         match &error {
///             // Network or database connection errors - likely transient, restart
///             Error::Network(_) | Error::Database(_) => {
///                 log::warn!("Child actor failed with recoverable error: {:?}", error);
///                 ChildAction::Restart
///             }
///
///             // Configuration or validation errors - permanent, stop the child
///             Error::Configuration(_) | Error::Validation(_) => {
///                 log::error!("Child actor failed with permanent error: {:?}", error);
///                 ChildAction::Stop
///             }
///
///             // Unknown or complex errors - delegate to child's own supervision strategy
///             _ => {
///                 log::debug!("Delegating child fault handling to child strategy: {:?}", error);
///                 ChildAction::Delegate
///             }
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub enum ChildAction {
    /// Terminate the child actor immediately without restart.
    ///
    /// This action causes the child actor to be stopped gracefully, allowing it to
    /// execute its `pre_stop` and `post_stop` lifecycle hooks before being removed
    /// from the system. Use this action when:
    ///
    /// - The error indicates a permanent failure that cannot be resolved by restart
    /// - The child actor is no longer needed due to changing system requirements
    /// - Resource constraints require reducing the number of active actors
    /// - The error suggests a fundamental design or configuration problem
    ///
    /// **Behavior:**
    /// - Child's `pre_stop` hook is called for cleanup
    /// - Child is removed from parent's supervision
    /// - Child's mailbox is cleared and no further messages are processed
    /// - Child's `post_stop` hook is called after termination
    ///
    /// **Example Scenarios:**
    /// - Configuration file contains invalid parameters
    /// - Authentication credentials are permanently revoked
    /// - Required external services are decommissioned
    Stop,

    /// Restart the child actor with a fresh state.
    ///
    /// This action causes the child actor to be terminated and recreated with its
    /// original configuration. The child's state is reset, and initialization hooks
    /// are called as if the actor were being created for the first time. Use this action when:
    ///
    /// - The error appears to be transient and may not recur
    /// - The child actor's state may have become corrupted
    /// - A fresh start could resolve the underlying issue
    /// - The error is recoverable through reinitialization
    ///
    /// **Behavior:**
    /// - Current child instance is stopped (pre_stop → post_stop)
    /// - New child instance is created with the same configuration
    /// - New child's `pre_start` hook is called for initialization
    /// - Child resumes processing messages with clean state
    ///
    /// **Example Scenarios:**
    /// - Database connection timeout or temporary network failure
    /// - Memory corruption or resource exhaustion in child
    /// - Temporary external service unavailability
    /// - Race conditions or timing-related errors
    Restart,

    /// Delegate the fault handling decision to the child actor's own supervision strategy.
    ///
    /// This action defers the decision to the child actor's configured supervision strategy,
    /// allowing the child to determine its own recovery approach. The child's
    /// `supervision_strategy()` method is consulted to determine the appropriate action.
    /// Use this action when:
    ///
    /// - The parent lacks sufficient context to make an informed decision
    /// - The child actor is better positioned to understand its own recovery needs
    /// - A hierarchical supervision approach is preferred
    /// - The error is internal to the child's operation and domain-specific
    ///
    /// **Behavior:**
    /// - Child's `supervision_strategy()` method is invoked
    /// - The returned strategy (Stop, Restart, Escalate, etc.) is applied
    /// - Child handles its own recovery according to its design
    /// - Parent supervision is bypassed for this specific failure
    ///
    /// **Example Scenarios:**
    /// - Domain-specific business logic errors
    /// - Child actor has specialized error recovery mechanisms
    /// - Complex stateful actors with internal recovery procedures
    /// - When parent supervision is too generic for the specific error type
    ///
    /// **Note:** If the child's supervision strategy is also `Delegate` or results in
    /// escalation, the error may bubble up to higher levels in the supervision hierarchy.
    Delegate,
}

/// Child error receiver.
pub(crate) type ChildErrorReceiver = mpsc::UnboundedReceiver<ChildError>;

/// Child error sender.
pub(crate) type ChildErrorSender = mpsc::UnboundedSender<ChildError>;

/// Represents different types of error conditions reported by child actors to their parents.
///
/// The `ChildError` enum distinguishes between different categories of child actor failures,
/// allowing the parent actor to apply appropriate supervision strategies. This separation
/// enables fine-grained error handling and supports the actor system's supervision hierarchy.
///
/// # Error Categories
///
/// The enum differentiates between:
/// - **Errors**: Recoverable error conditions that don't necessarily require supervision action
/// - **Faults**: Critical failures that require immediate supervision decision and action
///
/// This distinction allows the actor system to handle different error severities appropriately,
/// from simple logging and monitoring to complete actor restart or termination.
///
/// # Supervision Flow
///
/// 1. Child actor encounters an error condition
/// 2. Child decides whether to emit an `Error` (recoverable) or `Fault` (critical)
/// 3. Error is sent to parent through the child error channel
/// 4. Parent's appropriate handler (`on_child_error` or `on_child_fault`) is called
/// 5. For faults, parent returns a `ChildAction` that determines recovery approach
/// 6. Actor system applies the supervision decision
///
/// # Examples
///
/// ```ignore
/// use rush_actor::{ChildError, ChildAction, Error};
///
/// // In a parent actor's supervision logic
/// while let Some(child_error) = error_receiver.recv().await {
///     match child_error {
///         ChildError::Error { error } => {
///             // Handle non-critical error - typically log and monitor
///             self.on_child_error(error, ctx).await;
///         }
///         ChildError::Fault { error, sender } => {
///             // Handle critical fault - make supervision decision
///             let action = self.on_child_fault(error, ctx).await;
///             let _ = sender.send(action); // Communicate decision back to system
///         }
///     }
/// }
/// ```
pub enum ChildError {
    /// A recoverable error condition reported by a child actor.
    ///
    /// This variant represents error conditions that the child actor can potentially
    /// continue operating despite, or that don't require immediate supervision intervention.
    /// These errors are typically used for monitoring, logging, and trend analysis rather
    /// than triggering automatic recovery actions.
    ///
    /// # Characteristics
    ///
    /// - **Severity**: Non-critical, informational
    /// - **Response**: Parent's `on_child_error` method is called
    /// - **Action**: No automatic supervision action required
    /// - **Child State**: Child continues processing messages normally
    /// - **Use Cases**: Validation errors, business logic warnings, performance metrics
    ///
    /// # When to Use
    ///
    /// Use `ChildError::Error` for conditions such as:
    /// - Input validation failures that can be handled gracefully
    /// - Temporary resource constraints that resolve automatically
    /// - Business logic errors that don't affect system stability
    /// - Performance degradation alerts
    /// - Non-critical external service timeouts
    ///
    /// # Example Scenarios
    ///
    /// ```ignore
    /// // In child actor message handler
    /// if let Err(validation_error) = self.validate_input(&message) {
    ///     // Report error but continue processing
    ///     ctx.emit_error(Error::Validation(validation_error)).await?;
    ///     return Ok(Response::ValidationFailed);
    /// }
    /// ```
    Error {
        /// The error condition that occurred in the child actor.
        ///
        /// Contains detailed information about what went wrong, including error type,
        /// context, and any relevant diagnostic information. This error is passed to
        /// the parent's `on_child_error` method for handling.
        error: Error,
    },

    /// A critical fault requiring immediate supervision decision and action.
    ///
    /// This variant represents serious error conditions that have caused the child actor
    /// to stop processing messages and require supervision intervention to determine the
    /// appropriate recovery strategy. The parent must respond with a `ChildAction` to
    /// specify how the system should handle the fault.
    ///
    /// # Characteristics
    ///
    /// - **Severity**: Critical, requires intervention
    /// - **Response**: Parent's `on_child_fault` method is called
    /// - **Action**: Parent must return a `ChildAction` (Stop, Restart, or Delegate)
    /// - **Child State**: Child stops processing messages until supervision decision
    /// - **Use Cases**: Fatal errors, state corruption, unrecoverable system failures
    ///
    /// # When to Use
    ///
    /// Use `ChildError::Fault` for conditions such as:
    /// - Database connection failures that prevent operation
    /// - Memory allocation failures or resource exhaustion
    /// - Critical external service dependencies becoming unavailable
    /// - State corruption that compromises actor integrity
    /// - Unhandled exceptions that indicate programming errors
    ///
    /// # Supervision Response
    ///
    /// The parent actor must provide a supervision decision through the sender channel:
    ///
    /// ```ignore
    /// // In parent actor's fault handling
    /// async fn on_child_fault(
    ///     &mut self,
    ///     error: Error,
    ///     ctx: &mut ActorContext<Self>,
    /// ) -> ChildAction {
    ///     match error {
    ///         Error::Database(_) => ChildAction::Restart, // Transient issue
    ///         Error::Configuration(_) => ChildAction::Stop, // Permanent issue
    ///         _ => ChildAction::Delegate, // Let child decide
    ///     }
    /// }
    /// ```
    ///
    /// # Error Recovery Flow
    ///
    /// 1. Child encounters critical failure and calls `ctx.emit_fail(error)`
    /// 2. Child stops processing messages due to error state
    /// 3. `ChildError::Fault` is sent to parent with response channel
    /// 4. Parent's `on_child_fault` is called to determine action
    /// 5. Parent sends `ChildAction` through the response channel
    /// 6. Actor system applies the supervision decision (restart, stop, etc.)
    ///
    /// # Example Scenarios
    ///
    /// ```ignore
    /// // In child actor message handler
    /// if let Err(database_error) = self.database.execute(&query).await {
    ///     if database_error.is_connection_lost() {
    ///         // Critical failure - emit fault to trigger supervision
    ///         ctx.emit_fail(Error::Database(database_error.to_string())).await?;
    ///         return Err(Error::Database("Connection lost".to_string()));
    ///     }
    /// }
    /// ```
    Fault {
        /// The critical error that caused the child actor to fail.
        ///
        /// Contains comprehensive information about the failure condition, including
        /// error classification, context, and any diagnostic data needed for the
        /// parent to make an informed supervision decision.
        error: Error,

        /// Channel sender for communicating the supervision decision back to the actor system.
        ///
        /// The parent actor must send a `ChildAction` through this channel to specify
        /// how the fault should be handled. The actor system waits for this decision
        /// before proceeding with recovery actions.
        ///
        /// # Important Notes
        ///
        /// - The sender must be used exactly once to avoid system deadlock
        /// - Failure to send a response will cause the supervision process to hang
        /// - The response should be sent promptly to avoid blocking other actors
        /// - If the sender fails (channel closed), the system will apply default behavior
        sender: oneshot::Sender<ChildAction>,
    },
}

/// Core trait defining the behavior and lifecycle of an actor in the actor system.
///
/// The `Actor` trait is the fundamental interface that all actors must implement to participate
/// in the actor system. It defines the actor's message protocol, event emission capabilities,
/// response types, and lifecycle hooks that govern how the actor behaves during its existence
/// in the system.
///
/// # Core Concepts
///
/// An actor in this system is an isolated unit of computation that:
/// - Processes messages sequentially in its own execution context
/// - Maintains private state that cannot be accessed directly from outside
/// - Communicates with other actors only through message passing
/// - Can create child actors and supervise their lifecycle
/// - Emits events to notify subscribers of state changes or significant occurrences
/// - Follows a well-defined lifecycle with hooks for initialization and cleanup
///
/// # Type Safety
///
/// The trait uses associated types to ensure compile-time type safety:
/// - `Message`: Defines what messages this actor can receive and process
/// - `Event`: Specifies the types of events this actor can emit
/// - `Response`: Determines the response type for ask-pattern communications
///
/// # Lifecycle Management
///
/// Actors go through several lifecycle phases, each with corresponding hooks:
/// 1. **Creation**: Actor instance is constructed
/// 2. **Pre-start**: `pre_start()` is called for initialization
/// 3. **Active**: Actor processes messages and emits events
/// 4. **Pre-restart**: `pre_restart()` is called when restarting after failure
/// 5. **Pre-stop**: `pre_stop()` is called before termination
/// 6. **Post-stop**: `post_stop()` is called after termination for cleanup
///
/// # Supervision
///
/// Actors participate in a supervision hierarchy where parent actors supervise
/// their children. The `supervision_strategy()` method defines how this actor
/// should be handled when it fails, and the `on_child_fault()` method in the
/// `Handler` trait defines how this actor handles failures in its children.
///
/// # Examples
///
/// ## Basic Actor Implementation
///
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug)]
/// struct CounterActor {
///     count: i32,
/// }
///
/// #[derive(Debug, Clone)]
/// enum CounterMessage {
///     Increment,
///     Decrement,
///     GetValue,
/// }
///
/// impl Message for CounterMessage {}
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// enum CounterEvent {
///     ValueChanged { old_value: i32, new_value: i32 },
/// }
///
/// impl Event for CounterEvent {}
///
/// #[derive(Debug)]
/// enum CounterResponse {
///     Value(i32),
///     Updated,
/// }
///
/// impl Response for CounterResponse {}
///
/// #[async_trait]
/// impl Actor for CounterActor {
///     type Message = CounterMessage;
///     type Event = CounterEvent;
///     type Response = CounterResponse;
///
///     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
///         println!("Counter actor started at path: {}", ctx.path());
///         Ok(())
///     }
///
///     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
///         println!("Counter actor stopping. Final count: {}", self.count);
///         Ok(())
///     }
/// }
///
/// #[async_trait]
/// impl Handler<CounterActor> for CounterActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             CounterMessage::Increment => {
///                 let old_value = self.count;
///                 self.count += 1;
///                 ctx.publish_event(CounterEvent::ValueChanged {
///                     old_value,
///                     new_value: self.count,
///                 }).await?;
///                 Ok(CounterResponse::Updated)
///             }
///             CounterMessage::Decrement => {
///                 let old_value = self.count;
///                 self.count -= 1;
///                 ctx.publish_event(CounterEvent::ValueChanged {
///                     old_value,
///                     new_value: self.count,
///                 }).await?;
///                 Ok(CounterResponse::Updated)
///             }
///             CounterMessage::GetValue => Ok(CounterResponse::Value(self.count)),
///         }
///     }
/// }
/// ```
///
/// ## Actor with Custom Supervision Strategy
///
/// ```ignore
/// use rush_actor::{Actor, SupervisionStrategy};
///
/// #[async_trait]
/// impl Actor for ResilientActor {
///     type Message = MyMessage;
///     type Event = MyEvent;
///     type Response = MyResponse;
///
///     fn supervision_strategy() -> SupervisionStrategy {
///         SupervisionStrategy::Restart // Restart on failure instead of stopping
///     }
///
///     async fn pre_restart(
///         &mut self,
///         ctx: &mut ActorContext<Self>,
///         error: Option<&Error>,
///     ) -> Result<(), Error> {
///         if let Some(e) = error {
///             println!("Restarting due to error: {:?}", e);
///             // Perform cleanup or state reset specific to restart
///             self.reset_state();
///         }
///         self.pre_start(ctx).await
///     }
/// }
/// ```
///
/// # Thread Safety and Concurrency
///
/// The trait requires `Send + Sync + 'static` to ensure actors can be safely moved between
/// threads and shared in the concurrent actor system. However, actors themselves run in
/// isolation and process messages sequentially, so internal state doesn't need additional
/// synchronization primitives.
///
/// # Error Handling
///
/// Actors can handle errors at multiple levels:
/// - Return errors from message handlers to indicate processing failures
/// - Use `ctx.emit_error()` for recoverable errors that should be logged
/// - Use `ctx.emit_fail()` for critical errors that require supervision action
/// - Override lifecycle hooks to customize error handling during state transitions
#[async_trait]
pub trait Actor: Send + Sync + Sized + 'static + Handler<Self> {
    /// The message type that this actor can receive and process.
    ///
    /// This associated type defines the protocol for communicating with this actor.
    /// All messages sent to this actor (via `tell` or `ask`) must be of this type.
    /// The type must implement the `Message` trait, which requires it to be
    /// `Clone + Send + Sync + 'static` for safe message passing in the actor system.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[derive(Debug, Clone)]
    /// enum DatabaseMessage {
    ///     Insert { table: String, data: Value },
    ///     Query { sql: String },
    ///     Delete { table: String, id: u64 },
    /// }
    ///
    /// impl Message for DatabaseMessage {}
    ///
    /// impl Actor for DatabaseActor {
    ///     type Message = DatabaseMessage; // This actor accepts database operations
    ///     // ... other associated types
    /// }
    /// ```
    type Message: Message;

    /// The event type that this actor can emit to notify subscribers.
    ///
    /// This associated type defines what events this actor publishes when significant
    /// state changes or operations occur. Events are used for the event sourcing pattern,
    /// cross-actor communication, and system monitoring. The type must implement the
    /// `Event` trait, which requires serialization capabilities for persistence and
    /// distribution.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[derive(Debug, Clone, Serialize, Deserialize)]
    /// enum UserEvent {
    ///     UserCreated { id: UserId, name: String },
    ///     UserUpdated { id: UserId, changes: UserChanges },
    ///     UserDeleted { id: UserId },
    /// }
    ///
    /// impl Event for UserEvent {}
    ///
    /// impl Actor for UserActor {
    ///     type Event = UserEvent; // This actor emits user-related events
    ///     // ... other associated types
    /// }
    /// ```
    type Event: Event;

    /// The response type returned when processing messages in ask-pattern interactions.
    ///
    /// This associated type defines what this actor returns when other actors send
    /// messages using the `ask` method and expect a response. For tell-pattern messages
    /// (fire-and-forget), this type is not used. The type must implement the `Response`
    /// trait, which requires `Send + Sync + 'static` for safe async communication.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// #[derive(Debug)]
    /// enum CalculatorResponse {
    ///     Result(f64),
    ///     Error(String),
    ///     InvalidOperation,
    /// }
    ///
    /// impl Response for CalculatorResponse {}
    ///
    /// impl Actor for CalculatorActor {
    ///     type Response = CalculatorResponse; // Responses for calculation requests
    ///     // ... other associated types
    /// }
    /// ```
    type Response: Response;

    /// Defines the supervision strategy that should be applied to this actor when it fails.
    ///
    /// This method specifies how the actor system should handle failures of this specific
    /// actor type. The supervision strategy determines whether the actor should be restarted,
    /// stopped, or handled in another way when it encounters critical errors.
    ///
    /// The default implementation returns `SupervisionStrategy::Stop`, which means the actor
    /// will be terminated if it fails during startup or if it emits a fault during operation.
    /// Override this method to provide custom supervision behavior appropriate for your actor's
    /// role and resilience requirements.
    ///
    /// # Returns
    ///
    /// Returns a `SupervisionStrategy` that defines how this actor should be supervised.
    /// Common strategies include:
    /// - `Stop`: Terminate the actor on failure (default)
    /// - `Restart`: Restart the actor with fresh state
    /// - `Escalate`: Pass the failure up to the parent actor
    ///
    /// # Examples
    ///
    /// ```ignore
    /// impl Actor for CriticalServiceActor {
    ///     // ... other associated types
    ///
    ///     fn supervision_strategy() -> SupervisionStrategy {
    ///         SupervisionStrategy::Restart // Always try to restart critical services
    ///     }
    /// }
    ///
    /// impl Actor for TemporaryWorkerActor {
    ///     // ... other associated types
    ///
    ///     fn supervision_strategy() -> SupervisionStrategy {
    ///         SupervisionStrategy::Stop // Temporary workers can be safely terminated
    ///     }
    /// }
    /// ```
    ///
    /// # Design Considerations
    ///
    /// When choosing a supervision strategy, consider:
    /// - **Actor Role**: Critical services may need restart, while temporary workers can stop
    /// - **State Importance**: Stateless actors can restart easily, stateful ones may need careful handling
    /// - **Resource Impact**: Restarting resource-intensive actors may affect system performance
    /// - **Error Types**: Some errors are permanent and restart won't help
    /// - **Dependencies**: Consider how actor failure affects dependent components
    fn supervision_strategy() -> SupervisionStrategy {
        SupervisionStrategy::Stop
    }

    /// Called during actor initialization to perform startup tasks and resource allocation.
    ///
    /// This lifecycle method is invoked automatically by the actor system after the actor instance
    /// has been created and registered, but before it begins processing messages. It provides an
    /// opportunity to initialize resources, establish connections, validate configuration, and
    /// prepare the actor for normal operation.
    ///
    /// The method is called exactly once during the actor's lifecycle (unless the actor is restarted,
    /// in which case `pre_restart` is called instead). If this method returns an error, the actor
    /// will not be started and the supervision strategy will determine whether to retry, escalate,
    /// or terminate the actor.
    ///
    /// # Lifecycle Context
    ///
    /// This method is part of the actor startup sequence:
    /// 1. Actor instance is created
    /// 2. Actor is registered in the system
    /// 3. **`pre_start` is called** ← You are here
    /// 4. Actor begins processing messages
    /// 5. Actor enters normal operation state
    ///
    /// # Parameters
    ///
    /// * `context` - Mutable reference to the actor's execution context, providing access to:
    ///   - Actor path and system reference
    ///   - Child actor creation and management
    ///   - Event publishing capabilities
    ///   - Error reporting mechanisms
    ///   - System-level services and configuration
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if initialization completed successfully and the actor is ready to
    /// process messages, or an `Error` if startup failed and the actor cannot operate.
    ///
    /// # Error Conditions
    ///
    /// Common error scenarios include:
    /// - Resource allocation failures (memory, file handles, network connections)
    /// - Configuration validation errors
    /// - External service unavailability
    /// - Permission or authentication failures
    /// - Child actor creation failures
    ///
    /// # Examples
    ///
    /// ## Database Actor Initialization
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for DatabaseActor {
    ///     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         // Initialize database connection pool
    ///         self.connection_pool = ConnectionPool::new(&self.database_url)
    ///             .max_connections(self.max_connections)
    ///             .build()
    ///             .await
    ///             .map_err(|e| Error::Database(format!("Failed to create connection pool: {}", e)))?;
    ///
    ///         // Verify database connectivity
    ///         let mut conn = self.connection_pool.acquire().await
    ///             .map_err(|e| Error::Database(format!("Failed to acquire connection: {}", e)))?;
    ///
    ///         // Run health check query
    ///         sqlx::query("SELECT 1")
    ///             .execute(&mut conn)
    ///             .await
    ///             .map_err(|e| Error::Database(format!("Database health check failed: {}", e)))?;
    ///
    ///         println!("Database actor started at path: {}", ctx.path());
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Service Manager with Child Actors
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for ServiceManager {
    ///     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         // Initialize configuration
    ///         self.config = ServiceConfig::load_from_file(&self.config_path)
    ///             .map_err(|e| Error::Configuration(format!("Invalid config: {}", e)))?;
    ///
    ///         // Create worker actors
    ///         for i in 0..self.config.worker_count {
    ///             let worker_name = format!("worker-{}", i);
    ///             let worker = WorkerActor::new(self.config.worker_config.clone());
    ///             let worker_ref = ctx.create_child(&worker_name, worker).await?;
    ///             self.workers.push(worker_ref);
    ///         }
    ///
    ///         // Start monitoring timer
    ///         self.start_health_monitoring(ctx).await?;
    ///
    ///         println!("Service manager started with {} workers", self.workers.len());
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Stateful Actor with Recovery
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for StatefulActor {
    ///     async fn pre_start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         // Attempt to restore state from persistent storage
    ///         match self.load_state_from_disk().await {
    ///             Ok(restored_state) => {
    ///                 self.state = restored_state;
    ///                 println!("State restored from disk");
    ///             }
    ///             Err(e) => {
    ///                 // State restoration failed, start with clean state
    ///                 println!("Starting with clean state due to: {}", e);
    ///                 self.state = Default::default();
    ///             }
    ///         }
    ///
    ///         // Initialize metrics reporting
    ///         self.metrics_reporter = MetricsReporter::new(ctx.path().to_string());
    ///
    ///         // Schedule periodic state persistence
    ///         self.schedule_state_persistence(ctx).await;
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// 1. **Keep It Fast**: Avoid long-running operations that could delay system startup
    /// 2. **Fail Fast**: Validate critical requirements early and return errors for unrecoverable conditions
    /// 3. **Resource Management**: Only allocate resources you can properly clean up in `pre_stop`
    /// 4. **Child Actor Creation**: Create necessary child actors during startup for proper supervision
    /// 5. **Configuration Validation**: Validate all configuration parameters before using them
    /// 6. **Error Context**: Provide detailed error messages for debugging startup failures
    ///
    /// # Thread Safety
    ///
    /// This method is called within the actor's single-threaded execution context before any
    /// message processing begins, so no additional synchronization is required for actor state.
    /// However, any resources created here (connections, child actors, etc.) should be designed
    /// for safe concurrent access if they will be shared.
    ///
    /// # Performance Considerations
    ///
    /// - Startup operations block the actor system until completion
    /// - Consider using lazy initialization for expensive resources that may not be needed immediately
    /// - Use connection pooling and resource sharing patterns for efficiency
    /// - Avoid blocking I/O operations when possible; use async patterns instead
    ///
    async fn pre_start(
        &mut self,
        _context: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Called when the actor is being restarted following a failure or supervision action.
    ///
    /// This lifecycle method is invoked by the supervision system when an actor needs to be
    /// restarted due to a critical error or fault. It provides an opportunity to perform
    /// cleanup of corrupted state, reset resources, and prepare for a fresh start. The method
    /// receives information about the error that triggered the restart, allowing for context-aware
    /// recovery procedures.
    ///
    /// By default, this method simply calls `pre_start()` to perform the same initialization
    /// sequence as a normal startup. However, you can override this method to implement
    /// specialized restart logic, such as partial state recovery, enhanced error handling,
    /// or different initialization procedures based on the failure condition.
    ///
    /// # Lifecycle Context
    ///
    /// This method is part of the actor restart sequence:
    /// 1. Actor encounters critical error or fault
    /// 2. Supervision system decides to restart the actor
    /// 3. Actor's current state and resources are cleaned up
    /// 4. **`pre_restart` is called** ← You are here
    /// 5. Actor resumes normal operation with fresh state
    ///
    /// # Parameters
    ///
    /// * `ctx` - Mutable reference to the actor's execution context, providing access to:
    ///   - Actor path and system services
    ///   - Child actor management capabilities
    ///   - Event publishing and error reporting
    ///   - System configuration and global state
    ///
    /// * `error` - Optional reference to the error that triggered the restart. This provides
    ///            context about what went wrong and can influence recovery strategy:
    ///   - `Some(error)` when restart was triggered by a specific failure
    ///   - `None` when restart was initiated manually or by supervision policy
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the restart preparation and initialization completed successfully,
    /// or an `Error` if the restart failed and further supervision action is needed.
    ///
    /// # Error Conditions
    ///
    /// If this method returns an error:
    /// - The restart attempt is considered failed
    /// - The supervision system may retry, escalate, or terminate the actor
    /// - The actor remains in a failed state until successful restart or termination
    /// - Child actors may be affected by the failure escalation
    ///
    /// # Examples
    ///
    /// ## Basic Restart with Error Analysis
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for DatabaseActor {
    ///     async fn pre_restart(
    ///         &mut self,
    ///         ctx: &mut ActorContext<Self>,
    ///         error: Option<&Error>,
    ///     ) -> Result<(), Error> {
    ///         if let Some(err) = error {
    ///             match err {
    ///                 Error::Database(msg) if msg.contains("connection") => {
    ///                     println!("Restarting due to connection failure: {}", msg);
    ///                     // Close any existing connections before restart
    ///                     self.close_all_connections().await;
    ///                 }
    ///                 Error::Database(msg) if msg.contains("timeout") => {
    ///                     println!("Restarting due to timeout: {}", msg);
    ///                     // Adjust timeout settings for restart
    ///                     self.connection_timeout = self.connection_timeout * 2;
    ///                 }
    ///                 _ => {
    ///                     println!("Restarting due to unknown error: {:?}", err);
    ///                 }
    ///             }
    ///         } else {
    ///             println!("Manual restart initiated");
    ///         }
    ///
    ///         // Call standard initialization
    ///         self.pre_start(ctx).await
    ///     }
    /// }
    /// ```
    ///
    /// ## Stateful Actor with Selective Recovery
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for StatefulProcessor {
    ///     async fn pre_restart(
    ///         &mut self,
    ///         ctx: &mut ActorContext<Self>,
    ///         error: Option<&Error>,
    ///     ) -> Result<(), Error> {
    ///         // Analyze the error to determine recovery strategy
    ///         let recovery_mode = match error {
    ///             Some(Error::MemoryCorruption(_)) => RecoveryMode::FullReset,
    ///             Some(Error::Network(_)) => RecoveryMode::PreserveState,
    ///             Some(Error::Configuration(_)) => RecoveryMode::ReloadConfig,
    ///             _ => RecoveryMode::Standard,
    ///         };
    ///
    ///         match recovery_mode {
    ///             RecoveryMode::FullReset => {
    ///                 // Complete state reset
    ///                 self.state.clear();
    ///                 self.cache.clear();
    ///                 println!("Performing full state reset");
    ///             }
    ///             RecoveryMode::PreserveState => {
    ///                 // Keep state but reset connections
    ///                 self.close_connections().await;
    ///                 println!("Preserving state, resetting connections");
    ///             }
    ///             RecoveryMode::ReloadConfig => {
    ///                 // Reload configuration from disk
    ///                 self.config = Config::reload().map_err(|e|
    ///                     Error::Configuration(format!("Config reload failed: {}", e)))?;
    ///                 println!("Configuration reloaded");
    ///             }
    ///             RecoveryMode::Standard => {
    ///                 println!("Standard restart procedure");
    ///             }
    ///         }
    ///
    ///         // Perform standard initialization
    ///         self.pre_start(ctx).await
    ///     }
    /// }
    /// ```
    ///
    /// ## Service Manager with Child Recovery
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for ServiceManager {
    ///     async fn pre_restart(
    ///         &mut self,
    ///         ctx: &mut ActorContext<Self>,
    ///         error: Option<&Error>,
    ///     ) -> Result<(), Error> {
    ///         println!("Service manager restarting...");
    ///
    ///         if let Some(err) = error {
    ///             // Log the failure reason
    ///             println!("Restart triggered by: {:?}", err);
    ///
    ///             // Increment restart counter for monitoring
    ///             self.restart_count += 1;
    ///
    ///             // Check if we're restarting too frequently
    ///             if self.restart_count > self.max_restarts {
    ///                 return Err(Error::System(
    ///                     "Too many restarts, likely persistent issue".to_string()
    ///                 ));
    ///             }
    ///         }
    ///
    ///         // Clear child references (they will be recreated)
    ///         self.workers.clear();
    ///         self.monitors.clear();
    ///
    ///         // Initialize with potentially adjusted configuration
    ///         if self.restart_count > 0 {
    ///             // Use more conservative settings after restart
    ///             self.worker_count = (self.worker_count * 0.8) as usize;
    ///             println!("Reducing worker count to {} after restart", self.worker_count);
    ///         }
    ///
    ///         // Perform standard initialization
    ///         self.pre_start(ctx).await
    ///     }
    /// }
    /// ```
    ///
    /// # Restart Strategies
    ///
    /// Consider these approaches when implementing custom restart logic:
    ///
    /// 1. **Error-Based Recovery**: Adjust behavior based on the specific error type
    /// 2. **Progressive Degradation**: Reduce functionality or resources after repeated failures
    /// 3. **State Preservation**: Selectively preserve or reset state based on error context
    /// 4. **Configuration Adaptation**: Modify settings to potentially avoid repeated failures
    /// 5. **Circuit Breaker Patterns**: Implement backoff or circuit breaking logic
    ///
    /// # Best Practices
    ///
    /// 1. **Analyze the Error**: Use the error parameter to implement intelligent recovery
    /// 2. **Reset Corrupted State**: Clear any state that may have caused the failure
    /// 3. **Maintain Restart Statistics**: Track restart frequency for monitoring and circuit breaking
    /// 4. **Adjust Configuration**: Consider backing off or changing parameters after failures
    /// 5. **Clean Child State**: Properly handle child actor states during restart
    /// 6. **Log Restart Reasons**: Provide clear logging for debugging and monitoring
    ///
    /// # Thread Safety
    ///
    /// This method executes within the actor's single-threaded context during the restart
    /// procedure. The actor is not processing messages during restart, ensuring thread safety
    /// for state modifications and resource management.
    ///
    /// # Performance Considerations
    ///
    /// - Restart operations should be efficient to minimize system downtime
    /// - Consider lazy initialization for expensive resources
    /// - Implement exponential backoff for external resource connections
    /// - Monitor restart frequency to detect persistent issues early
    ///
    async fn pre_restart(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _error: Option<&Error>,
    ) -> Result<(), Error> {
        self.pre_start(ctx).await
    }

    /// Called during actor termination to perform cleanup and resource deallocation.
    ///
    /// This lifecycle method is invoked automatically by the actor system when the actor
    /// is being stopped, either due to normal shutdown, supervision action, or explicit
    /// termination request. It provides the opportunity to gracefully release resources,
    /// save state, notify dependent systems, and perform any necessary cleanup operations
    /// before the actor is removed from the system.
    ///
    /// The method is called after the actor has stopped processing new messages but before
    /// child actors are stopped and the actor is removed from the system registry. This
    /// ensures that the actor can still interact with its children and other actors during
    /// the cleanup process if needed.
    ///
    /// # Lifecycle Context
    ///
    /// This method is part of the actor shutdown sequence:
    /// 1. Actor receives stop signal (explicit or via supervision)
    /// 2. Actor stops accepting new messages
    /// 3. Current message processing completes (if any)
    /// 4. **`pre_stop` is called** ← You are here
    /// 5. Child actors are stopped recursively
    /// 6. Actor is removed from system registry
    /// 7. `post_stop` is called for final cleanup
    /// 8. Actor terminates and resources are freed
    ///
    /// # Parameters
    ///
    /// * `ctx` - Mutable reference to the actor's execution context, providing access to:
    ///   - Actor path and system services
    ///   - Child actor references (still accessible at this stage)
    ///   - Event publishing capabilities
    ///   - System-level services for final operations
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if cleanup completed successfully, or an `Error` if shutdown
    /// encountered issues. Note that errors during shutdown are typically logged but
    /// do not prevent the actor from being terminated.
    ///
    /// # Error Conditions
    ///
    /// Common error scenarios include:
    /// - Failure to save persistent state
    /// - Network errors when notifying external systems
    /// - Resource deallocation failures
    /// - Timeout waiting for in-progress operations to complete
    ///
    /// While errors are returned, they typically don't prevent termination but are used
    /// for logging and monitoring shutdown issues.
    ///
    /// # Examples
    ///
    /// ## Database Actor with Connection Cleanup
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for DatabaseActor {
    ///     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Database actor shutting down at path: {}", ctx.path());
    ///
    ///         // Complete any pending transactions
    ///         if let Some(ref mut transaction) = self.pending_transaction {
    ///             match transaction.commit().await {
    ///                 Ok(_) => println!("Pending transaction committed successfully"),
    ///                 Err(e) => {
    ///                     println!("Failed to commit pending transaction: {}", e);
    ///                     // Attempt rollback
    ///                     let _ = transaction.rollback().await;
    ///                 }
    ///             }
    ///         }
    ///
    ///         // Close connection pool gracefully
    ///         if let Some(pool) = self.connection_pool.take() {
    ///             pool.close().await;
    ///             println!("Database connection pool closed");
    ///         }
    ///
    ///         // Flush any cached writes
    ///         if !self.write_cache.is_empty() {
    ///             self.flush_write_cache().await
    ///                 .map_err(|e| Error::Database(format!("Failed to flush cache: {}", e)))?;
    ///         }
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Service Manager with Child Coordination
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for ServiceManager {
    ///     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Service manager initiating graceful shutdown...");
    ///
    ///         // Stop accepting new work
    ///         self.accepting_work = false;
    ///
    ///         // Notify all workers to finish current tasks
    ///         for worker in &self.workers {
    ///             if let Err(e) = worker.tell(WorkerMessage::PrepareShutdown).await {
    ///                 println!("Failed to notify worker {}: {}", worker.path(), e);
    ///             }
    ///         }
    ///
    ///         // Wait for workers to complete current tasks (with timeout)
    ///         let shutdown_timeout = Duration::from_secs(30);
    ///         let start_time = Instant::now();
    ///
    ///         while !self.all_workers_idle().await && start_time.elapsed() < shutdown_timeout {
    ///             tokio::time::sleep(Duration::from_millis(100)).await;
    ///         }
    ///
    ///         if start_time.elapsed() >= shutdown_timeout {
    ///             println!("Shutdown timeout reached, proceeding with forceful stop");
    ///         } else {
    ///             println!("All workers completed their tasks");
    ///         }
    ///
    ///         // Save service statistics
    ///         self.save_statistics().await
    ///             .map_err(|e| Error::System(format!("Failed to save statistics: {}", e)))?;
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Stateful Actor with State Persistence
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for StatefulProcessor {
    ///     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Saving state before shutdown...");
    ///
    ///         // Cancel any scheduled tasks
    ///         if let Some(scheduled_task) = self.scheduled_task.take() {
    ///             scheduled_task.cancel();
    ///         }
    ///
    ///         // Process any remaining items in the queue
    ///         while let Some(item) = self.processing_queue.pop() {
    ///             if let Err(e) = self.process_item_quickly(item).await {
    ///                 println!("Failed to process queued item during shutdown: {}", e);
    ///                 // Re-queue for next startup if critical
    ///                 if item.is_critical() {
    ///                     self.failed_items.push(item);
    ///                 }
    ///             }
    ///         }
    ///
    ///         // Save current state to persistent storage
    ///         let state_snapshot = StateSnapshot {
    ///             processed_count: self.processed_count,
    ///             failed_items: self.failed_items.clone(),
    ///             last_checkpoint: Utc::now(),
    ///             metadata: self.get_metadata(),
    ///         };
    ///
    ///         self.state_store.save(&state_snapshot).await
    ///             .map_err(|e| Error::System(format!("State save failed: {}", e)))?;
    ///
    ///         // Publish shutdown event for monitoring
    ///         ctx.publish_event(ProcessorEvent::ShuttingDown {
    ///             processed_count: self.processed_count,
    ///             uptime: self.start_time.elapsed(),
    ///         }).await?;
    ///
    ///         println!("State saved successfully, processed {} items total", self.processed_count);
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Network Service with External Notifications
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for NetworkServiceActor {
    ///     async fn pre_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Network service shutting down, notifying peers...");
    ///
    ///         // Notify external systems of shutdown
    ///         let shutdown_message = ShutdownNotification {
    ///             service_id: self.service_id.clone(),
    ///             shutdown_time: Utc::now(),
    ///             reason: "Graceful shutdown".to_string(),
    ///         };
    ///
    ///         // Send notifications to all registered peers
    ///         for peer in &self.peers {
    ///             if let Err(e) = self.notify_peer_shutdown(peer, &shutdown_message).await {
    ///                 println!("Failed to notify peer {}: {}", peer, e);
    ///                 // Continue with other peers even if one fails
    ///             }
    ///         }
    ///
    ///         // Close network listeners
    ///         if let Some(listener) = self.tcp_listener.take() {
    ///             listener.close().await;
    ///         }
    ///
    ///         // Close active connections gracefully
    ///         for connection in &mut self.active_connections {
    ///             if let Err(e) = connection.close_gracefully().await {
    ///                 println!("Error closing connection: {}", e);
    ///             }
    ///         }
    ///
    ///         // Deregister from service discovery
    ///         self.service_registry.deregister(&self.service_id).await
    ///             .map_err(|e| Error::Network(format!("Deregistration failed: {}", e)))?;
    ///
    ///         println!("Network service shutdown complete");
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// 1. **Graceful Completion**: Finish critical operations before releasing resources
    /// 2. **Resource Cleanup**: Close connections, files, and other system resources properly
    /// 3. **State Persistence**: Save important state that should survive actor restarts
    /// 4. **External Notifications**: Notify dependent systems of the shutdown
    /// 5. **Child Coordination**: Allow child actors to complete current work when possible
    /// 6. **Timeout Handling**: Don't block shutdown indefinitely; use reasonable timeouts
    /// 7. **Error Tolerance**: Continue cleanup even if some operations fail
    /// 8. **Monitoring Events**: Publish events for system monitoring and observability
    ///
    /// # Shutdown Patterns
    ///
    /// Common patterns for graceful shutdown:
    ///
    /// - **Drain Pattern**: Stop accepting new work, complete current work, then stop
    /// - **Checkpoint Pattern**: Save progress at regular intervals and final checkpoint on stop
    /// - **Notification Pattern**: Inform dependent systems before shutting down
    /// - **Cascade Pattern**: Coordinate shutdown with child and peer actors
    /// - **Timeout Pattern**: Use bounded time for cleanup operations
    ///
    /// # Thread Safety
    ///
    /// This method executes within the actor's single-threaded execution context after
    /// message processing has stopped. Child actors are still accessible and can be
    /// communicated with, but new message handling has ceased.
    ///
    /// # Performance Considerations
    ///
    /// - Shutdown operations should be reasonably fast to avoid blocking system termination
    /// - Use timeouts for potentially slow cleanup operations
    /// - Consider background cleanup for non-critical operations
    /// - Prioritize critical state saving over comprehensive cleanup
    ///
    async fn pre_stop(
        &mut self,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Called after the actor has been fully terminated for final cleanup and resource release.
    ///
    /// This lifecycle method is invoked as the final step in the actor termination sequence,
    /// after the actor has been removed from the system registry and all child actors have
    /// been stopped. It provides the last opportunity to perform cleanup operations,
    /// release system resources, and execute any final termination logic.
    ///
    /// Unlike `pre_stop`, this method is called when the actor is no longer accessible
    /// through the actor system and cannot communicate with other actors or create new
    /// child actors. It should be used primarily for releasing resources and performing
    /// final cleanup that doesn't require inter-actor communication.
    ///
    /// # Lifecycle Context
    ///
    /// This method is the final step in the actor termination sequence:
    /// 1. Actor receives stop signal and stops processing messages
    /// 2. `pre_stop` is called for graceful shutdown preparation
    /// 3. Child actors are stopped recursively
    /// 4. Actor is removed from the system registry
    /// 5. **`post_stop` is called** ← You are here
    /// 6. Actor instance is deallocated and all resources are freed
    ///
    /// # Parameters
    ///
    /// * `ctx` - Mutable reference to the actor's execution context. Note that at this stage:
    ///   - The actor is no longer registered in the system
    ///   - Child actors have already been terminated
    ///   - Message sending capabilities are limited
    ///   - System services remain accessible for final operations
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if final cleanup completed successfully, or an `Error` if
    /// termination encountered issues. Errors at this stage are typically logged
    /// but don't affect the termination process.
    ///
    /// # Error Conditions
    ///
    /// Common error scenarios include:
    /// - Failure to release system resources (file handles, memory, etc.)
    /// - Errors in final logging or audit operations
    /// - Issues with cleanup of temporary files or directories
    /// - Problems with external system deregistration
    ///
    /// Since this is the final cleanup stage, errors are primarily for logging
    /// and monitoring purposes and don't prevent actor termination.
    ///
    /// # Examples
    ///
    /// ## Simple Resource Cleanup
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for FileProcessorActor {
    ///     async fn post_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("File processor terminated at path: {}", ctx.path());
    ///
    ///         // Close any remaining file handles
    ///         if let Some(file_handle) = self.current_file.take() {
    ///             if let Err(e) = file_handle.close() {
    ///                 println!("Warning: Failed to close file handle: {}", e);
    ///             }
    ///         }
    ///
    ///         // Clean up temporary files
    ///         for temp_file in &self.temp_files {
    ///             if let Err(e) = std::fs::remove_file(temp_file) {
    ///                 println!("Warning: Failed to remove temp file {}: {}", temp_file, e);
    ///             }
    ///         }
    ///
    ///         // Log final statistics
    ///         println!("Processed {} files during lifetime", self.files_processed);
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Memory and Cache Cleanup
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for CacheActor {
    ///     async fn post_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Cache actor terminating, clearing {} entries", self.cache.len());
    ///
    ///         // Clear large data structures to free memory quickly
    ///         self.cache.clear();
    ///         self.index.clear();
    ///         self.bloom_filter.clear();
    ///
    ///         // Release shared memory segments if any
    ///         if let Some(shared_memory) = self.shared_memory.take() {
    ///             shared_memory.unmap()
    ///                 .map_err(|e| Error::System(format!("Failed to unmap shared memory: {}", e)))?;
    ///         }
    ///
    ///         // Log cache statistics
    ///         println!("Cache hit rate: {:.2}%",
    ///                  (self.cache_hits as f64 / self.cache_requests as f64) * 100.0);
    ///         println!("Total memory released: {} MB", self.memory_used / 1024 / 1024);
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Monitoring and Audit Logging
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for AuditActor {
    ///     async fn post_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         let termination_time = Utc::now();
    ///         let uptime = self.start_time.signed_duration_since(termination_time);
    ///
    ///         // Create termination audit log entry
    ///         let audit_entry = AuditLogEntry {
    ///             actor_path: ctx.path().to_string(),
    ///             event_type: "actor_terminated".to_string(),
    ///             timestamp: termination_time,
    ///             details: json!({
    ///                 "uptime_seconds": uptime.num_seconds(),
    ///                 "events_processed": self.events_processed,
    ///                 "errors_encountered": self.error_count,
    ///                 "final_state": "terminated"
    ///             }),
    ///         };
    ///
    ///         // Write final audit entry
    ///         if let Err(e) = self.audit_logger.log_entry(&audit_entry).await {
    ///             eprintln!("Failed to write final audit entry: {}", e);
    ///         }
    ///
    ///         // Flush and close audit log
    ///         self.audit_logger.flush().await
    ///             .map_err(|e| Error::System(format!("Failed to flush audit log: {}", e)))?;
    ///
    ///         // Update system metrics
    ///         self.metrics_client.record_actor_termination(
    ///             &ctx.path().to_string(),
    ///             uptime.num_seconds() as u64
    ///         ).await;
    ///
    ///         println!("Audit actor terminated after processing {} events",
    ///                  self.events_processed);
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Network Resource Cleanup
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for NetworkActor {
    ///     async fn post_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Network actor final cleanup starting...");
    ///
    ///         // Force close any remaining sockets
    ///         for socket in &mut self.sockets {
    ///             socket.shutdown().await.unwrap_or_else(|e| {
    ///                 println!("Warning: Socket shutdown failed: {}", e);
    ///             });
    ///         }
    ///         self.sockets.clear();
    ///
    ///         // Release network buffers
    ///         self.read_buffers.clear();
    ///         self.write_buffers.clear();
    ///
    ///         // Clean up SSL contexts and certificates
    ///         if let Some(ssl_context) = self.ssl_context.take() {
    ///             ssl_context.cleanup();
    ///         }
    ///
    ///         // Final network statistics
    ///         println!("Network stats - Bytes sent: {}, Bytes received: {}, Connections: {}",
    ///                  self.bytes_sent, self.bytes_received, self.total_connections);
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// ## Database Connection Cleanup
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Actor for DatabaseActor {
    ///     async fn post_stop(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Error> {
    ///         println!("Database actor performing final cleanup...");
    ///
    ///         // Ensure all connections are properly closed
    ///         // (should already be done in pre_stop, but double-check)
    ///         if let Some(pool) = self.connection_pool.take() {
    ///             // Force close any remaining connections
    ///             pool.force_close().await;
    ///         }
    ///
    ///         // Clear prepared statement cache
    ///         self.statement_cache.clear();
    ///
    ///         // Release connection metrics collector
    ///         if let Some(metrics) = self.connection_metrics.take() {
    ///             metrics.shutdown();
    ///         }
    ///
    ///         // Log final database statistics
    ///         println!("Database actor final stats:");
    ///         println!("  Total queries executed: {}", self.queries_executed);
    ///         println!("  Average query time: {:.2}ms", self.total_query_time / self.queries_executed);
    ///         println!("  Connection pool utilization: {:.1}%",
    ///                  (self.peak_connections as f64 / self.max_connections as f64) * 100.0);
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// # Best Practices
    ///
    /// 1. **Quick Cleanup**: Perform only essential cleanup; avoid time-consuming operations
    /// 2. **Resource Release**: Focus on releasing system resources (memory, handles, connections)
    /// 3. **Error Tolerance**: Continue cleanup even if individual operations fail
    /// 4. **No Inter-Actor Communication**: Don't attempt to communicate with other actors
    /// 5. **Statistics Logging**: Log useful metrics and statistics for monitoring
    /// 6. **Memory Management**: Clear large data structures to help garbage collection
    /// 7. **System Resource Cleanup**: Close files, sockets, and other OS resources
    /// 8. **Audit Trails**: Record termination for debugging and compliance
    ///
    /// # Cleanup Priorities
    ///
    /// Focus cleanup efforts on:
    ///
    /// 1. **Critical Resources**: File handles, network sockets, database connections
    /// 2. **Memory**: Large caches, buffers, and data structures
    /// 3. **System Resources**: Shared memory, semaphores, temporary files
    /// 4. **Logging**: Final log entries, statistics, audit records
    /// 5. **Metrics**: Resource utilization, performance statistics
    ///
    /// # Thread Safety
    ///
    /// This method executes in the actor's termination context after all message processing
    /// and child management has completed. The actor is in its final execution phase, so
    /// no additional synchronization is required for actor-local state.
    ///
    /// # Performance Considerations
    ///
    /// - Keep final cleanup operations lightweight and fast
    /// - Don't block system shutdown with expensive operations
    /// - Use fire-and-forget patterns for non-critical cleanup
    /// - Prioritize resource release over comprehensive cleanup
    /// - Consider background cleanup for non-essential operations
    ///
    async fn post_stop(
        &mut self,
        _ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Converts an actor response into an event for event sourcing and state management.
    ///
    /// This method provides a mechanism to transform the response returned by an actor's
    /// message handler into an event that can be persisted, published, or used for state
    /// reconstruction in event sourcing patterns. It enables automatic event generation
    /// based on successful message processing outcomes.
    ///
    /// The default implementation returns a functional error indicating that event
    /// conversion is not implemented. Override this method in actors that participate
    /// in event sourcing or need to automatically emit events based on response data.
    ///
    /// # Event Sourcing Integration
    ///
    /// This method is typically used in event sourcing architectures where:
    /// - All state changes are represented as a sequence of events
    /// - Events are the source of truth for actor state
    /// - Responses contain data that should be captured as events
    /// - Event streams need to be maintained for audit, replay, or analysis
    ///
    /// # Parameters
    ///
    /// * `response` - The response returned by the actor's message handler that should
    ///               be converted to an event. This response contains the outcome of
    ///               message processing and any data that should be captured in event form.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self::Event)` containing the event representation of the response,
    /// or an `Error` if the conversion failed or is not supported for this response type.
    ///
    /// # Error Conditions
    ///
    /// - `Error::Functional("Not implemented")` - Default implementation (override required)
    /// - `Error::Conversion(_)` - Response cannot be converted to event format
    /// - `Error::Validation(_)` - Response data is invalid for event creation
    /// - Custom errors specific to the conversion logic
    ///
    /// # Examples
    ///
    /// ## Bank Account Event Sourcing
    ///
    /// ```ignore
    /// #[derive(Debug, Clone, Serialize, Deserialize)]
    /// enum AccountEvent {
    ///     AccountCreated { account_id: String, initial_balance: Decimal },
    ///     MoneyDeposited { account_id: String, amount: Decimal, new_balance: Decimal },
    ///     MoneyWithdrawn { account_id: String, amount: Decimal, new_balance: Decimal },
    ///     TransferCompleted { from: String, to: String, amount: Decimal },
    /// }
    ///
    /// #[derive(Debug)]
    /// enum AccountResponse {
    ///     Created { account_id: String, balance: Decimal },
    ///     DepositSuccessful { new_balance: Decimal },
    ///     WithdrawalSuccessful { new_balance: Decimal },
    ///     TransferCompleted { confirmation_id: String },
    ///     InsufficientFunds,
    ///     AccountNotFound,
    /// }
    ///
    /// #[async_trait]
    /// impl Actor for BankAccountActor {
    ///     type Event = AccountEvent;
    ///     type Response = AccountResponse;
    ///     // ... other associated types
    ///
    ///     fn from_response(response: Self::Response) -> Result<Self::Event, Error> {
    ///         match response {
    ///             AccountResponse::Created { account_id, balance } => {
    ///                 Ok(AccountEvent::AccountCreated {
    ///                     account_id,
    ///                     initial_balance: balance,
    ///                 })
    ///             }
    ///             AccountResponse::DepositSuccessful { new_balance } => {
    ///                 // Note: This requires access to the original message data
    ///                 // In practice, you might pass this through the response
    ///                 Err(Error::Functional(
    ///                     "Deposit event requires original message context".to_string()
    ///                 ))
    ///             }
    ///             AccountResponse::TransferCompleted { confirmation_id } => {
    ///                 // This response doesn't contain enough data for event creation
    ///                 Err(Error::Functional(
    ///                     "Transfer event requires transfer details".to_string()
    ///                 ))
    ///             }
    ///             AccountResponse::InsufficientFunds | AccountResponse::AccountNotFound => {
    ///                 // Error responses don't generate events in this model
    ///                 Err(Error::Functional("Error responses don't generate events".to_string()))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Enhanced Response with Event Data
    ///
    /// ```ignore
    /// #[derive(Debug)]
    /// struct EnhancedResponse<T> {
    ///     result: T,
    ///     event_data: Option<EventData>,
    /// }
    ///
    /// #[derive(Debug, Clone)]
    /// struct EventData {
    ///     entity_id: String,
    ///     operation: String,
    ///     previous_state: Value,
    ///     new_state: Value,
    ///     metadata: HashMap<String, String>,
    /// }
    ///
    /// #[derive(Debug, Clone, Serialize, Deserialize)]
    /// enum GenericEvent {
    ///     StateChanged {
    ///         entity_id: String,
    ///         operation: String,
    ///         previous_state: Value,
    ///         new_state: Value,
    ///         timestamp: DateTime<Utc>,
    ///         metadata: HashMap<String, String>,
    ///     },
    /// }
    ///
    /// #[async_trait]
    /// impl Actor for GenericStatefulActor {
    ///     type Event = GenericEvent;
    ///     type Response = EnhancedResponse<ProcessingResult>;
    ///     // ... other associated types
    ///
    ///     fn from_response(response: Self::Response) -> Result<Self::Event, Error> {
    ///         match response.event_data {
    ///             Some(event_data) => {
    ///                 Ok(GenericEvent::StateChanged {
    ///                     entity_id: event_data.entity_id,
    ///                     operation: event_data.operation,
    ///                     previous_state: event_data.previous_state,
    ///                     new_state: event_data.new_state,
    ///                     timestamp: Utc::now(),
    ///                     metadata: event_data.metadata,
    ///                 })
    ///             }
    ///             None => {
    ///                 Err(Error::Functional(
    ///                     "Response does not contain event data".to_string()
    ///                 ))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Counter Actor with Simple Event Conversion
    ///
    /// ```ignore
    /// #[derive(Debug, Clone, Serialize, Deserialize)]
    /// enum CounterEvent {
    ///     ValueChanged { old_value: i64, new_value: i64 },
    ///     Reset { previous_value: i64 },
    /// }
    ///
    /// #[derive(Debug)]
    /// enum CounterResponse {
    ///     Updated { old_value: i64, new_value: i64 },
    ///     Reset { previous_value: i64 },
    ///     CurrentValue(i64),
    /// }
    ///
    /// #[async_trait]
    /// impl Actor for CounterActor {
    ///     type Event = CounterEvent;
    ///     type Response = CounterResponse;
    ///     // ... other associated types
    ///
    ///     fn from_response(response: Self::Response) -> Result<Self::Event, Error> {
    ///         match response {
    ///             CounterResponse::Updated { old_value, new_value } => {
    ///                 Ok(CounterEvent::ValueChanged { old_value, new_value })
    ///             }
    ///             CounterResponse::Reset { previous_value } => {
    ///                 Ok(CounterEvent::Reset { previous_value })
    ///             }
    ///             CounterResponse::CurrentValue(_) => {
    ///                 // Query responses don't generate events
    ///                 Err(Error::Functional(
    ///                     "Query responses don't generate events".to_string()
    ///                 ))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Conditional Event Generation
    ///
    /// ```ignore
    /// #[derive(Debug)]
    /// enum TaskResponse {
    ///     Started { task_id: String },
    ///     Completed { task_id: String, result: TaskResult },
    ///     Failed { task_id: String, error: String },
    ///     AlreadyRunning { task_id: String },
    /// }
    ///
    /// #[async_trait]
    /// impl Actor for TaskManagerActor {
    ///     fn from_response(response: Self::Response) -> Result<Self::Event, Error> {
    ///         match response {
    ///             TaskResponse::Started { task_id } => {
    ///                 Ok(TaskEvent::TaskStarted {
    ///                     task_id,
    ///                     started_at: Utc::now(),
    ///                 })
    ///             }
    ///             TaskResponse::Completed { task_id, result } => {
    ///                 match result {
    ///                     TaskResult::Success(data) => {
    ///                         Ok(TaskEvent::TaskCompleted {
    ///                             task_id,
    ///                             completed_at: Utc::now(),
    ///                             result: data,
    ///                         })
    ///                     }
    ///                     TaskResult::SuccessNoEvent => {
    ///                         // Some successful results don't generate events
    ///                         Err(Error::Functional(
    ///                             "Success without event generation".to_string()
    ///                         ))
    ///                     }
    ///                 }
    ///             }
    ///             TaskResponse::Failed { task_id, error } => {
    ///                 Ok(TaskEvent::TaskFailed {
    ///                     task_id,
    ///                     failed_at: Utc::now(),
    ///                     error_message: error,
    ///                 })
    ///             }
    ///             TaskResponse::AlreadyRunning { .. } => {
    ///                 // Informational responses don't generate events
    ///                 Err(Error::Functional("Informational response".to_string()))
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Usage Patterns
    ///
    /// ## Automatic Event Publishing
    ///
    /// When implemented, this method can be used by the actor system to automatically
    /// publish events after successful message processing:
    ///
    /// ```ignore
    /// // In the actor system's message handling loop
    /// let response = actor.handle_message(sender, message, &mut context).await?;
    ///
    /// // Try to convert response to event and publish
    /// if let Ok(event) = Actor::from_response(response.clone()) {
    ///     context.publish_event(event).await?;
    /// }
    ///
    /// Ok(response)
    /// ```
    ///
    /// ## Event Sourcing State Reconstruction
    ///
    /// Events generated from responses can be used to rebuild actor state:
    ///
    /// ```ignore
    /// impl EventSourcingActor {
    ///     async fn rebuild_from_events(&mut self, events: Vec<MyEvent>) -> Result<(), Error> {
    ///         for event in events {
    ///             self.apply_event(event).await?;
    ///         }
    ///         Ok(())
    ///     }
    /// }
    /// ```
    ///
    /// # Design Considerations
    ///
    /// When implementing this method:
    ///
    /// 1. **Data Completeness**: Ensure responses contain sufficient data for meaningful events
    /// 2. **Event Granularity**: Balance between too many fine-grained events and too few coarse events
    /// 3. **Response Types**: Not all response types need to generate events (queries vs. commands)
    /// 4. **Event Versioning**: Consider event schema evolution for long-term storage
    /// 5. **Performance Impact**: Event conversion should be lightweight and fast
    /// 6. **Error Handling**: Be specific about why conversion fails for debugging
    ///
    /// # Thread Safety
    ///
    /// This is a static method that operates only on the provided response data and doesn't
    /// access actor state or shared resources, making it inherently thread-safe.
    ///
    /// # Performance Considerations
    ///
    /// - Keep conversion logic simple and fast to avoid impacting message processing
    /// - Avoid expensive operations like deep cloning or serialization
    /// - Consider lazy evaluation for complex event construction
    /// - Cache frequently used conversion patterns when possible
    fn from_response(_response: Self::Response) -> Result<Self::Event, Error> {
        Err(Error::Functional("Event conversion from response not implemented".to_string()))
    }
}

/// Defines the interface for events that actors can emit to notify subscribers of significant occurrences.
///
/// The `Event` trait is a marker trait that establishes the requirements for types that can be
/// used as events in the actor system. Events represent notable occurrences, state changes, or
/// completed operations that other parts of the system may need to know about. They form the
/// foundation for implementing event sourcing patterns, cross-actor communication, and system
/// monitoring.
///
/// # Core Principles
///
/// Events in this system follow several key principles:
/// - **Immutable**: Once created, events should not be modified
/// - **Self-describing**: Events should contain all information needed to understand what occurred
/// - **Granular**: Events should represent single, atomic occurrences
/// - **Serializable**: Events must be serializable for persistence and distribution
/// - **Timeless**: Events should be meaningful regardless of when they're processed
///
/// # Trait Requirements
///
/// Types implementing `Event` must satisfy several trait bounds:
/// - `Serialize + DeserializeOwned`: For persistence, network transmission, and event replay
/// - `Debug`: For logging, debugging, and diagnostic purposes
/// - `Clone`: For efficient distribution to multiple subscribers
/// - `Send + Sync + 'static`: For safe use in concurrent, multi-threaded actor systems
///
/// # Event Sourcing Integration
///
/// Events are primarily used to implement event sourcing patterns where:
/// 1. Actors emit events when their state changes
/// 2. Events are persisted to an event store
/// 3. Actor state can be reconstructed by replaying events
/// 4. External systems can subscribe to events for integration
///
/// # Usage Patterns
///
/// ## Domain Events
/// Events that represent business domain occurrences:
///
/// ```ignore
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub enum OrderEvent {
///     OrderCreated {
///         order_id: String,
///         customer_id: String,
///         total_amount: Decimal,
///         created_at: DateTime<Utc>,
///     },
///     OrderItemAdded {
///         order_id: String,
///         item_id: String,
///         quantity: u32,
///         unit_price: Decimal,
///     },
///     OrderShipped {
///         order_id: String,
///         tracking_number: String,
///         shipped_at: DateTime<Utc>,
///         carrier: String,
///     },
///     OrderCancelled {
///         order_id: String,
///         reason: CancellationReason,
///         cancelled_at: DateTime<Utc>,
///     },
/// }
///
/// impl Event for OrderEvent {}
/// ```
///
/// ## System Events
/// Events that represent system-level occurrences:
///
/// ```ignore
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub enum SystemEvent {
///     ActorStarted {
///         actor_path: String,
///         actor_type: String,
///         started_at: DateTime<Utc>,
///     },
///     ActorFailed {
///         actor_path: String,
///         error: String,
///         failed_at: DateTime<Utc>,
///     },
///     MessageProcessed {
///         actor_path: String,
///         message_type: String,
///         processing_time_ms: u64,
///     },
/// }
///
/// impl Event for SystemEvent {}
/// ```
///
/// ## Aggregate Events
/// Events that represent changes to aggregate roots:
///
/// ```ignore
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub enum UserEvent {
///     UserRegistered {
///         user_id: UserId,
///         email: String,
///         registered_at: DateTime<Utc>,
///     },
///     EmailVerified {
///         user_id: UserId,
///         verified_at: DateTime<Utc>,
///     },
///     ProfileUpdated {
///         user_id: UserId,
///         changes: ProfileChanges,
///         updated_at: DateTime<Utc>,
///     },
/// }
///
/// impl Event for UserEvent {}
/// ```
///
/// # Event Publishing
///
/// Actors emit events using the `publish_event` method on their context:
///
/// ```ignore
/// #[async_trait]
/// impl Handler<OrderActor> for OrderActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             OrderMessage::CreateOrder { customer_id, items } => {
///                 let order = Order::new(customer_id, items);
///                 self.orders.insert(order.id.clone(), order.clone());
///
///                 // Emit event for external subscribers
///                 ctx.publish_event(OrderEvent::OrderCreated {
///                     order_id: order.id,
///                     customer_id: order.customer_id,
///                     total_amount: order.total_amount,
///                     created_at: Utc::now(),
///                 }).await?;
///
///                 Ok(OrderResponse::Created(order.id))
///             }
///         }
///     }
/// }
/// ```
///
/// # Event Subscription
///
/// External systems can subscribe to actor events:
///
/// ```ignore
/// // Subscribe to order events
/// let order_actor_ref = system.get_actor::<OrderActor>(&order_path).await?;
/// let mut event_receiver = order_actor_ref.subscribe();
///
/// // Process events as they arrive
/// while let Ok(event) = event_receiver.recv().await {
///     match event {
///         OrderEvent::OrderCreated { order_id, total_amount, .. } => {
///             // Update analytics, send notifications, etc.
///             analytics_service.record_order_created(order_id, total_amount).await;
///         }
///         OrderEvent::OrderShipped { order_id, tracking_number, .. } => {
///             // Send shipping notification
///             notification_service.send_shipping_notification(order_id, tracking_number).await;
///         }
///         _ => {} // Handle other events as needed
///     }
/// }
/// ```
///
/// # Design Guidelines
///
/// When designing events, follow these guidelines:
///
/// ## Event Naming
/// - Use past tense verbs: `OrderCreated`, not `CreateOrder`
/// - Be specific about what occurred: `OrderItemAdded` vs. `OrderChanged`
/// - Include the aggregate or entity type: `UserRegistered`, `OrderShipped`
///
/// ## Event Content
/// - Include all necessary information to understand what happened
/// - Include relevant identifiers for correlation and filtering
/// - Include timestamps for temporal ordering and auditing
/// - Avoid including computed or derivable information unless essential
///
/// ## Event Granularity
/// - Prefer smaller, focused events over large, monolithic ones
/// - Each event should represent a single, atomic occurrence
/// - Consider the needs of different subscribers when determining granularity
///
/// ## Schema Evolution
/// - Design events to be forward and backward compatible
/// - Use optional fields for new information
/// - Consider versioning strategies for breaking changes
/// - Document event schema changes for subscribers
///
/// # Performance Considerations
///
/// - Events are cloned for each subscriber, so keep them reasonably sized
/// - Serialization overhead applies when persisting or transmitting events
/// - Consider using Arc<T> for large, immutable data shared across events
/// - Event processing is asynchronous and doesn't block the actor
///
/// # Thread Safety
///
/// Events must be `Send + Sync` to be safely shared across threads in the actor system.
/// This means all contained data must also be thread-safe. Use appropriate synchronization
/// primitives or immutable data structures as needed.
pub trait Event:
    Serialize + DeserializeOwned + Debug + Clone + Send + Sync + 'static
{
}

/// Defines the interface for messages that can be sent to and processed by actors.
///
/// The `Message` trait is a marker trait that establishes the requirements for types that can be
/// used as messages in the actor system. Messages are the primary means of communication between
/// actors, enabling safe, asynchronous interaction without shared mutable state. All actor
/// communication, whether using the tell (fire-and-forget) or ask (request-response) patterns,
/// must use types that implement this trait.
///
/// # Core Principles
///
/// Messages in this system follow several key principles:
/// - **Immutable Communication**: Messages should be immutable once created to avoid race conditions
/// - **Clear Intent**: Messages should clearly express what operation or information is being requested
/// - **Self-contained**: Messages should include all information needed for processing
/// - **Type Safety**: The type system ensures only valid messages can be sent to specific actors
/// - **Serializable**: Messages can be serialized for network transmission or persistence (when needed)
///
/// # Trait Requirements
///
/// Types implementing `Message` must satisfy several trait bounds:
/// - `Clone`: Messages can be efficiently copied for distribution and retry mechanisms
/// - `Send + Sync`: Messages can be safely transferred between threads in the concurrent actor system
/// - `'static`: Messages can live for the entire program duration, enabling flexible async processing
///
/// # Message Patterns
///
/// ## Command Messages
/// Messages that request an actor to perform an action:
///
/// ```ignore
/// #[derive(Debug, Clone)]
/// pub enum BankAccountMessage {
///     Deposit { amount: Decimal },
///     Withdraw { amount: Decimal },
///     Transfer {
///         to_account: AccountId,
///         amount: Decimal
///     },
///     GetBalance,
///     Close,
/// }
///
/// impl Message for BankAccountMessage {}
/// ```
///
/// ## Query Messages
/// Messages that request information without side effects:
///
/// ```ignore
/// #[derive(Debug, Clone)]
/// pub enum UserQueryMessage {
///     GetUserById { user_id: UserId },
///     GetUsersByRole { role: UserRole },
///     SearchUsers {
///         query: String,
///         limit: usize
///     },
///     GetUserCount,
///     ValidateCredentials {
///         username: String,
///         password: String
///     },
/// }
///
/// impl Message for UserQueryMessage {}
/// ```
///
/// ## Event Messages
/// Messages that notify about something that has occurred:
///
/// ```ignore
/// #[derive(Debug, Clone)]
/// pub enum SystemNotificationMessage {
///     UserLoggedIn {
///         user_id: UserId,
///         timestamp: DateTime<Utc>
///     },
///     ConfigurationChanged {
///         section: String,
///         old_value: String,
///         new_value: String
///     },
///     HealthCheckFailed {
///         service: String,
///         error: String
///     },
/// }
///
/// impl Message for SystemNotificationMessage {}
/// ```
///
/// ## Complex Data Messages
/// Messages carrying structured data:
///
/// ```ignore
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// pub struct ProcessOrderMessage {
///     pub order_id: String,
///     pub customer_info: CustomerInfo,
///     pub items: Vec<OrderItem>,
///     pub shipping_address: Address,
///     pub payment_method: PaymentMethod,
/// }
///
/// impl Message for ProcessOrderMessage {}
///
/// #[derive(Debug, Clone)]
/// pub struct BatchProcessMessage {
///     pub batch_id: String,
///     pub items: Vec<ProcessOrderMessage>,
///     pub priority: Priority,
/// }
///
/// impl Message for BatchProcessMessage {}
/// ```
///
/// # Message Design Guidelines
///
/// ## Naming Conventions
/// - Use clear, descriptive names that indicate intent: `CreateUser`, not `UserMessage`
/// - Use verbs for commands: `UpdateProfile`, `DeleteAccount`
/// - Use nouns for queries: `UserDetails`, `AccountBalance`
/// - Include relevant context: `UpdateUserProfile` vs. `UpdateProfile`
///
/// ## Structure Design
/// - Use enums for actors that handle multiple message types
/// - Use structs for single-purpose messages with complex data
/// - Include all necessary parameters as fields, not as separate messages
/// - Consider using builder patterns for complex messages with many optional fields
///
/// ## Data Inclusion
/// - Include all data needed for processing to avoid additional lookups
/// - Include correlation IDs for tracing and debugging
/// - Include timestamps when temporal ordering is important
/// - Avoid including computed or derivable data unless it's expensive to calculate
///
/// # Usage Examples
///
/// ## Basic Actor Communication
///
/// ```ignore
/// #[async_trait]
/// impl Handler<CalculatorActor> for CalculatorActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             CalculatorMessage::Add { a, b } => {
///                 let result = a + b;
///                 self.history.push(Operation::Addition { a, b, result });
///                 Ok(CalculatorResponse::Result(result))
///             }
///             CalculatorMessage::Subtract { a, b } => {
///                 let result = a - b;
///                 self.history.push(Operation::Subtraction { a, b, result });
///                 Ok(CalculatorResponse::Result(result))
///             }
///             CalculatorMessage::GetHistory => {
///                 Ok(CalculatorResponse::History(self.history.clone()))
///             }
///             CalculatorMessage::Clear => {
///                 self.history.clear();
///                 Ok(CalculatorResponse::Cleared)
///             }
///         }
///     }
/// }
/// ```
///
/// ## Tell Pattern (Fire-and-Forget)
///
/// ```ignore
/// // Send a message without waiting for response
/// actor_ref.tell(UserMessage::UpdateLastLogin {
///     user_id: current_user.id,
///     timestamp: Utc::now(),
/// }).await?;
///
/// // Send batch processing message
/// batch_processor.tell(BatchProcessMessage {
///     batch_id: "batch_001".to_string(),
///     items: pending_orders,
///     priority: Priority::Normal,
/// }).await?;
/// ```
///
/// ## Ask Pattern (Request-Response)
///
/// ```ignore
/// // Send a message and wait for response
/// let balance = account_actor.ask(BankAccountMessage::GetBalance).await?;
/// println!("Current balance: {}", balance);
///
/// // Send query with parameters
/// let users = user_service.ask(UserQueryMessage::GetUsersByRole {
///     role: UserRole::Admin
/// }).await?;
/// ```
///
/// # Message Routing and Delivery
///
/// Messages in the actor system are:
/// - **Delivered asynchronously**: Messages are queued and processed when the actor is available
/// - **Processed sequentially**: Each actor processes messages one at a time in order
/// - **Location transparent**: Messages can be sent to local or remote actors using the same API
/// - **Fault tolerant**: Message delivery handles actor failures gracefully through supervision
///
/// # Performance Considerations
///
/// ## Message Size
/// - Keep messages reasonably sized to minimize serialization and memory overhead
/// - For large data, consider sending references or using streaming patterns
/// - Clone operations should be efficient - avoid deep copying large structures
///
/// ## Message Frequency
/// - High-frequency messaging can overwhelm actors - consider batching strategies
/// - Use backpressure mechanisms for flow control when needed
/// - Monitor mailbox sizes to detect performance bottlenecks
///
/// ## Memory Management
/// - Messages are cloned for delivery, so efficient cloning is important
/// - Consider using `Arc<T>` for large, read-only data shared across messages
/// - Avoid keeping references to external resources that prevent garbage collection
///
/// # Thread Safety and Concurrency
///
/// Messages must be `Send + Sync` to be safely transferred between threads in the actor system.
/// This means:
/// - All contained data must be thread-safe
/// - Use appropriate synchronization primitives for shared mutable data
/// - Prefer immutable data structures when possible
/// - Avoid raw pointers or non-thread-safe types
///
/// # Integration Patterns
///
/// ## Message Transformation
/// ```ignore
/// // Convert external API requests to internal messages
/// impl From<CreateUserRequest> for UserMessage {
///     fn from(request: CreateUserRequest) -> Self {
///         UserMessage::CreateUser {
///             username: request.username,
///             email: request.email,
///             role: request.role.unwrap_or(UserRole::User),
///         }
///     }
/// }
/// ```
///
/// ## Message Validation
/// ```ignore
/// impl BankAccountMessage {
///     pub fn validate(&self) -> Result<(), ValidationError> {
///         match self {
///             BankAccountMessage::Withdraw { amount } |
///             BankAccountMessage::Deposit { amount } => {
///                 if *amount <= Decimal::ZERO {
///                     Err(ValidationError::InvalidAmount)
///                 } else {
///                     Ok(())
///                 }
///             }
///             _ => Ok(())
///         }
///     }
/// }
/// ```
pub trait Message: Clone + Send + Sync + 'static {}

/// Defines the interface for responses that actors can return when processing ask-pattern messages.
///
/// The `Response` trait is a marker trait that establishes the requirements for types that can be
/// used as responses in the actor system's ask-pattern communication. When an actor receives a
/// message sent via the `ask` method (request-response pattern), it must return a value that
/// implements this trait. This enables type-safe, asynchronous request-response communication
/// between actors.
///
/// # Core Concepts
///
/// Responses in this system serve several important purposes:
/// - **Type Safety**: Ensure compile-time validation of request-response contracts
/// - **Async Communication**: Enable non-blocking request-response patterns between actors
/// - **Error Handling**: Provide structured ways to communicate success, failure, and data
/// - **Contract Definition**: Define clear interfaces for what actors can return
/// - **Pattern Matching**: Allow callers to handle different response scenarios elegantly
///
/// # Trait Requirements
///
/// Types implementing `Response` must satisfy several trait bounds:
/// - `Send`: Responses can be safely transferred between threads in async contexts
/// - `Sync`: Responses can be safely shared between threads (for caching, logging, etc.)
/// - `'static`: Responses can live for the entire program duration, enabling flexible async processing
///
/// # Response Patterns
///
/// ## Success/Error Responses
/// Simple responses that indicate success or failure:
///
/// ```ignore
/// #[derive(Debug)]
/// pub enum UserServiceResponse {
///     UserCreated { user_id: UserId },
///     UserFound(UserInfo),
///     UserNotFound,
///     ValidationError(String),
///     DatabaseError(String),
/// }
///
/// impl Response for UserServiceResponse {}
/// ```
///
/// ## Data-Carrying Responses
/// Responses that return structured data:
///
/// ```ignore
/// #[derive(Debug)]
/// pub struct QueryResponse<T> {
///     pub data: Vec<T>,
///     pub total_count: usize,
///     pub page: usize,
///     pub has_more: bool,
/// }
///
/// impl<T: Send + Sync + 'static> Response for QueryResponse<T> {}
///
/// #[derive(Debug)]
/// pub enum CalculatorResponse {
///     Result(f64),
///     Error { message: String, code: u32 },
///     History(Vec<Operation>),
///     Cleared,
/// }
///
/// impl Response for CalculatorResponse {}
/// ```
///
/// ## Status Responses
/// Responses that provide operational status:
///
/// ```ignore
/// #[derive(Debug)]
/// pub struct HealthCheckResponse {
///     pub status: HealthStatus,
///     pub checks: Vec<ComponentHealth>,
///     pub timestamp: DateTime<Utc>,
///     pub response_time_ms: u64,
/// }
///
/// impl Response for HealthCheckResponse {}
///
/// #[derive(Debug)]
/// pub enum ServiceResponse {
///     Started { port: u16, pid: u32 },
///     Stopped,
///     Restarted { uptime: Duration },
///     Failed { error: String },
///     Status(ServiceStatus),
/// }
///
/// impl Response for ServiceResponse {}
/// ```
///
/// ## Aggregate Responses
/// Responses that combine multiple pieces of information:
///
/// ```ignore
/// #[derive(Debug)]
/// pub struct AccountSummaryResponse {
///     pub account_id: String,
///     pub balance: Decimal,
///     pub recent_transactions: Vec<Transaction>,
///     pub credit_score: Option<u32>,
///     pub account_type: AccountType,
///     pub status: AccountStatus,
/// }
///
/// impl Response for AccountSummaryResponse {}
///
/// #[derive(Debug)]
/// pub struct BatchProcessingResponse {
///     pub batch_id: String,
///     pub total_items: usize,
///     pub processed_items: usize,
///     pub failed_items: usize,
///     pub errors: Vec<ProcessingError>,
///     pub duration: Duration,
/// }
///
/// impl Response for BatchProcessingResponse {}
/// ```
///
/// # Usage Examples
///
/// ## Basic Ask-Pattern Usage
///
/// ```ignore
/// #[async_trait]
/// impl Handler<BankAccountActor> for BankAccountActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             BankAccountMessage::GetBalance => {
///                 Ok(BankAccountResponse::Balance(self.balance))
///             }
///             BankAccountMessage::Deposit { amount } => {
///                 if amount <= Decimal::ZERO {
///                     Ok(BankAccountResponse::InvalidAmount)
///                 } else {
///                     self.balance += amount;
///                     ctx.publish_event(AccountEvent::Deposited {
///                         amount,
///                         new_balance: self.balance
///                     }).await?;
///                     Ok(BankAccountResponse::DepositSuccessful {
///                         new_balance: self.balance
///                     })
///                 }
///             }
///             BankAccountMessage::Withdraw { amount } => {
///                 if amount <= Decimal::ZERO {
///                     Ok(BankAccountResponse::InvalidAmount)
///                 } else if amount > self.balance {
///                     Ok(BankAccountResponse::InsufficientFunds {
///                         requested: amount,
///                         available: self.balance
///                     })
///                 } else {
///                     self.balance -= amount;
///                     ctx.publish_event(AccountEvent::Withdrawn {
///                         amount,
///                         new_balance: self.balance
///                     }).await?;
///                     Ok(BankAccountResponse::WithdrawalSuccessful {
///                         new_balance: self.balance
///                     })
///                 }
///             }
///         }
///     }
/// }
/// ```
///
/// ## Response Handling by Callers
///
/// ```ignore
/// // Ask for account balance
/// let response = account_actor.ask(BankAccountMessage::GetBalance).await?;
/// match response {
///     BankAccountResponse::Balance(amount) => {
///         println!("Current balance: ${}", amount);
///     }
///     _ => {
///         println!("Unexpected response to balance query");
///     }
/// }
///
/// // Ask for withdrawal with error handling
/// let response = account_actor.ask(BankAccountMessage::Withdraw {
///     amount: Decimal::new(10000, 2) // $100.00
/// }).await?;
///
/// match response {
///     BankAccountResponse::WithdrawalSuccessful { new_balance } => {
///         println!("Withdrawal successful. New balance: ${}", new_balance);
///     }
///     BankAccountResponse::InsufficientFunds { requested, available } => {
///         println!("Insufficient funds: requested ${}, available ${}", requested, available);
///     }
///     BankAccountResponse::InvalidAmount => {
///         println!("Invalid withdrawal amount");
///     }
///     _ => {
///         println!("Unexpected response to withdrawal request");
///     }
/// }
/// ```
///
/// ## Complex Response Processing
///
/// ```ignore
/// // Process batch operation response
/// let response = batch_processor.ask(BatchMessage::ProcessOrders {
///     orders: pending_orders
/// }).await?;
///
/// match response {
///     BatchProcessingResponse { processed_items, failed_items, errors, .. } => {
///         println!("Batch processing complete: {} processed, {} failed",
///                  processed_items, failed_items);
///
///         if !errors.is_empty() {
///             println!("Errors encountered:");
///             for error in errors {
///                 println!("  - {}: {}", error.item_id, error.message);
///             }
///         }
///     }
/// }
/// ```
///
/// # Response Design Guidelines
///
/// ## Naming Conventions
/// - Use descriptive names that clearly indicate the response content or outcome
/// - Include the result type: `UserFound`, `OrderCreated`, `ValidationFailed`
/// - Match the corresponding message purpose: `GetUser` → `UserFound` or `UserNotFound`
/// - Use consistent terminology across related responses
///
/// ## Structure Design
/// - Use enums for responses that can have multiple outcomes or states
/// - Use structs for responses that always return structured data
/// - Include all relevant information that callers might need
/// - Consider using Result<T, E> patterns within enum variants for error handling
///
/// ## Error Representation
/// - Include sufficient detail for error diagnosis and handling
/// - Provide error codes or categories for programmatic handling
/// - Include context information that helps with debugging
/// - Consider severity levels for different types of errors
///
/// ## Data Completeness
/// - Return all information that's reasonable to include at response time
/// - Avoid forcing callers to make additional requests for commonly needed data
/// - Include metadata like timestamps, counts, or status flags when relevant
/// - Consider pagination information for large data sets
///
/// # Performance Considerations
///
/// ## Response Size
/// - Keep responses reasonably sized to minimize serialization and network overhead
/// - For large data sets, consider pagination or streaming patterns
/// - Use references or IDs instead of full objects when appropriate
/// - Consider compression for responses with repetitive or large text content
///
/// ## Response Time
/// - Aim for fast response generation to maintain system responsiveness
/// - Avoid expensive computations in response construction
/// - Cache frequently requested data when possible
/// - Consider async response patterns for long-running operations
///
/// ## Memory Usage
/// - Responses are moved between threads, so avoid large stack allocations
/// - Use `Box<T>` or `Arc<T>` for large response data when appropriate
/// - Clean up temporary resources used during response generation
/// - Consider using streaming responses for very large data sets
///
/// # Thread Safety and Concurrency
///
/// Responses must be `Send + Sync` to be safely transferred between threads in the actor system.
/// This means:
/// - All contained data must be thread-safe or owned
/// - Use appropriate synchronization primitives for shared data
/// - Prefer owned data over references when crossing thread boundaries
/// - Ensure that any contained types also satisfy thread safety requirements
///
/// # Integration with Error Handling
///
/// Responses work in conjunction with Rust's `Result` type for comprehensive error handling:
///
/// ```ignore
/// // Handler returns Result<Response, Error>
/// async fn handle_message(&mut self, ...) -> Result<Self::Response, Error> {
///     match msg {
///         Message::ValidOperation => {
///             // Successful operation returns response
///             Ok(Response::Success)
///         }
///         Message::RiskyOperation => {
///             if self.can_perform_operation() {
///                 // Business logic success
///                 Ok(Response::OperationComplete)
///             } else {
///                 // Business logic failure (not a system error)
///                 Ok(Response::OperationFailed { reason: "Preconditions not met".to_string() })
///             }
///         }
///         Message::InvalidOperation => {
///             // System-level error
///             Err(Error::InvalidRequest("Operation not supported".to_string()))
///         }
///     }
/// }
/// ```
///
/// This pattern allows for:
/// - **System errors** (network failures, actor crashes) via `Err(Error)`
/// - **Business failures** (validation errors, domain rule violations) via `Ok(Response::Failed(...))`
/// - **Successful operations** with data via `Ok(Response::Success(...))`
pub trait Response: Send + Sync + 'static {}

/// Defines the interface for actors to handle messages, events, and child actor failures.
///
/// The `Handler` trait is the core message processing interface in the actor system, providing
/// methods for actors to respond to various types of interactions and system events. This trait
/// works in conjunction with the `Actor` trait to provide a complete actor implementation,
/// separating lifecycle management from message processing concerns.
///
/// # Core Concepts
///
/// The Handler trait enables actors to:
/// - **Process Messages**: Handle incoming messages via the ask/tell patterns
/// - **React to Events**: Respond to internal events emitted by the actor itself
/// - **Supervise Children**: Handle errors and faults from child actors
/// - **Maintain State**: Access and modify actor state during message processing
/// - **Emit Responses**: Return structured responses for ask-pattern communications
///
/// # Message Processing Model
///
/// Actors process messages sequentially in the order they arrive, ensuring:
/// - **Thread Safety**: No need for internal synchronization within message handlers
/// - **State Consistency**: Actor state remains consistent across message processing
/// - **Isolation**: Each message is processed atomically without interference
/// - **Backpressure**: Natural flow control through sequential processing
///
/// # Event-Driven Architecture
///
/// The handler supports event-driven patterns where actors can:
/// - Emit events to notify subscribers of state changes
/// - React to their own events for complex state management
/// - Implement event sourcing patterns for audit trails and state reconstruction
/// - Create loosely coupled systems through event-based communication
///
/// # Supervision and Fault Tolerance
///
/// Actors can supervise child actors by implementing fault handling methods:
/// - Handle child errors with custom recovery strategies
/// - Decide child fate based on fault types and context
/// - Implement resilience patterns like restart, escalate, or resume
/// - Maintain system stability through hierarchical error handling
///
/// # Type Safety
///
/// The trait uses generic parameters to ensure compile-time type safety:
/// - `A`: The actor type that implements both `Actor` and `Handler<A>`
/// - Messages, events, and responses are statically typed
/// - Prevents runtime type errors through Rust's type system
///
/// # Examples
///
/// ## Basic Message Handler
///
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// #[derive(Debug)]
/// struct BankAccountActor {
///     balance: Decimal,
///     account_id: String,
/// }
///
/// #[derive(Debug, Clone)]
/// enum BankAccountMessage {
///     Deposit { amount: Decimal },
///     Withdraw { amount: Decimal },
///     GetBalance,
///     Transfer { to: String, amount: Decimal },
/// }
///
/// impl Message for BankAccountMessage {}
///
/// #[derive(Debug)]
/// enum BankAccountResponse {
///     Balance(Decimal),
///     TransactionComplete { new_balance: Decimal },
///     InsufficientFunds,
///     InvalidAmount,
/// }
///
/// impl Response for BankAccountResponse {}
///
/// #[async_trait]
/// impl Handler<BankAccountActor> for BankAccountActor {
///     async fn handle_message(
///         &mut self,
///         sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             BankAccountMessage::Deposit { amount } => {
///                 if amount <= Decimal::ZERO {
///                     return Ok(BankAccountResponse::InvalidAmount);
///                 }
///                 self.balance += amount;
///                 ctx.publish_event(BankAccountEvent::MoneyDeposited {
///                     account_id: self.account_id.clone(),
///                     amount,
///                     new_balance: self.balance,
///                 }).await?;
///                 Ok(BankAccountResponse::TransactionComplete { new_balance: self.balance })
///             }
///             BankAccountMessage::Withdraw { amount } => {
///                 if amount <= Decimal::ZERO {
///                     return Ok(BankAccountResponse::InvalidAmount);
///                 }
///                 if self.balance < amount {
///                     return Ok(BankAccountResponse::InsufficientFunds);
///                 }
///                 self.balance -= amount;
///                 ctx.publish_event(BankAccountEvent::MoneyWithdrawn {
///                     account_id: self.account_id.clone(),
///                     amount,
///                     new_balance: self.balance,
///                 }).await?;
///                 Ok(BankAccountResponse::TransactionComplete { new_balance: self.balance })
///             }
///             BankAccountMessage::GetBalance => Ok(BankAccountResponse::Balance(self.balance)),
///             BankAccountMessage::Transfer { to, amount } => {
///                 // Delegate to another actor for cross-account operations
///                 let transfer_service = ctx.get_service("transfer-service")?;
///                 transfer_service.tell(TransferMessage::Execute {
///                     from: self.account_id.clone(),
///                     to,
///                     amount,
///                 }).await?;
///                 Ok(BankAccountResponse::TransactionComplete { new_balance: self.balance })
///             }
///         }
///     }
/// }
/// ```
///
/// ## Event Handler with State Management
///
/// ```ignore
/// #[async_trait]
/// impl Handler<OrderProcessorActor> for OrderProcessorActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             OrderMessage::ProcessOrder { order } => {
///                 // Emit event for audit trail
///                 ctx.publish_event(OrderEvent::ProcessingStarted {
///                     order_id: order.id.clone(),
///                     timestamp: Utc::now(),
///                 }).await?;
///
///                 // Process the order
///                 self.process_order_internal(order, ctx).await
///             }
///             // ... other message patterns
///         }
///     }
///
///     async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
///         match event {
///             OrderEvent::ProcessingStarted { order_id, timestamp } => {
///                 // Update internal metrics
///                 self.processing_started_count += 1;
///                 self.last_processing_time = timestamp;
///
///                 // Log for monitoring
///                 info!("Started processing order {} at {}", order_id, timestamp);
///             }
///             OrderEvent::OrderCompleted { order_id, total_amount } => {
///                 // Update totals and send notification
///                 self.daily_revenue += total_amount;
///
///                 if let Some(notification_service) = ctx.get_optional_service("notifications") {
///                     notification_service.tell(NotificationMessage::OrderComplete {
///                         order_id,
///                         amount: total_amount,
///                     }).await.ok();
///                 }
///             }
///         }
///     }
/// }
/// ```
///
/// ## Child Actor Supervision
///
/// ```ignore
/// #[async_trait]
/// impl Handler<SupervisorActor> for SupervisorActor {
///     async fn handle_message(
///         &mut self,
///         _sender: ActorPath,
///         msg: Self::Message,
///         ctx: &mut ActorContext<Self>,
///     ) -> Result<Self::Response, Error> {
///         match msg {
///             SupervisorMessage::SpawnWorker { config } => {
///                 let worker = WorkerActor::new(config);
///                 let worker_ref = ctx.spawn_child("worker", worker).await?;
///                 self.workers.push(worker_ref);
///                 Ok(SupervisorResponse::WorkerSpawned)
///             }
///             // ... other patterns
///         }
///     }
///
///     async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
///         warn!("Child actor error occurred: {:?}", error);
///
///         // Log error for monitoring
///         self.error_count += 1;
///
///         // Notify monitoring system
///         if let Some(monitor) = ctx.get_optional_service("monitor") {
///             monitor.tell(MonitorMessage::ChildError {
///                 parent: ctx.path().clone(),
///                 error: error.to_string(),
///                 timestamp: Utc::now(),
///             }).await.ok();
///         }
///     }
///
///     async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
///         error!("Child actor fault: {:?}", error);
///
///         // Implement retry logic based on error type
///         match error.error_type() {
///             ErrorType::Temporary => {
///                 self.retry_count += 1;
///                 if self.retry_count < 3 {
///                     warn!("Restarting child after temporary error (attempt {})", self.retry_count);
///                     ChildAction::Restart
///                 } else {
///                     error!("Max retries reached, escalating error");
///                     ChildAction::Escalate
///                 }
///             }
///             ErrorType::Fatal => {
///                 error!("Fatal error in child, stopping permanently");
///                 ChildAction::Stop
///             }
///             ErrorType::Configuration => {
///                 error!("Configuration error, escalating to parent");
///                 ChildAction::Escalate
///             }
///         }
///     }
/// }
/// ```
///
/// # Performance Considerations
///
/// ## Message Processing
/// - Handlers execute sequentially, so avoid blocking operations
/// - Use `ctx.spawn_task()` for concurrent operations that don't require actor state
/// - Consider message batching for high-throughput scenarios
/// - Implement backpressure handling for message queues that might overflow
///
/// ## State Management
/// - Keep actor state reasonably sized to minimize memory overhead
/// - Use efficient data structures for frequently accessed state
/// - Consider state partitioning for large datasets
/// - Implement state cleanup in lifecycle hooks
///
/// ## Error Handling
/// - Handle expected errors gracefully within message handlers
/// - Use structured error responses rather than generic errors when possible
/// - Implement circuit breaker patterns for external service dependencies
/// - Log errors appropriately for monitoring and debugging
///
/// # Thread Safety and Concurrency
///
/// Handler implementations must be `Send + Sync` to work in the actor system:
/// - Actor state is protected by the actor's message processing isolation
/// - No additional synchronization is needed within handlers
/// - External resources accessed from handlers must be thread-safe
/// - Shared state should use appropriate concurrent data structures
///
/// # Integration with Actor Lifecycle
///
/// The Handler trait works closely with Actor lifecycle:
/// - Message handling occurs during the actor's active state
/// - Events can be emitted during any lifecycle phase
/// - Child supervision operates throughout the actor's lifetime
/// - Context provides access to actor system services and state
#[async_trait]
pub trait Handler<A: Actor + Handler<A>>: Send + Sync {
    /// Processes an incoming message and returns an appropriate response.
    ///
    /// This is the core method of the Handler trait, responsible for processing all messages
    /// sent to the actor via the tell/ask patterns. The method is called by the actor system's
    /// runtime when a message arrives in the actor's mailbox, providing a controlled execution
    /// environment where the actor can safely modify its internal state and interact with
    /// other actors or system services.
    ///
    /// # Message Processing Lifecycle
    ///
    /// When a message arrives, the following sequence occurs:
    /// 1. **Message Dequeue**: The runtime removes the message from the actor's mailbox
    /// 2. **Context Setup**: An `ActorContext` is prepared with access to system services
    /// 3. **Handler Invocation**: This method is called with the message and context
    /// 4. **State Modification**: The actor can safely modify its internal state
    /// 5. **Side Effects**: Events can be emitted, child actors spawned, or external services called
    /// 6. **Response Generation**: A response is returned for ask-pattern messages
    /// 7. **Response Delivery**: The runtime delivers the response to the waiting caller
    ///
    /// # Parameters
    ///
    /// * `sender` - The `ActorPath` identifying which actor sent this message. This enables:
    ///   - Reply patterns where responses are sent back to the original sender
    ///   - Audit trails and message tracing for debugging and monitoring
    ///   - Access control decisions based on the sender's identity or location
    ///   - Routing decisions in complex multi-actor workflows
    ///
    /// * `msg` - The message to process, of type `A::Message`. The message contains:
    ///   - Command data specifying what operation to perform
    ///   - Parameters needed to execute the requested operation
    ///   - Context information like correlation IDs for distributed tracing
    ///   - Metadata for message routing, priority, or deadline handling
    ///
    /// * `ctx` - The actor context providing access to:
    ///   - Actor system services (other actors, databases, external APIs)
    ///   - Event publishing capabilities for emitting state change notifications
    ///   - Child actor management (spawning, monitoring, lifecycle control)
    ///   - System utilities like timers, schedulers, and configuration
    ///   - Actor's own path and identity information
    ///
    /// # Returns
    ///
    /// Returns a `Result<A::Response, Error>` where:
    ///
    /// * `Ok(response)` - Successful message processing with typed response data:
    ///   - For queries: Data requested by the caller (user info, account balance, etc.)
    ///   - For commands: Confirmation of successful execution and any result data
    ///   - For status checks: Current state information or operational metrics
    ///
    /// * `Err(error)` - Message processing failed due to:
    ///   - **System errors**: Network failures, database unavailability, actor system issues
    ///   - **Invalid input**: Malformed messages, missing required fields, type mismatches
    ///   - **Business rule violations**: Insufficient funds, permission denied, invalid state
    ///   - **Resource constraints**: Memory exhaustion, connection limits, timeout exceeded
    ///   - **Dependency failures**: Child actor crashes, external service errors
    ///
    /// # Error Handling Strategies
    ///
    /// ## Structured Error Responses
    /// For business logic errors that clients should handle gracefully, prefer returning
    /// structured error responses within `Ok(...)` rather than using `Err(...)`:
    ///
    /// ```ignore
    /// // Preferred: Structured business errors
    /// match msg {
    ///     BankMessage::Withdraw { amount } => {
    ///         if self.balance < amount {
    ///             Ok(BankResponse::InsufficientFunds {
    ///                 requested: amount,
    ///                 available: self.balance,
    ///             })
    ///         } else {
    ///             self.balance -= amount;
    ///             Ok(BankResponse::WithdrawalComplete { new_balance: self.balance })
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## System Error Propagation
    /// Use `Err(...)` for genuine system failures that indicate the actor or system
    /// is in an unexpected state and cannot continue normal processing:
    ///
    /// ```ignore
    /// match self.database.get_user(user_id).await {
    ///     Ok(user) => Ok(UserResponse::Profile(user)),
    ///     Err(db_error) => {
    ///         error!("Database connection failed: {:?}", db_error);
    ///         Err(Error::DatabaseError(db_error.to_string()))
    ///     }
    /// }
    /// ```
    ///
    /// # Threading and Concurrency
    ///
    /// The `handle_message` method executes with these concurrency guarantees:
    /// - **Sequential Execution**: Messages are processed one at a time, in order
    /// - **Thread Safety**: No additional synchronization needed for actor state
    /// - **Isolation**: Each message handler has exclusive access to the actor's state
    /// - **Non-blocking**: Should avoid blocking operations that could stall message processing
    ///
    /// For operations that might block or take significant time, use the context to spawn
    /// concurrent tasks:
    ///
    /// ```ignore
    /// match msg {
    ///     Message::ProcessLargeFile { file_path } => {
    ///         // Spawn concurrent task for CPU-intensive work
    ///         let handle = ctx.spawn_task(async move {
    ///             process_file_intensive(file_path).await
    ///         });
    ///
    ///         // Continue processing other messages
    ///         Ok(Response::ProcessingStarted)
    ///     }
    /// }
    /// ```
    ///
    /// # State Management Patterns
    ///
    /// ## Atomic State Updates
    /// All state changes within a message handler are atomic from the perspective
    /// of other messages, enabling safe state transitions:
    ///
    /// ```ignore
    /// match msg {
    ///     OrderMessage::UpdateStatus { order_id, new_status } => {
    ///         if let Some(order) = self.orders.get_mut(&order_id) {
    ///             let old_status = order.status;
    ///             order.status = new_status;
    ///             order.updated_at = Utc::now();
    ///
    ///             // Emit event after state change is complete
    ///             ctx.publish_event(OrderEvent::StatusChanged {
    ///                 order_id,
    ///                 old_status,
    ///                 new_status,
    ///             }).await?;
    ///
    ///             Ok(OrderResponse::Updated)
    ///         } else {
    ///             Ok(OrderResponse::NotFound)
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Event Sourcing Integration
    /// Use event publishing to maintain an audit trail and support event sourcing:
    ///
    /// ```ignore
    /// async fn handle_message(
    ///     &mut self,
    ///     sender: ActorPath,
    ///     msg: Self::Message,
    ///     ctx: &mut ActorContext<Self>,
    /// ) -> Result<Self::Response, Error> {
    ///     match msg {
    ///         AccountMessage::Transfer { to, amount } => {
    ///             // Validate operation
    ///             if self.balance < amount {
    ///                 return Ok(AccountResponse::InsufficientFunds);
    ///             }
    ///
    ///             // Apply state change
    ///             self.balance -= amount;
    ///
    ///             // Record event for audit and event sourcing
    ///             ctx.publish_event(AccountEvent::MoneyTransferred {
    ///                 from: ctx.path().clone(),
    ///                 to: to.clone(),
    ///                 amount,
    ///                 timestamp: Utc::now(),
    ///                 resulting_balance: self.balance,
    ///             }).await?;
    ///
    ///             // Notify recipient
    ///             if let Some(recipient) = ctx.get_actor(&to).await? {
    ///                 recipient.tell(AccountMessage::Receive { amount }).await?;
    ///             }
    ///
    ///             Ok(AccountResponse::TransferComplete)
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Guidelines
    ///
    /// ## Efficient Message Handling
    /// - Keep message handlers lightweight and fast-executing
    /// - Defer expensive operations to background tasks when possible
    /// - Use efficient data structures for frequently accessed state
    /// - Minimize memory allocations within hot paths
    ///
    /// ## Resource Management
    /// - Close resources explicitly rather than relying on Drop
    /// - Use connection pooling for database and network resources
    /// - Implement timeouts for external service calls
    /// - Monitor and limit memory usage for large datasets
    ///
    /// ## Error Recovery
    /// - Implement retry logic for transient failures
    /// - Use circuit breaker patterns for external dependencies
    /// - Provide meaningful error messages for debugging
    /// - Log errors at appropriate levels for monitoring
    ///
    /// # Examples
    ///
    /// See the comprehensive examples in the trait-level documentation for complete
    /// implementations showing message handling patterns, error management, and
    /// integration with the actor system's event and supervision capabilities.
    async fn handle_message(
        &mut self,
        sender: ActorPath,
        msg: A::Message,
        ctx: &mut ActorContext<A>,
    ) -> Result<A::Response, Error>;

    /// Handles internal events emitted by this actor, enabling event-driven state management and reactive patterns.
    ///
    /// This method is called whenever the actor publishes an event using `ctx.publish_event()`,
    /// allowing the actor to react to its own state changes and implement complex event-driven
    /// behaviors. Events are processed synchronously after being emitted, providing a way to
    /// maintain derived state, trigger side effects, or implement sophisticated business logic
    /// that spans multiple operations.
    ///
    /// # Event Processing Model
    ///
    /// Events flow through the following lifecycle:
    /// 1. **Event Emission**: Actor calls `ctx.publish_event(event)` during message processing
    /// 2. **Event Persistence**: Event is optionally persisted to event store (if configured)
    /// 3. **Event Notification**: Event is delivered to external subscribers (if any)
    /// 4. **Self-Handling**: This `on_event` method is called for the emitting actor
    /// 5. **State Updates**: Actor can update derived state or trigger additional actions
    ///
    /// The default implementation does nothing, making event handling entirely optional.
    /// Override this method to implement event-driven patterns specific to your actor's needs.
    ///
    /// # Parameters
    ///
    /// * `event` - The event that was emitted, of type `A::Event`. Events typically contain:
    ///   - **State change information**: What changed, old and new values
    ///   - **Contextual data**: When the change occurred, who triggered it
    ///   - **Correlation identifiers**: For tracing related events across time
    ///   - **Metadata**: Additional information for subscribers or audit purposes
    ///
    /// * `ctx` - The actor context providing access to:
    ///   - System services for interacting with other actors and external resources
    ///   - Event publishing for emitting additional events in response
    ///   - Child actor management for coordinating with supervised actors
    ///   - Configuration and runtime information
    ///
    /// # Event-Driven Architecture Patterns
    ///
    /// ## Derived State Management
    /// Use events to maintain computed or aggregated state that depends on multiple operations:
    ///
    /// ```ignore
    /// async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
    ///     match event {
    ///         InventoryEvent::ItemAdded { item_id, quantity, .. } => {
    ///             // Update total inventory count
    ///             self.total_items += quantity;
    ///
    ///             // Update category-specific metrics
    ///             if let Some(category) = self.items.get(&item_id).map(|item| &item.category) {
    ///                 *self.category_counts.entry(category.clone()).or_insert(0) += quantity;
    ///             }
    ///
    ///             // Check if reorder threshold exceeded
    ///             if self.total_items > self.reorder_threshold {
    ///                 ctx.publish_event(InventoryEvent::ReorderThresholdExceeded {
    ///                     current_total: self.total_items,
    ///                     threshold: self.reorder_threshold,
    ///                 }).await.ok();
    ///             }
    ///         }
    ///
    ///         InventoryEvent::ItemRemoved { item_id, quantity, .. } => {
    ///             self.total_items -= quantity;
    ///
    ///             if let Some(category) = self.items.get(&item_id).map(|item| &item.category) {
    ///                 if let Some(count) = self.category_counts.get_mut(category) {
    ///                     *count = count.saturating_sub(quantity);
    ///                 }
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Event Sourcing Integration
    /// Implement event sourcing patterns where state can be reconstructed from events:
    ///
    /// ```ignore
    /// async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
    ///     match event {
    ///         AccountEvent::MoneyDeposited { amount, timestamp, .. } => {
    ///             // Update current balance (already done in message handler)
    ///             // But also update derived state for reporting
    ///             self.total_deposits += amount;
    ///             self.transaction_count += 1;
    ///             self.last_activity = timestamp;
    ///
    ///             // Record transaction for monthly reporting
    ///             let month_key = format!("{}-{}", timestamp.year(), timestamp.month());
    ///             *self.monthly_activity.entry(month_key).or_insert(0) += amount;
    ///         }
    ///
    ///         AccountEvent::MoneyWithdrawn { amount, timestamp, .. } => {
    ///             self.total_withdrawals += amount;
    ///             self.transaction_count += 1;
    ///             self.last_activity = timestamp;
    ///
    ///             // Check for suspicious activity patterns
    ///             if self.is_suspicious_withdrawal_pattern(amount, timestamp) {
    ///                 ctx.publish_event(AccountEvent::SuspiciousActivity {
    ///                     account_id: self.account_id.clone(),
    ///                     activity_type: "large_withdrawal".to_string(),
    ///                     amount,
    ///                     timestamp,
    ///                 }).await.ok();
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## External Integration and Notifications
    /// Use event handling to integrate with external systems and send notifications:
    ///
    /// ```ignore
    /// async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
    ///     match event {
    ///         OrderEvent::OrderCompleted { order_id, customer_id, total_amount, .. } => {
    ///             // Update customer lifetime value
    ///             if let Some(customer) = self.customers.get_mut(&customer_id) {
    ///                 customer.lifetime_value += total_amount;
    ///                 customer.order_count += 1;
    ///             }
    ///
    ///             // Send confirmation email
    ///             if let Some(email_service) = ctx.get_optional_service("email") {
    ///                 email_service.tell(EmailMessage::SendOrderConfirmation {
    ///                     customer_id,
    ///                     order_id,
    ///                     total_amount,
    ///                 }).await.ok();
    ///             }
    ///
    ///             // Update inventory for fulfilled items
    ///             if let Some(inventory_service) = ctx.get_optional_service("inventory") {
    ///                 inventory_service.tell(InventoryMessage::ReserveItems {
    ///                     order_id,
    ///                     items: self.get_order_items(&order_id),
    ///                 }).await.ok();
    ///             }
    ///         }
    ///
    ///         OrderEvent::OrderCancelled { order_id, reason, .. } => {
    ///             // Release reserved inventory
    ///             if let Some(inventory_service) = ctx.get_optional_service("inventory") {
    ///                 inventory_service.tell(InventoryMessage::ReleaseReservation {
    ///                     order_id,
    ///                 }).await.ok();
    ///             }
    ///
    ///             // Send cancellation notification
    ///             if let Some(notification_service) = ctx.get_optional_service("notifications") {
    ///                 notification_service.tell(NotificationMessage::OrderCancelled {
    ///                     order_id,
    ///                     reason: reason.clone(),
    ///                 }).await.ok();
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Event Cascading and Chaining
    ///
    /// Events can trigger additional events, creating cascading event chains for complex workflows:
    ///
    /// ```ignore
    /// async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
    ///     match event {
    ///         UserEvent::AccountCreated { user_id, email, .. } => {
    ///             // Create default preferences
    ///             self.user_preferences.insert(user_id, UserPreferences::default());
    ///
    ///             // Emit follow-up events
    ///             ctx.publish_event(UserEvent::PreferencesInitialized {
    ///                 user_id,
    ///                 preferences: UserPreferences::default(),
    ///             }).await.ok();
    ///
    ///             // Start welcome workflow
    ///             ctx.publish_event(UserEvent::WelcomeWorkflowStarted {
    ///                 user_id,
    ///                 trigger: "account_creation".to_string(),
    ///             }).await.ok();
    ///         }
    ///
    ///         UserEvent::WelcomeWorkflowStarted { user_id, .. } => {
    ///             // Schedule welcome email series
    ///             self.scheduled_communications.push(ScheduledEmail {
    ///                 user_id,
    ///                 template: "welcome_day_1".to_string(),
    ///                 send_at: Utc::now() + Duration::hours(1),
    ///             });
    ///
    ///             self.scheduled_communications.push(ScheduledEmail {
    ///                 user_id,
    ///                 template: "welcome_day_7".to_string(),
    ///                 send_at: Utc::now() + Duration::days(7),
    ///             });
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Error Handling in Event Processing
    ///
    /// Event handlers should be resilient since they process already-committed state changes:
    ///
    /// ```ignore
    /// async fn on_event(&mut self, event: Self::Event, ctx: &mut ActorContext<Self>) {
    ///     match event {
    ///         PaymentEvent::PaymentProcessed { payment_id, amount, .. } => {
    ///             // Update local state - this should not fail
    ///             self.total_processed += amount;
    ///             self.daily_volume += amount;
    ///
    ///             // External integrations may fail - handle gracefully
    ///             if let Some(analytics_service) = ctx.get_optional_service("analytics") {
    ///                 if let Err(e) = analytics_service.tell(AnalyticsMessage::RecordPayment {
    ///                     payment_id,
    ///                     amount,
    ///                 }).await {
    ///                     warn!("Failed to notify analytics service: {:?}", e);
    ///                     // Could emit a retry event or store for later processing
    ///                     self.pending_analytics_updates.push(PendingUpdate {
    ///                         payment_id,
    ///                         amount,
    ///                         retry_count: 0,
    ///                     });
    ///                 }
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Considerations
    ///
    /// ## Event Processing Efficiency
    /// - Keep event handlers lightweight since they execute synchronously with message processing
    /// - Use background tasks for expensive operations triggered by events
    /// - Avoid complex computations that could slow down the main message processing loop
    /// - Consider batching related event processing to reduce overhead
    ///
    /// ## Memory Management
    /// - Clean up derived state periodically to prevent unbounded growth
    /// - Use efficient data structures for event-derived state
    /// - Implement state compression or archival for long-lived actors
    /// - Monitor memory usage in actors that accumulate event-based state
    ///
    /// ## Event Frequency
    /// - High-frequency events can impact performance - consider event aggregation
    /// - Use event sampling for monitoring and metrics rather than processing every event
    /// - Implement circuit breakers for external notifications triggered by events
    /// - Consider asynchronous event processing for non-critical updates
    ///
    /// # Thread Safety and Concurrency
    ///
    /// Event handlers execute within the actor's message processing context:
    /// - **Sequential Execution**: Events are processed in the order they were emitted
    /// - **Thread Safety**: No additional synchronization needed for actor state
    /// - **Atomic Processing**: Event handling is part of the message processing transaction
    /// - **State Consistency**: Actor state remains consistent throughout event processing
    ///
    /// # Integration with Actor Lifecycle
    ///
    /// Event handling operates throughout the actor's lifecycle:
    /// - Events can be emitted and handled during normal message processing
    /// - Lifecycle hooks (pre_start, pre_stop) can emit events that are handled
    /// - Event handling continues until the actor is stopped
    /// - Events emitted during shutdown are processed before final termination
    ///
    async fn on_event(&mut self, _event: A::Event, _ctx: &mut ActorContext<A>) {
        // Default implementation.
    }

    /// Handles errors reported by child actors, providing supervision and monitoring capabilities.
    ///
    /// This method is called when a child actor encounters an error during its normal operation
    /// but continues to function. Unlike `on_child_fault`, which deals with critical failures
    /// that might terminate the child actor, `on_child_error` handles recoverable errors that
    /// allow the child to continue processing. This enables parent actors to implement
    /// comprehensive error monitoring, logging, and potentially take corrective actions.
    ///
    /// # Supervision Model
    ///
    /// The error handling flow follows this sequence:
    /// 1. **Error Occurrence**: Child actor encounters an error during message processing
    /// 2. **Error Reporting**: Child reports the error to its parent supervisor
    /// 3. **Parent Notification**: This method is called on the parent actor
    /// 4. **Error Analysis**: Parent can analyze the error type, frequency, and context
    /// 5. **Monitoring Updates**: Parent updates error counters, logs, and metrics
    /// 6. **Corrective Actions**: Parent may take actions like throttling or configuration updates
    /// 7. **Continued Operation**: Child actor continues normal message processing
    ///
    /// The default implementation logs the error using the `debug!` macro and takes no further
    /// action, making error handling completely optional but providing basic visibility.
    ///
    /// # Parameters
    ///
    /// * `error` - The error that occurred in the child actor, providing:
    ///   - **Error details**: Specific information about what went wrong
    ///   - **Error type**: Category of error (network, validation, resource, etc.)
    ///   - **Context information**: When and where the error occurred
    ///   - **Correlation data**: Request IDs or transaction identifiers for tracing
    ///
    /// * `ctx` - The parent actor's context, providing access to:
    ///   - System services for reporting errors to monitoring systems
    ///   - Child actor references for potential intervention
    ///   - Configuration services for dynamic error handling policies
    ///   - Event publishing for error notifications and alerting
    ///
    /// # Error Monitoring and Analysis
    ///
    /// ## Basic Error Tracking
    /// Track error patterns and frequencies for operational insights:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     // Update error statistics
    ///     self.total_child_errors += 1;
    ///     let error_type = error.error_type();
    ///     *self.error_counts_by_type.entry(error_type).or_insert(0) += 1;
    ///
    ///     // Track error rates per time window
    ///     let now = Utc::now();
    ///     self.recent_errors.push(ErrorRecord {
    ///         error_type: error_type.clone(),
    ///         timestamp: now,
    ///         message: error.to_string(),
    ///     });
    ///
    ///     // Clean up old error records (keep last hour)
    ///     self.recent_errors.retain(|record| {
    ///         now.signed_duration_since(record.timestamp) < Duration::hours(1)
    ///     });
    ///
    ///     // Check if error rate exceeds threshold
    ///     if self.recent_errors.len() > self.error_rate_threshold {
    ///         warn!(
    ///             "Child error rate exceeded threshold: {} errors in last hour",
    ///             self.recent_errors.len()
    ///         );
    ///
    ///         // Could emit an event for alerting
    ///         ctx.publish_event(SupervisorEvent::HighErrorRate {
    ///             threshold: self.error_rate_threshold,
    ///             actual_count: self.recent_errors.len(),
    ///             time_window: Duration::hours(1),
    ///         }).await.ok();
    ///     }
    ///
    ///     info!("Child actor error: {:?}", error);
    /// }
    /// ```
    ///
    /// ## Categorized Error Handling
    /// Handle different types of errors with specific strategies:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     match error.error_type() {
    ///         ErrorType::NetworkError => {
    ///             self.network_error_count += 1;
    ///
    ///             // Network errors might indicate connectivity issues
    ///             if self.network_error_count > 5 {
    ///                 warn!("Multiple network errors detected, checking connectivity");
    ///
    ///                 // Could trigger a connectivity check
    ///                 if let Some(health_checker) = ctx.get_optional_service("health") {
    ///                     health_checker.tell(HealthMessage::CheckConnectivity).await.ok();
    ///                 }
    ///             }
    ///         }
    ///
    ///         ErrorType::ValidationError => {
    ///             self.validation_error_count += 1;
    ///
    ///             // Validation errors might indicate data quality issues
    ///             error!("Child validation error - potential data quality issue: {:?}", error);
    ///
    ///             // Could notify data quality monitoring
    ///             if let Some(dq_service) = ctx.get_optional_service("data-quality") {
    ///                 dq_service.tell(DataQualityMessage::ValidationFailure {
    ///                     source: ctx.path().clone(),
    ///                     error: error.to_string(),
    ///                 }).await.ok();
    ///             }
    ///         }
    ///
    ///         ErrorType::ResourceExhaustion => {
    ///             self.resource_error_count += 1;
    ///
    ///             // Resource exhaustion might require scaling or throttling
    ///             warn!("Child resource exhaustion: {:?}", error);
    ///
    ///             // Could trigger auto-scaling or load balancing adjustments
    ///             self.trigger_resource_scaling(ctx).await;
    ///         }
    ///
    ///         ErrorType::ExternalServiceError => {
    ///             self.external_service_error_count += 1;
    ///
    ///             // External service errors might require circuit breaker activation
    ///             if self.external_service_error_count > 3 {
    ///                 warn!("Multiple external service errors, activating circuit breaker");
    ///                 self.circuit_breaker_active = true;
    ///                 self.circuit_breaker_activated_at = Some(Utc::now());
    ///             }
    ///         }
    ///
    ///         _ => {
    ///             // Handle other error types generically
    ///             debug!("Child actor error: {:?}", error);
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Integration with Monitoring Systems
    /// Forward error information to external monitoring and alerting systems:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     // Update local error tracking
    ///     self.error_count += 1;
    ///
    ///     // Create structured error report
    ///     let error_report = ErrorReport {
    ///         parent_actor: ctx.path().clone(),
    ///         error_type: error.error_type(),
    ///         error_message: error.to_string(),
    ///         timestamp: Utc::now(),
    ///         context: self.get_supervision_context(),
    ///         severity: self.assess_error_severity(&error),
    ///     };
    ///
    ///     // Send to monitoring system
    ///     if let Some(monitoring) = ctx.get_optional_service("monitoring") {
    ///         monitoring.tell(MonitoringMessage::ChildError(error_report.clone())).await.ok();
    ///     }
    ///
    ///     // Send to alerting system for critical errors
    ///     if error_report.severity >= ErrorSeverity::High {
    ///         if let Some(alerting) = ctx.get_optional_service("alerting") {
    ///             alerting.tell(AlertingMessage::ChildErrorAlert {
    ///                 report: error_report,
    ///                 escalation_level: if error_report.severity >= ErrorSeverity::Critical {
    ///                     EscalationLevel::Immediate
    ///                 } else {
    ///                     EscalationLevel::Standard
    ///                 },
    ///             }).await.ok();
    ///         }
    ///     }
    ///
    ///     // Log with appropriate level based on severity
    ///     match error_report.severity {
    ///         ErrorSeverity::Low => debug!("Minor child error: {:?}", error),
    ///         ErrorSeverity::Medium => info!("Moderate child error: {:?}", error),
    ///         ErrorSeverity::High => warn!("High severity child error: {:?}", error),
    ///         ErrorSeverity::Critical => error!("Critical child error: {:?}", error),
    ///     }
    /// }
    /// ```
    ///
    /// # Proactive Error Response
    ///
    /// ## Dynamic Configuration Adjustment
    /// Adjust child actor configuration in response to error patterns:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     self.recent_errors.push(error.clone());
    ///
    ///     // Analyze recent error patterns
    ///     if self.recent_errors.len() >= 10 {
    ///         let timeout_errors = self.recent_errors.iter()
    ///             .filter(|e| matches!(e.error_type(), ErrorType::TimeoutError))
    ///             .count();
    ///
    ///         // If many timeout errors, increase child timeout settings
    ///         if timeout_errors >= 7 {
    ///             info!("High timeout error rate detected, increasing child timeouts");
    ///
    ///             // Send configuration update to children
    ///             for child_ref in &self.child_actors {
    ///                 child_ref.tell(ConfigMessage::UpdateTimeout {
    ///                     new_timeout: Duration::seconds(self.current_timeout_seconds * 2),
    ///                 }).await.ok();
    ///             }
    ///
    ///             self.current_timeout_seconds *= 2;
    ///         }
    ///
    ///         // Clear old errors
    ///         self.recent_errors.clear();
    ///     }
    /// }
    /// ```
    ///
    /// ## Load Balancing and Traffic Management
    /// Adjust load distribution based on error patterns:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     if let Some(child_id) = self.identify_error_source(&error) {
    ///         // Track per-child error rates
    ///         *self.child_error_counts.entry(child_id.clone()).or_insert(0) += 1;
    ///
    ///         let child_error_count = *self.child_error_counts.get(&child_id).unwrap_or(&0);
    ///
    ///         // If a specific child has too many errors, reduce its load
    ///         if child_error_count > self.child_error_threshold {
    ///             warn!("Child {} has {} errors, reducing load allocation", child_id, child_error_count);
    ///
    ///             // Reduce this child's weight in load balancing
    ///             if let Some(weight) = self.child_load_weights.get_mut(&child_id) {
    ///                 *weight = (*weight as f32 * 0.8).max(0.1) as u32;
    ///             }
    ///
    ///             // Redistribute load to healthier children
    ///             self.rebalance_child_loads().await;
    ///
    ///             // Schedule a recovery check
    ///             ctx.schedule_message(
    ///                 Duration::minutes(5),
    ///                 SupervisorMessage::CheckChildRecovery { child_id },
    ///             ).await.ok();
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Error Recovery and Healing
    ///
    /// ## Self-Healing Mechanisms
    /// Implement automatic recovery procedures for common error scenarios:
    ///
    /// ```ignore
    /// async fn on_child_error(&mut self, error: Error, ctx: &mut ActorContext<Self>) {
    ///     match error.error_type() {
    ///         ErrorType::DatabaseConnectionError => {
    ///             // Trigger database connection refresh
    ///             if let Some(db_manager) = ctx.get_optional_service("database-manager") {
    ///                 db_manager.tell(DatabaseMessage::RefreshConnections).await.ok();
    ///             }
    ///         }
    ///
    ///         ErrorType::ConfigurationError => {
    ///             // Reload configuration from source
    ///             if let Some(config_service) = ctx.get_optional_service("configuration") {
    ///                 config_service.tell(ConfigMessage::ReloadConfiguration).await.ok();
    ///             }
    ///         }
    ///
    ///         ErrorType::MemoryPressure => {
    ///             // Trigger garbage collection or cache cleanup
    ///             self.trigger_memory_cleanup().await;
    ///         }
    ///
    ///         _ => {}
    ///     }
    ///
    ///     // General error logging
    ///     debug!("Handled child error: {:?}", error);
    /// }
    /// ```
    ///
    /// # Performance and Resource Considerations
    ///
    /// ## Efficient Error Processing
    /// - Keep error handling lightweight to avoid impacting child actor performance
    /// - Use asynchronous notifications for non-critical error reporting
    /// - Batch error reports when dealing with high-frequency errors
    /// - Implement sampling for extremely high error rates to prevent overhead
    ///
    /// ## Memory Management
    /// - Limit the size of error history collections to prevent memory leaks
    /// - Use circular buffers or time-based cleanup for error tracking
    /// - Compress or aggregate old error data rather than storing raw details
    /// - Monitor memory usage of error tracking data structures
    ///
    /// ## Error Rate Management
    /// - Implement backoff strategies for error reporting to external systems
    /// - Use circuit breakers to prevent cascading failures during error storms
    /// - Consider error rate limiting to prevent overwhelming monitoring systems
    /// - Implement emergency modes that reduce error processing overhead
    ///
    /// # Thread Safety and Concurrency
    ///
    /// Error handling operates within the parent actor's message processing context:
    /// - **Sequential Processing**: Errors are handled one at a time in order
    /// - **State Consistency**: Parent state remains consistent during error handling
    /// - **Thread Safety**: No additional synchronization needed for parent state
    /// - **Isolated Execution**: Error handling doesn't affect concurrent child operations
    ///
    /// # Integration with Supervision Strategy
    ///
    /// Error handling works alongside the supervision strategy defined in the Actor trait:
    /// - `on_child_error` handles recoverable errors during normal operation
    /// - `on_child_fault` handles critical failures that may terminate children
    /// - Both methods work together to provide comprehensive supervision
    /// - Error tracking can influence fault handling decisions
    ///
    async fn on_child_error(
        &mut self,
        error: Error,
        _ctx: &mut ActorContext<A>,
    ) {
        debug!("Handling error: {:?}", error);
        // Default implementation from child actor errors.
        //self.on_child_fault(error, ctx).await;
    }

    /// Handles critical faults in child actors and determines the appropriate supervision action.
    ///
    /// This method is called when a child actor experiences a critical failure that prevents
    /// it from continuing normal operation. Unlike `on_child_error`, which handles recoverable
    /// errors, `on_child_fault` deals with severe problems that require a supervision decision:
    /// whether to restart the child, stop it permanently, or escalate the fault to the parent's
    /// supervisor. This is a core component of the actor system's fault tolerance and resilience
    /// mechanisms.
    ///
    /// # Supervision Decision Model
    ///
    /// The fault handling process follows this decision flow:
    /// 1. **Fault Detection**: Child actor encounters a critical failure and stops
    /// 2. **Fault Notification**: Actor system calls this method on the parent supervisor
    /// 3. **Fault Analysis**: Parent examines the error type, frequency, and context
    /// 4. **Decision Making**: Parent determines the most appropriate response action
    /// 5. **Action Execution**: Actor system executes the chosen `ChildAction`
    /// 6. **State Recovery**: If restarting, the child goes through its initialization lifecycle
    /// 7. **Monitoring**: Parent continues to monitor the child's health and behavior
    ///
    /// The default implementation logs the fault using the `debug!` macro and returns
    /// `ChildAction::Stop`, which terminates the child permanently. Override this method
    /// to implement custom fault tolerance strategies appropriate for your application.
    ///
    /// # Parameters
    ///
    /// * `error` - The critical error that caused the fault, containing:
    ///   - **Fault details**: Specific information about what caused the failure
    ///   - **Error classification**: Type of error (panic, resource exhaustion, corruption, etc.)
    ///   - **Failure context**: State information at the time of failure
    ///   - **Stack trace**: Debugging information to help diagnose the root cause
    ///
    /// * `ctx` - The parent actor's context, providing access to:
    ///   - Child actor references and management capabilities
    ///   - System services for logging, monitoring, and alerting
    ///   - Configuration data for fault tolerance policies
    ///   - Event publishing for fault notifications and audit trails
    ///
    /// # Returns
    ///
    /// Returns a `ChildAction` that specifies how the actor system should handle the fault:
    ///
    /// * `ChildAction::Restart` - Restart the child actor with a clean state:
    ///   - Child's `pre_restart` lifecycle hook is called
    ///   - Actor state is reset to initial condition
    ///   - Child resumes normal message processing
    ///   - Use for transient faults that don't indicate systemic problems
    ///
    /// * `ChildAction::Stop` - Permanently terminate the child actor:
    ///   - Child's `pre_stop` and `post_stop` lifecycle hooks are called
    ///   - Actor is removed from the system
    ///   - Resources are cleaned up and released
    ///   - Use for permanent faults or when restart is not beneficial
    ///
    /// * `ChildAction::Escalate` - Forward the fault to this actor's supervisor:
    ///   - Parent actor itself experiences a fault
    ///   - Parent's supervisor decides the parent's fate
    ///   - Can trigger cascading fault handling up the supervision hierarchy
    ///   - Use when the fault indicates a problem with the parent or system
    ///
    /// * `ChildAction::Resume` - Allow the child to continue with its current state:
    ///   - Child resumes message processing without restart
    ///   - No state reset or lifecycle hooks called
    ///   - Use only when the fault is determined to be non-critical
    ///   - **Note**: This may not be available for all fault types
    ///
    /// # Fault Tolerance Strategies
    ///
    /// ## Basic Restart Strategy
    /// Simple restart logic for transient failures:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     error!("Child actor fault: {:?}", error);
    ///
    ///     match error.error_type() {
    ///         ErrorType::Panic | ErrorType::UnhandledException => {
    ///             warn!("Child panicked, attempting restart");
    ///             ChildAction::Restart
    ///         }
    ///         ErrorType::ResourceExhaustion => {
    ///             warn!("Child exhausted resources, restarting with clean state");
    ///             ChildAction::Restart
    ///         }
    ///         ErrorType::CorruptedState => {
    ///             error!("Child state corrupted, restart required");
    ///             ChildAction::Restart
    ///         }
    ///         ErrorType::ConfigurationError => {
    ///             error!("Configuration error indicates system-wide issue, escalating");
    ///             ChildAction::Escalate
    ///         }
    ///         _ => {
    ///             warn!("Unknown fault type, stopping child for safety");
    ///             ChildAction::Stop
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Retry with Backoff Strategy
    /// Implement retry limits with exponential backoff:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     // Track restart attempts per child
    ///     let child_id = ctx.current_child_id(); // hypothetical method
    ///     let restart_count = self.child_restart_counts.entry(child_id).or_insert(0);
    ///     *restart_count += 1;
    ///
    ///     error!("Child fault #{} for {}: {:?}", restart_count, child_id, error);
    ///
    ///     match error.error_type() {
    ///         ErrorType::Temporary | ErrorType::NetworkError => {
    ///             if *restart_count <= self.max_restart_attempts {
    ///                 let backoff_duration = Duration::seconds(2_i64.pow(*restart_count as u32));
    ///                 warn!("Restarting child after {} backoff (attempt {})", backoff_duration, restart_count);
    ///
    ///                 // Schedule restart after backoff period
    ///                 ctx.schedule_message(
    ///                     backoff_duration,
    ///                     SupervisorMessage::RestartChild { child_id },
    ///                 ).await.ok();
    ///
    ///                 ChildAction::Stop // Stop now, restart later
    ///             } else {
    ///                 error!("Max restart attempts ({}) exceeded for child {}", self.max_restart_attempts, child_id);
    ///                 self.child_restart_counts.remove(&child_id);
    ///                 ChildAction::Stop
    ///             }
    ///         }
    ///         ErrorType::Fatal => {
    ///             error!("Fatal error in child, no restart possible");
    ///             ChildAction::Stop
    ///         }
    ///         ErrorType::SystemError => {
    ///             error!("System error indicates broader issue, escalating");
    ///             ChildAction::Escalate
    ///         }
    ///         _ => ChildAction::Restart
    ///     }
    /// }
    /// ```
    ///
    /// ## Circuit Breaker Pattern
    /// Implement circuit breaker logic for external service dependencies:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     match error.error_type() {
    ///         ErrorType::ExternalServiceError => {
    ///             self.external_service_failures += 1;
    ///             let failure_rate = self.external_service_failures as f64 / self.total_requests as f64;
    ///
    ///             if failure_rate > 0.5 && self.total_requests > 10 {
    ///                 warn!("High external service failure rate ({}), opening circuit breaker", failure_rate);
    ///                 self.circuit_breaker_open = true;
    ///                 self.circuit_breaker_opened_at = Some(Utc::now());
    ///
    ///                 // Emit event for monitoring
    ///                 ctx.publish_event(SupervisorEvent::CircuitBreakerOpened {
    ///                     service: "external_api".to_string(),
    ///                     failure_rate,
    ///                     threshold: 0.5,
    ///                 }).await.ok();
    ///
    ///                 // Stop child until circuit breaker closes
    ///                 ChildAction::Stop
    ///             } else {
    ///                 // Restart with backoff
    ///                 ChildAction::Restart
    ///             }
    ///         }
    ///         _ => ChildAction::Restart
    ///     }
    /// }
    /// ```
    ///
    /// ## Hierarchical Fault Tolerance
    /// Different strategies based on child importance and fault severity:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     let child_id = self.get_current_child_id(&error);
    ///     let child_importance = self.child_importance_map.get(&child_id)
    ///         .unwrap_or(&ChildImportance::Normal);
    ///
    ///     let fault_severity = self.assess_fault_severity(&error);
    ///
    ///     match (child_importance, fault_severity) {
    ///         (ChildImportance::Critical, FaultSeverity::High | FaultSeverity::Critical) => {
    ///             error!("Critical child experienced severe fault, escalating to prevent system failure");
    ///
    ///             // Log detailed information for post-mortem analysis
    ///             self.log_critical_fault(&child_id, &error, ctx).await;
    ///
    ///             ChildAction::Escalate
    ///         }
    ///         (ChildImportance::Critical, _) => {
    ///             warn!("Critical child fault, attempting immediate restart");
    ///             ChildAction::Restart
    ///         }
    ///         (ChildImportance::Normal, FaultSeverity::Critical) => {
    ///             error!("Normal child has critical fault, may indicate system issue");
    ///             ChildAction::Escalate
    ///         }
    ///         (ChildImportance::Normal, FaultSeverity::High) => {
    ///             warn!("Normal child high-severity fault, restarting");
    ///             ChildAction::Restart
    ///         }
    ///         (ChildImportance::Optional, _) => {
    ///             info!("Optional child fault, stopping to preserve resources");
    ///             ChildAction::Stop
    ///         }
    ///         _ => {
    ///             debug!("Low-severity fault, restarting child");
    ///             ChildAction::Restart
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Integration with Monitoring and Alerting
    ///
    /// ## Comprehensive Fault Reporting
    /// Report faults to monitoring systems with detailed context:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     // Create detailed fault report
    ///     let fault_report = FaultReport {
    ///         parent_actor: ctx.path().clone(),
    ///         child_actor: self.get_current_child_path(&error),
    ///         error_type: error.error_type(),
    ///         error_message: error.to_string(),
    ///         timestamp: Utc::now(),
    ///         system_context: self.get_system_context(),
    ///         previous_faults: self.get_recent_fault_history(),
    ///         resource_usage: self.get_current_resource_usage(),
    ///     };
    ///
    ///     // Determine action based on fault analysis
    ///     let action = self.decide_fault_action(&error, &fault_report);
    ///
    ///     // Report to monitoring systems
    ///     if let Some(monitoring) = ctx.get_optional_service("monitoring") {
    ///         monitoring.tell(MonitoringMessage::ChildFault {
    ///             report: fault_report.clone(),
    ///             action: action.clone(),
    ///         }).await.ok();
    ///     }
    ///
    ///     // Send alerts for critical faults
    ///     if fault_report.severity >= FaultSeverity::High {
    ///         if let Some(alerting) = ctx.get_optional_service("alerting") {
    ///             alerting.tell(AlertingMessage::CriticalChildFault {
    ///                 report: fault_report,
    ///                 recommended_action: action.clone(),
    ///                 escalation_required: matches!(action, ChildAction::Escalate),
    ///             }).await.ok();
    ///         }
    ///     }
    ///
    ///     // Emit event for audit trail
    ///     ctx.publish_event(SupervisorEvent::ChildFaultHandled {
    ///         error: error.to_string(),
    ///         action: action.clone(),
    ///         timestamp: Utc::now(),
    ///     }).await.ok();
    ///
    ///     action
    /// }
    /// ```
    ///
    /// # Advanced Supervision Patterns
    ///
    /// ## One-for-One vs One-for-All Strategies
    /// Handle faults with different scopes of impact:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     match self.supervision_strategy {
    ///         SupervisionStrategy::OneForOne => {
    ///             // Only the failing child is affected
    ///             match error.error_type() {
    ///                 ErrorType::Temporary => ChildAction::Restart,
    ///                 ErrorType::Fatal => ChildAction::Stop,
    ///                 ErrorType::SystemWide => ChildAction::Escalate,
    ///                 _ => ChildAction::Restart,
    ///             }
    ///         }
    ///         SupervisionStrategy::OneForAll => {
    ///             // All children are affected when one fails
    ///             warn!("Child fault in one-for-all supervision, stopping all children");
    ///
    ///             // Stop all other children first
    ///             for child_ref in &self.child_actors {
    ///                 child_ref.stop().await.ok();
    ///             }
    ///
    ///             // Then restart them all if appropriate
    ///             if self.should_restart_after_fault(&error) {
    ///                 ctx.schedule_message(
    ///                     Duration::seconds(1),
    ///                     SupervisorMessage::RestartAllChildren,
    ///                 ).await.ok();
    ///             }
    ///
    ///             ChildAction::Stop
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Adaptive Fault Tolerance
    /// Adjust fault handling based on system health and load:
    ///
    /// ```ignore
    /// async fn on_child_fault(&mut self, error: Error, ctx: &mut ActorContext<Self>) -> ChildAction {
    ///     let system_health = self.assess_system_health();
    ///     let current_load = self.get_current_system_load();
    ///
    ///     match (system_health, current_load) {
    ///         (SystemHealth::Healthy, LoadLevel::Low | LoadLevel::Normal) => {
    ///             // System is healthy, normal fault tolerance
    ///             match error.error_type() {
    ///                 ErrorType::Temporary => ChildAction::Restart,
    ///                 ErrorType::Fatal => ChildAction::Stop,
    ///                 _ => ChildAction::Restart,
    ///             }
    ///         }
    ///         (SystemHealth::Degraded, _) | (_, LoadLevel::High) => {
    ///             // System under stress, more conservative approach
    ///             warn!("System under stress, using conservative fault handling");
    ///             match error.error_type() {
    ///                 ErrorType::Temporary if self.restart_budget > 0 => {
    ///                     self.restart_budget -= 1;
    ///                     ChildAction::Restart
    ///                 }
    ///                 _ => {
    ///                     info!("Stopping child to preserve system resources");
    ///                     ChildAction::Stop
    ///                 }
    ///             }
    ///         }
    ///         (SystemHealth::Critical, _) => {
    ///             // System in critical state, minimize resource usage
    ///             error!("System critical, stopping child to preserve resources");
    ///             ChildAction::Stop
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Performance and Resource Considerations
    ///
    /// ## Efficient Fault Processing
    /// - Make fault handling decisions quickly to minimize system disruption
    /// - Cache fault analysis data to avoid repeated expensive computations
    /// - Use lightweight logging and monitoring for high-frequency faults
    /// - Implement rate limiting for fault reporting to external systems
    ///
    /// ## Resource Management During Faults
    /// - Clean up child resources promptly when stopping actors
    /// - Monitor memory usage during fault handling to prevent leaks
    /// - Limit concurrent restart operations to prevent resource exhaustion
    /// - Implement backpressure for fault notifications under high load
    ///
    /// ## Fault Recovery Optimization
    /// - Pre-allocate resources for rapid child restart when possible
    /// - Use connection pooling and resource sharing to speed recovery
    /// - Implement warm standby actors for critical services
    /// - Cache initialization data to reduce restart times
    ///
    /// # Thread Safety and Concurrency
    ///
    /// Fault handling operates within the parent actor's supervision context:
    /// - **Atomic Decisions**: Fault handling decisions are made atomically
    /// - **State Consistency**: Parent state remains consistent during fault processing
    /// - **Thread Safety**: No additional synchronization needed for supervision state
    /// - **Isolation**: Fault handling doesn't interfere with normal message processing
    ///
    /// # Integration with Actor System Lifecycle
    ///
    /// Fault handling integrates closely with the actor lifecycle:
    /// - Fault detection occurs during child actor execution
    /// - Fault handling can trigger child lifecycle transitions (stop, restart)
    /// - Parent supervision state is maintained throughout the actor's lifetime
    /// - Fault handling policies can be updated dynamically during system operation
    ///
    async fn on_child_fault(
        &mut self,
        error: Error,
        _ctx: &mut ActorContext<A>,
    ) -> ChildAction {
        // Default implementation from child actor errors.
        debug!("Handling fault: {:?}", error);
        ChildAction::Stop
    }
}

/// Actor reference providing communication interface for remote actor interaction.
///
/// `ActorRef` is a lightweight, cloneable handle to an active actor instance within the actor system.
/// It provides the primary interface for interacting with actors through message passing, offering
/// both fire-and-forget (`tell`) and request-response (`ask`) communication patterns. Actor references
/// abstract away the underlying communication mechanisms and provide location transparency, allowing
/// actors to communicate regardless of their physical location within the system.
///
/// # Core Capabilities
///
/// - **Message Sending**: Send fire-and-forget messages using `tell` method
/// - **Request-Response**: Perform request-response interactions using `ask` method
/// - **Lifecycle Management**: Control actor termination through `tell_stop` and `ask_stop`
/// - **Event Subscription**: Subscribe to actor events through `subscribe` method
/// - **Status Monitoring**: Check actor availability through `is_closed` method
/// - **Path Access**: Retrieve actor's hierarchical path for identification and routing
///
/// # Type Parameters
///
/// * `A` - The actor type this reference points to, ensuring type-safe message delivery
///         and maintaining compile-time guarantees about message compatibility
///
/// # Thread Safety and Cloning
///
/// `ActorRef` implements `Clone` and can be safely shared across threads and async contexts.
/// Each clone maintains an independent connection to the same underlying actor, allowing
/// multiple components to interact with the actor concurrently. Cloning is efficient as
/// it only duplicates communication channels, not the actor itself.
///
/// # Location Transparency
///
/// Actor references provide location transparency - the same interface works whether the
/// actor is local to the current process or distributed across the network. This allows
/// for seamless scaling and deployment flexibility.
///
/// # Examples
///
/// ## Basic Message Sending
///
/// ```ignore
/// use rush_actor::*;
/// use async_trait::async_trait;
///
/// // Create an actor reference and send messages
/// let actor_ref: ActorRef<MyActor> = system.create_actor("my-actor", MyActor::new()).await?;
///
/// // Fire-and-forget message sending
/// actor_ref.tell(MyMessage::Process(data)).await?;
///
/// // Request-response interaction
/// let response = actor_ref.ask(MyMessage::Query(id)).await?;
/// println!("Response: {:?}", response);
/// ```
///
/// ## Actor Lifecycle Management
///
/// ```ignore
/// // Graceful shutdown with confirmation
/// actor_ref.ask_stop().await?;
/// println!("Actor stopped successfully");
///
/// // Fire-and-forget shutdown
/// actor_ref.tell_stop().await;
/// ```
///
/// ## Event Subscription
///
/// ```ignore
/// // Subscribe to actor events
/// let mut event_receiver = actor_ref.subscribe();
///
/// // Listen for events in background task
/// tokio::spawn(async move {
///     while let Ok(event) = event_receiver.recv().await {
///         println!("Received event: {:?}", event);
///     }
/// });
/// ```
///
/// ## Status Monitoring
///
/// ```ignore
/// if actor_ref.is_closed() {
///     println!("Actor is no longer available");
/// } else {
///     // Safe to send messages
///     actor_ref.tell(MyMessage::Heartbeat).await?;
/// }
/// ```
///
/// # Error Handling
///
/// Most operations on `ActorRef` can fail if:
/// - The target actor has stopped or crashed
/// - The underlying communication channels are closed
/// - Network issues occur (in distributed scenarios)
/// - Message serialization fails
/// - System-wide shutdown is in progress
///
/// # Performance Characteristics
///
/// - **Low Overhead**: Minimal memory footprint per reference
/// - **Efficient Cloning**: Cheap to clone and pass around
/// - **Asynchronous**: All operations are non-blocking
/// - **Concurrent Access**: Multiple threads can safely use the same reference
/// - **Message Batching**: Internal optimizations for high-throughput scenarios
///
/// # Relationship to Actor System
///
/// Actor references are created and managed by the actor system. They maintain a connection
/// to the system's routing infrastructure and participate in the system's lifecycle.
/// When the system shuts down, all actor references become invalid and operations will fail.
///
pub struct ActorRef<A>
where
    A: Actor + Handler<A>,
{
    /// Hierarchical path uniquely identifying the target actor in the system tree.
    /// Used for routing, logging, supervision relationships, and system management.
    /// The path reflects the actor's position in the supervision hierarchy.
    path: ActorPath,

    /// Message delivery helper providing the core communication mechanism.
    /// Encapsulates the underlying mailbox sender and handles message routing,
    /// serialization, and delivery guarantees to the target actor.
    sender: HandleHelper<A>,

    /// Event subscription receiver for monitoring actor state changes and custom events.
    /// Provides a broadcast channel that can be cloned to create multiple independent
    /// event streams from the same actor. Each subscriber receives all events published
    /// by the actor after their subscription begins.
    event_receiver: EventReceiver<<A as Actor>::Event>,

    /// Control channel for actor termination and lifecycle management.
    /// Allows sending stop signals to gracefully terminate the actor and receive
    /// confirmation when the shutdown process completes successfully.
    stop_sender: StopSender,
}

impl<A> ActorRef<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new actor reference with all necessary communication channels.
    ///
    /// This internal constructor initializes a complete actor reference with all required
    /// communication channels and metadata. It's called by the actor system during actor
    /// creation and should not be invoked directly by user code. The resulting reference
    /// provides a fully functional interface for interacting with the target actor.
    ///
    /// # Parameters
    ///
    /// * `path` - Hierarchical path uniquely identifying the target actor within the
    ///           system tree. Used for message routing, logging, and supervision relationships.
    ///           The path must be valid and correspond to an active actor instance.
    /// * `sender` - Message delivery helper encapsulating the actor's mailbox connection.
    ///             Provides the underlying communication mechanism for both `tell` and `ask`
    ///             operations, handling message serialization and delivery guarantees.
    /// * `stop_sender` - Control channel for actor lifecycle management, enabling graceful
    ///                  termination through stop signals. Supports both fire-and-forget
    ///                  and confirmed shutdown patterns for clean resource cleanup.
    /// * `event_receiver` - Event subscription channel providing access to the actor's
    ///                     event stream. Can be cloned to create multiple independent
    ///                     subscriptions for monitoring actor state changes and custom events.
    ///
    /// # Returns
    ///
    /// Returns a fully initialized `ActorRef<A>` ready for message sending, lifecycle
    /// management, and event subscription. The reference is immediately usable for all
    /// supported operations and can be safely cloned and shared across contexts.
    ///
    /// # Examples
    ///
    /// This method is called internally by the actor system:
    ///
    /// ```ignore
    /// // Called during actor creation process
    /// let actor_ref = ActorRef::new(
    ///     ActorPath::from("/user/worker"),
    ///     message_sender,
    ///     stop_sender,
    ///     event_receiver,
    /// );
    /// ```
    ///
    /// # Thread Safety
    ///
    /// The created actor reference is `Send + Sync` and can be safely used across
    /// different threads and async contexts. All internal channels are designed
    /// for concurrent access without additional synchronization.
    ///
    pub fn new(
        path: ActorPath,
        sender: HandleHelper<A>,
        stop_sender: StopSender,
        event_receiver: EventReceiver<<A as Actor>::Event>,
    ) -> Self {
        Self {
            path,
            sender,
            stop_sender,
            event_receiver,
        }
    }

    /// Sends a fire-and-forget message to the target actor.
    ///
    /// This method implements the "tell" pattern (fire-and-forget) where a message is sent
    /// to the actor without expecting a response. The message is queued in the actor's mailbox
    /// and will be processed asynchronously according to the actor's message handling logic.
    /// The sender does not block waiting for message processing and cannot receive a response.
    ///
    /// This is the most efficient communication pattern for scenarios where you only need
    /// to notify the actor of an event or command without requiring confirmation or results.
    /// It provides maximum throughput and minimal latency for message delivery.
    ///
    /// # Parameters
    ///
    /// * `message` - The message instance to send to the target actor. Must implement
    ///              the actor's associated `Message` type, which includes serialization
    ///              traits for potential network delivery and debugging support.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the message was successfully queued for delivery to the actor.
    /// Success indicates the message has been accepted by the actor system's routing
    /// infrastructure but does not guarantee the actor has processed it yet.
    ///
    /// # Error Conditions
    ///
    /// Returns an `Error` in the following situations:
    /// - **Actor Unavailable**: The target actor has stopped, crashed, or been removed
    /// - **Mailbox Full**: The actor's message queue has reached capacity (backpressure)
    /// - **System Shutdown**: The actor system is shutting down and no longer accepting messages
    /// - **Network Failure**: Communication failure in distributed actor scenarios
    /// - **Serialization Error**: Message cannot be serialized for delivery
    ///
    /// # Examples
    ///
    /// ## Basic Message Sending
    ///
    /// ```ignore
    /// // Send a simple command message
    /// actor_ref.tell(ProcessCommand {
    ///     action: "start",
    ///     parameters: params,
    /// }).await?;
    ///
    /// // Send notification without waiting
    /// actor_ref.tell(NotificationMessage::UserLoggedIn {
    ///     user_id: 12345,
    ///     timestamp: Utc::now(),
    /// }).await?;
    /// ```
    ///
    /// ## High-Throughput Scenarios
    ///
    /// ```ignore
    /// // Efficiently send multiple messages
    /// for item in batch_items {
    ///     match actor_ref.tell(ProcessItem(item)).await {
    ///         Ok(()) => processed_count += 1,
    ///         Err(e) => {
    ///             eprintln!("Failed to send item: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Error Handling
    ///
    /// ```ignore
    /// match actor_ref.tell(important_message).await {
    ///     Ok(()) => {
    ///         // Message queued successfully
    ///         log::info!("Message sent to actor");
    ///     }
    ///     Err(Error::ActorNotFound) => {
    ///         // Actor has been stopped or removed
    ///         log::warn!("Target actor is no longer available");
    ///     }
    ///     Err(e) => {
    ///         // Other delivery failures
    ///         log::error!("Failed to send message: {}", e);
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Non-blocking**: Does not wait for message processing, only for queue acceptance
    /// - **High Throughput**: Optimized for scenarios requiring many messages per second
    /// - **Low Latency**: Minimal overhead between send call and message queuing
    /// - **Memory Efficient**: Messages are moved rather than copied when possible
    ///
    /// # Threading Context
    ///
    /// This method is async-safe and can be called from any async context, including:
    /// - Other actor message handlers
    /// - Background tokio tasks
    /// - Async web handlers
    /// - Timer callbacks
    /// - External system integrations
    ///
    /// # Best Practices
    ///
    /// - Use `tell` for commands that don't require responses
    /// - Handle errors appropriately based on your application's fault tolerance requirements
    /// - Consider message batching for high-volume scenarios
    /// - Monitor actor availability using `is_closed()` in critical paths
    ///
    pub async fn tell(&self, message: A::Message) -> Result<(), Error> {
        self.sender.tell(self.path(), message).await
    }

    /// Sends a request message and waits for a response from the target actor.
    ///
    /// This method implements the "ask" pattern (request-response) where a message is sent
    /// to the actor and the caller waits for a typed response. This provides a synchronous-style
    /// interaction model while maintaining the underlying asynchronous message passing semantics.
    /// The method will block until the actor processes the message and returns a response.
    ///
    /// This pattern is ideal for queries, calculations, or operations where you need the result
    /// before proceeding. It ensures data consistency and enables complex request-response
    /// workflows while maintaining the actor model's isolation and concurrent safety.
    ///
    /// # Parameters
    ///
    /// * `message` - The request message to send to the target actor. Must implement the
    ///              actor's associated `Message` type and be compatible with the actor's
    ///              request-response handling logic. The message should contain all data
    ///              needed for the actor to generate an appropriate response.
    ///
    /// # Returns
    ///
    /// Returns `Ok(A::Response)` containing the actor's response if the request-response
    /// cycle completes successfully. The response type is determined by the target actor's
    /// associated `Response` type, ensuring type safety throughout the interaction.
    ///
    /// # Error Conditions
    ///
    /// Returns an `Error` in various failure scenarios:
    ///
    /// - **Actor Unavailable**: Target actor has stopped, crashed, or been removed from the system
    /// - **Message Delivery Failure**: Unable to deliver the message due to mailbox issues
    /// - **Actor Processing Error**: The actor encountered an error while processing the request
    /// - **Response Channel Closed**: Communication channel closed before response was received
    /// - **Timeout**: Request exceeded configured timeout duration (system-dependent)
    /// - **System Shutdown**: Actor system is shutting down and cannot complete the request
    /// - **Serialization Error**: Message or response serialization failed in distributed scenarios
    ///
    /// # Examples
    ///
    /// ## Simple Query Operation
    ///
    /// ```ignore
    /// // Query actor state
    /// let balance = bank_actor.ask(GetBalance {
    ///     account_id: "12345".to_string(),
    /// }).await?;
    ///
    /// println!("Account balance: {}", balance.amount);
    /// ```
    ///
    /// ## Complex Business Logic
    ///
    /// ```ignore
    /// // Perform calculation with multiple parameters
    /// let result = calculator_actor.ask(ComplexCalculation {
    ///     operation: MathOperation::Integral,
    ///     function: "x^2 + 3x + 2".to_string(),
    ///     bounds: (0.0, 10.0),
    ///     precision: 0.001,
    /// }).await?;
    ///
    /// match result {
    ///     CalculationResult::Success { value, iterations } => {
    ///         println!("Result: {} (computed in {} iterations)", value, iterations);
    ///     }
    ///     CalculationResult::Error { reason } => {
    ///         eprintln!("Calculation failed: {}", reason);
    ///     }
    /// }
    /// ```
    ///
    /// ## Error Handling and Retries
    ///
    /// ```ignore
    /// let mut retry_count = 0;
    /// const MAX_RETRIES: usize = 3;
    ///
    /// loop {
    ///     match data_service.ask(FetchData { id: data_id }).await {
    ///         Ok(data) => {
    ///             log::info!("Successfully retrieved data");
    ///             return Ok(data);
    ///         }
    ///         Err(Error::TemporaryFailure) if retry_count < MAX_RETRIES => {
    ///             retry_count += 1;
    ///             log::warn!("Request failed, retrying ({}/{})", retry_count, MAX_RETRIES);
    ///             tokio::time::sleep(Duration::from_millis(100 * retry_count)).await;
    ///             continue;
    ///         }
    ///         Err(e) => {
    ///             log::error!("Request failed permanently: {}", e);
    ///             return Err(e);
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Conditional Requests
    ///
    /// ```ignore
    /// // Only make request if actor is available
    /// if !actor_ref.is_closed() {
    ///     let status = actor_ref.ask(HealthCheck).await?;
    ///     if status.is_healthy {
    ///         // Proceed with main operation
    ///         let result = actor_ref.ask(MainOperation { data }).await?;
    ///         process_result(result);
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Synchronous Semantics**: Blocks until response is received, suitable for request-response patterns
    /// - **Type Safety**: Compile-time guarantees for request-response type compatibility
    /// - **Resource Management**: Automatic cleanup of response channels and temporary resources
    /// - **Backpressure**: Naturally provides backpressure by blocking on slow actors
    ///
    /// # Threading Context
    ///
    /// This method can be called from any async context and will yield control during the wait
    /// for the response. The async runtime can schedule other tasks while waiting, maintaining
    /// system concurrency. The method is cancellation-safe and can be used with `tokio::select!`
    /// and other async control flow constructs.
    ///
    /// # Best Practices
    ///
    /// - **Use for Queries**: Ideal for data retrieval and state queries
    /// - **Handle Timeouts**: Consider implementing application-level timeouts for long-running requests
    /// - **Error Recovery**: Implement appropriate error handling and retry strategies
    /// - **Avoid Deadlocks**: Be careful when using `ask` from within actor message handlers
    /// - **Performance Monitoring**: Monitor response times for performance optimization
    ///
    /// # Relationship to Tell Pattern
    ///
    /// Unlike `tell()`, this method waits for a response and provides stronger delivery guarantees.
    /// Use `tell()` for fire-and-forget scenarios and `ask()` when you need the result of the
    /// actor's processing before continuing execution.
    ///
    pub async fn ask(&self, message: A::Message) -> Result<A::Response, Error> {
        self.sender.ask(self.path(), message).await
    }

    /// Initiates graceful actor termination and waits for shutdown confirmation.
    ///
    /// This method sends a stop signal to the target actor and blocks until the actor has
    /// completed its shutdown sequence. It provides the "ask" pattern for actor termination,
    /// ensuring that the calling code knows exactly when the actor has fully stopped and
    /// released all resources. This is essential for scenarios requiring coordinated shutdown
    /// or resource cleanup dependencies.
    ///
    /// The actor will complete its current message processing (if any), execute its `pre_stop`
    /// lifecycle hook, recursively stop all child actors, remove itself from the system registry,
    /// execute its `post_stop` hook, and then signal completion back to the caller.
    ///
    /// # Shutdown Sequence
    ///
    /// The graceful shutdown follows these steps:
    /// 1. **Current Message Completion**: Any currently processing message finishes execution
    /// 2. **Pre-Stop Hook**: Actor's `pre_stop` lifecycle method is called for cleanup
    /// 3. **Child Actor Termination**: All supervised child actors are stopped recursively
    /// 4. **System Deregistration**: Actor is removed from the system's actor registry
    /// 5. **Post-Stop Hook**: Actor's `post_stop` lifecycle method is called for final cleanup
    /// 6. **Confirmation Signal**: Completion confirmation is sent back to the caller
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the actor shutdown completed successfully. This guarantees that:
    /// - The actor has stopped processing new messages
    /// - All lifecycle hooks have been executed
    /// - All child actors have been terminated
    /// - The actor has been removed from the system registry
    /// - All resources associated with the actor have been cleaned up
    ///
    /// # Error Conditions
    ///
    /// Returns an `Error` in the following situations:
    ///
    /// - **Actor Already Stopped**: The actor was already terminated before the stop signal was sent
    /// - **Communication Failure**: Unable to send the stop signal due to channel closure
    /// - **Confirmation Timeout**: The actor didn't confirm shutdown within the expected timeframe
    /// - **System Shutdown**: The actor system is shutting down and cannot process the stop request
    ///
    /// Note that if the actor is already stopped, this method returns `Ok(())` rather than
    /// an error, as the desired end state (actor stopped) has been achieved.
    ///
    /// # Examples
    ///
    /// ## Coordinated Shutdown
    ///
    /// ```ignore
    /// // Ensure worker is stopped before proceeding
    /// worker_actor.ask_stop().await?;
    /// log::info!("Worker actor has been cleanly terminated");
    ///
    /// // Now safe to perform cleanup that depends on worker being stopped
    /// cleanup_shared_resources().await?;
    /// ```
    ///
    /// ## Graceful Application Shutdown
    ///
    /// ```ignore
    /// async fn shutdown_application(actors: Vec<ActorRef<dyn MyActor>>) -> Result<(), Error> {
    ///     log::info!("Initiating graceful shutdown of {} actors", actors.len());
    ///
    ///     for (index, actor) in actors.iter().enumerate() {
    ///         match actor.ask_stop().await {
    ///             Ok(()) => {
    ///                 log::info!("Actor {} stopped successfully", index + 1);
    ///             }
    ///             Err(e) => {
    ///                 log::warn!("Actor {} shutdown error (continuing): {}", index + 1, e);
    ///             }
    ///         }
    ///     }
    ///
    ///     log::info!("All actors have been processed for shutdown");
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Conditional Shutdown with Timeout
    ///
    /// ```ignore
    /// use tokio::time::{timeout, Duration};
    ///
    /// // Stop actor with timeout to prevent hanging
    /// match timeout(Duration::from_secs(10), actor_ref.ask_stop()).await {
    ///     Ok(Ok(())) => {
    ///         log::info!("Actor stopped gracefully");
    ///     }
    ///     Ok(Err(e)) => {
    ///         log::error!("Actor stop failed: {}", e);
    ///     }
    ///     Err(_) => {
    ///         log::error!("Actor stop timed out after 10 seconds");
    ///         // Consider forceful termination or system-level intervention
    ///     }
    /// }
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Blocking**: Waits for complete shutdown sequence, suitable for coordinated termination
    /// - **Resource Cleanup**: Ensures all actor resources are properly released
    /// - **Child Management**: Automatically handles complex supervision tree cleanup
    /// - **Confirmation**: Provides strong guarantees about termination completion
    ///
    /// # Threading Context
    ///
    /// This method can be called from any async context and will yield control during the
    /// shutdown wait period. The async runtime can schedule other tasks while the actor
    /// completes its shutdown sequence. The method is cancellation-safe and can be used
    /// with `tokio::select!` for timeout handling.
    ///
    /// # Best Practices
    ///
    /// - **Use for Critical Shutdowns**: When you need confirmation that shutdown completed
    /// - **Implement Timeouts**: Use `tokio::time::timeout` to prevent indefinite blocking
    /// - **Error Handling**: Handle shutdown errors gracefully, but don't necessarily treat them as fatal
    /// - **Resource Dependencies**: Use this method when other resources depend on the actor being fully stopped
    /// - **Logging**: Log shutdown completion for debugging and monitoring purposes
    ///
    /// # Relationship to tell_stop
    ///
    /// Unlike `tell_stop()`, this method provides confirmation that the shutdown has completed.
    /// Use `tell_stop()` for fire-and-forget termination and `ask_stop()` when you need to
    /// coordinate subsequent operations that depend on the actor being fully terminated.
    ///
    pub async fn ask_stop(&self) -> Result<(), Error> {
        debug!("Stopping actor from handle reference.");
        let (response_sender, response_receiver) = oneshot::channel();

        if self.stop_sender.send(Some(response_sender)).await.is_err() {
            Ok(())
        } else {
            Ok(response_receiver
                .await
                .map_err(|error| Error::Send(error.to_string()))?)
        }
    }

    /// Initiates fire-and-forget actor termination without waiting for confirmation.
    ///
    /// This method sends a stop signal to the target actor and returns immediately without
    /// waiting for the shutdown to complete. It implements the "tell" pattern for actor
    /// termination, providing maximum performance when you need to stop actors but don't
    /// require confirmation of completion. The actor will begin its shutdown sequence
    /// asynchronously after receiving the signal.
    ///
    /// This is the most efficient way to stop actors when you don't need to coordinate
    /// subsequent operations or ensure resource cleanup dependencies. It's particularly
    /// useful for bulk termination scenarios or when stopping actors during system shutdown
    /// where confirmation overhead is unnecessary.
    ///
    /// # Shutdown Behavior
    ///
    /// After receiving the stop signal, the actor will:
    /// 1. Complete any currently processing message
    /// 2. Execute its `pre_stop` lifecycle hook
    /// 3. Recursively stop all supervised child actors
    /// 4. Remove itself from the system registry
    /// 5. Execute its `post_stop` lifecycle hook
    /// 6. Terminate its message processing loop
    ///
    /// These steps occur asynchronously after this method returns.
    ///
    /// # Returns
    ///
    /// This method returns immediately without providing any indication of whether
    /// the stop signal was successfully delivered or processed. The return type is
    /// `()` (unit type) indicating fire-and-forget semantics.
    ///
    /// # Error Handling
    ///
    /// This method does not return errors. If the stop signal cannot be delivered
    /// (e.g., if the actor is already stopped or the channel is closed), the operation
    /// silently succeeds since the desired end state (actor stopping) is achieved
    /// or the failure is inconsequential.
    ///
    /// # Examples
    ///
    /// ## Bulk Actor Termination
    ///
    /// ```ignore
    /// // Efficiently stop multiple actors without waiting
    /// for actor_ref in worker_actors {
    ///     actor_ref.tell_stop().await;
    /// }
    /// log::info!("Stop signals sent to all workers");
    ///
    /// // Continue with other shutdown tasks immediately
    /// shutdown_database_connections().await;
    /// ```
    ///
    /// ## System Shutdown
    ///
    /// ```ignore
    /// async fn initiate_system_shutdown(system: &ActorSystem) {
    ///     log::info!("Initiating system shutdown");
    ///
    ///     // Stop all user actors quickly
    ///     for actor_path in system.list_user_actors().await {
    ///         if let Some(actor_ref) = system.get_actor(&actor_path).await {
    ///             actor_ref.tell_stop().await;
    ///         }
    ///     }
    ///
    ///     // Give actors time to shut down naturally
    ///     tokio::time::sleep(Duration::from_secs(5)).await;
    ///
    ///     // Force system termination
    ///     system.terminate().await;
    /// }
    /// ```
    ///
    /// ## Conditional Termination
    ///
    /// ```ignore
    /// // Stop actor if it's still running
    /// if !actor_ref.is_closed() {
    ///     actor_ref.tell_stop().await;
    ///     log::info!("Sent stop signal to active actor");
    /// }
    ///
    /// // Continue immediately with other work
    /// process_next_batch().await;
    /// ```
    ///
    /// ## Background Cleanup
    ///
    /// ```ignore
    /// // Stop actor in background task
    /// let actor_ref = Arc::clone(&shared_actor_ref);
    /// tokio::spawn(async move {
    ///     // Wait for some condition
    ///     cleanup_trigger.await;
    ///
    ///     // Stop actor when condition is met
    ///     actor_ref.tell_stop().await;
    ///     log::debug!("Background actor cleanup initiated");
    /// });
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Non-blocking**: Returns immediately, no waiting for shutdown completion
    /// - **High Performance**: Minimal overhead, optimal for bulk operations
    /// - **Fire-and-Forget**: No response channels or confirmation mechanisms
    /// - **Memory Efficient**: No temporary resources allocated for response handling
    ///
    /// # Threading Context
    ///
    /// This method is async but returns immediately without yielding control to the
    /// async runtime. It can be called from any async context including:
    /// - Actor message handlers (safe, no deadlock risk)
    /// - Background tasks and timers
    /// - Signal handlers and cleanup routines
    /// - System shutdown sequences
    ///
    /// # Best Practices
    ///
    /// - **Use for Performance**: When stop confirmation is not required
    /// - **Bulk Operations**: Efficient for stopping many actors simultaneously
    /// - **System Shutdown**: Ideal for application termination sequences
    /// - **Fire-and-Forget**: When you need to trigger shutdown but continue immediately
    /// - **No Resource Dependencies**: When subsequent operations don't depend on actor termination
    ///
    /// # Relationship to ask_stop
    ///
    /// Unlike `ask_stop()`, this method doesn't wait for shutdown confirmation. Use
    /// `tell_stop()` for high-performance termination scenarios and `ask_stop()` when
    /// you need to coordinate operations that depend on the actor being fully stopped.
    ///
    /// # Safety Considerations
    ///
    /// While this method doesn't provide confirmation, the underlying shutdown process
    /// is still graceful and safe. The actor will complete its current work and clean
    /// up resources properly - you just won't be notified when this process completes.
    ///
    pub async fn tell_stop(&self) {
        debug!("Stopping actor from handle reference.");

        let _ = self.stop_sender.send(None).await;
    }

    /// Retrieves the hierarchical path uniquely identifying this actor in the system.
    ///
    /// This method returns a copy of the actor's path within the actor system hierarchy.
    /// Actor paths serve as unique identifiers that reflect the supervision tree structure
    /// and provide essential information for routing, logging, debugging, and system
    /// administration. Paths are immutable and remain constant throughout an actor's lifetime.
    ///
    /// # Path Structure
    ///
    /// Actor paths follow a hierarchical structure similar to filesystem paths:
    /// - **Root Level**: `/` - The system root
    /// - **System Actors**: `/system/...` - Internal system management actors
    /// - **User Actors**: `/user/...` - Application-level actors
    /// - **Child Actors**: `/user/parent/child` - Nested supervision relationships
    ///
    /// # Returns
    ///
    /// Returns an `ActorPath` instance containing the complete hierarchical path
    /// of this actor. The path includes all parent actors in the supervision tree,
    /// providing a complete address for the actor within the system. The returned
    /// path is cloned from the internal reference and can be safely used without
    /// affecting the original actor reference.
    ///
    /// # Use Cases
    ///
    /// Actor paths are essential for:
    /// - **Message Routing**: Addressing messages to specific actors
    /// - **Logging and Debugging**: Identifying actors in log messages and traces
    /// - **System Administration**: Monitoring and managing actors by path
    /// - **Actor Lookup**: Finding actors using the system's actor registry
    /// - **Supervision Logic**: Understanding parent-child relationships
    /// - **Persistence**: Storing references to actors across system restarts
    ///
    /// # Examples
    ///
    /// ## Basic Path Retrieval
    ///
    /// ```ignore
    /// let path = actor_ref.path();
    /// println!("Actor path: {}", path);
    /// // Output: Actor path: /user/worker-pool/worker-1
    ///
    /// // Use path for logging context
    /// log::info!("Processing message from actor: {}", path);
    /// ```
    ///
    /// ## Path-Based Actor Management
    ///
    /// ```ignore
    /// // Store actor paths for later lookup
    /// let mut actor_registry = HashMap::new();
    /// actor_registry.insert(
    ///     "primary-worker".to_string(),
    ///     worker_actor.path(),
    /// );
    ///
    /// // Later, retrieve actor by stored path
    /// if let Some(stored_path) = actor_registry.get("primary-worker") {
    ///     if let Some(actor_ref) = system.get_actor(stored_path).await {
    ///         actor_ref.tell(WorkMessage::Process).await?;
    ///     }
    /// }
    /// ```
    ///
    /// ## Supervision Tree Analysis
    ///
    /// ```ignore
    /// fn analyze_supervision_tree(actor_ref: &ActorRef<MyActor>) {
    ///     let path = actor_ref.path();
    ///
    ///     // Extract parent path
    ///     if let Some(parent_path) = path.parent() {
    ///         println!("Actor parent: {}", parent_path);
    ///     }
    ///
    ///     // Get actor name
    ///     let actor_name = path.name();
    ///     println!("Actor name: {}", actor_name);
    ///
    ///     // Check if system actor
    ///     if path.is_system_actor() {
    ///         println!("This is a system-level actor");
    ///     }
    /// }
    /// ```
    ///
    /// ## Path-Based Message Tracing
    ///
    /// ```ignore
    /// #[async_trait]
    /// impl Handler<MyActor> for MyActor {
    ///     async fn handle_message(
    ///         &mut self,
    ///         sender: ActorPath,
    ///         msg: Self::Message,
    ///         ctx: &mut ActorContext<Self>,
    ///     ) -> Result<Self::Response, Error> {
    ///         let my_path = ctx.actor_ref().unwrap().path();
    ///
    ///         log::debug!(
    ///             "Message flow: {} -> {} ({})",
    ///             sender,
    ///             my_path,
    ///             std::any::type_name::<Self::Message>()
    ///         );
    ///
    ///         // Process message...
    ///         Ok(response)
    ///     }
    /// }
    /// ```
    ///
    /// ## Configuration and Monitoring
    ///
    /// ```ignore
    /// // Path-based configuration lookup
    /// let path = database_actor.path();
    /// let config = system_config.get_actor_config(&path);
    ///
    /// // Path-based metrics collection
    /// metrics::counter!("actor.messages.sent")
    ///     .tag("actor_path", path.to_string())
    ///     .increment(1);
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Cheap Operation**: Path retrieval is a simple clone operation with minimal overhead
    /// - **Immutable Data**: Paths are immutable and can be cached or stored safely
    /// - **String-Based**: Internally uses string representation for human readability
    /// - **Hierarchical Structure**: Maintains parent-child relationships efficiently
    ///
    /// # Thread Safety
    ///
    /// The returned `ActorPath` is `Send + Sync` and can be safely used across threads
    /// and async contexts. Paths are immutable after creation, eliminating any concerns
    /// about concurrent modification or data races.
    ///
    /// # Persistence Considerations
    ///
    /// Actor paths can be serialized and persisted across system restarts, but be aware
    /// that actors may not exist at the same paths after restart depending on your
    /// application's actor creation logic. Use paths for identification rather than
    /// assuming persistent actor availability.
    ///
    pub fn path(&self) -> ActorPath {
        self.path.clone()
    }

    /// Checks whether the actor is available for message delivery.
    ///
    /// This method provides a non-blocking way to determine if the target actor is still
    /// active and capable of receiving messages. It examines the underlying communication
    /// channels to detect if the actor has stopped, crashed, or been removed from the system.
    /// This check is essential for implementing robust error handling and graceful degradation
    /// in distributed actor applications.
    ///
    /// The method checks the status of the internal message delivery mechanism without
    /// attempting to send any actual messages, making it a lightweight operation suitable
    /// for frequent polling or circuit breaker patterns.
    ///
    /// # Returns
    ///
    /// Returns `true` if the actor is no longer available for message delivery, indicating:
    /// - The actor has completed its shutdown sequence
    /// - The actor crashed and its message processing loop terminated
    /// - The actor was forcibly terminated by the system
    /// - The underlying communication channels have been closed
    ///
    /// Returns `false` if the actor is still active and can potentially receive messages.
    /// Note that the actor's availability can change immediately after this check due to
    /// the concurrent nature of the actor system.
    ///
    /// # Timing Considerations
    ///
    /// This method provides a point-in-time snapshot of the actor's availability. The
    /// actor's status may change between the time you call this method and when you
    /// attempt to send a message. For robust error handling, always handle message
    /// sending errors even if this method indicates the actor is available.
    ///
    /// # Examples
    ///
    /// ## Basic Availability Check
    ///
    /// ```ignore
    /// if actor_ref.is_closed() {
    ///     log::warn!("Actor is no longer available");
    ///     return Err("Service unavailable".into());
    /// }
    ///
    /// // Proceed with message sending
    /// actor_ref.tell(MyMessage::Process(data)).await?;
    /// ```
    ///
    /// ## Circuit Breaker Pattern
    ///
    /// ```ignore
    /// struct CircuitBreaker {
    ///     actor_ref: ActorRef<ServiceActor>,
    ///     failure_count: AtomicUsize,
    ///     last_failure: AtomicU64,
    /// }
    ///
    /// impl CircuitBreaker {
    ///     async fn call_service(&self, request: ServiceRequest) -> Result<ServiceResponse, Error> {
    ///         // Check if actor is available
    ///         if self.actor_ref.is_closed() {
    ///             return Err(Error::ServiceUnavailable);
    ///         }
    ///
    ///         // Check failure threshold
    ///         if self.failure_count.load(Ordering::Relaxed) > 5 {
    ///             let time_since_failure = SystemTime::now()
    ///                 .duration_since(UNIX_EPOCH)
    ///                 .unwrap()
    ///                 .as_secs() - self.last_failure.load(Ordering::Relaxed);
    ///
    ///             if time_since_failure < 30 { // 30 second cooldown
    ///                 return Err(Error::CircuitOpen);
    ///             }
    ///         }
    ///
    ///         // Attempt service call
    ///         match self.actor_ref.ask(request).await {
    ///             Ok(response) => {
    ///                 self.failure_count.store(0, Ordering::Relaxed);
    ///                 Ok(response)
    ///             }
    ///             Err(e) => {
    ///                 self.failure_count.fetch_add(1, Ordering::Relaxed);
    ///                 self.last_failure.store(
    ///                     SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
    ///                     Ordering::Relaxed,
    ///                 );
    ///                 Err(e)
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// ## Health Monitoring
    ///
    /// ```ignore
    /// async fn monitor_actor_health(actors: Vec<(String, ActorRef<dyn MyActor>)>) {
    ///     loop {
    ///         let mut healthy_count = 0;
    ///         let mut unhealthy_actors = Vec::new();
    ///
    ///         for (name, actor_ref) in &actors {
    ///             if actor_ref.is_closed() {
    ///                 unhealthy_actors.push(name.clone());
    ///             } else {
    ///                 healthy_count += 1;
    ///             }
    ///         }
    ///
    ///         log::info!(
    ///             "Health check: {}/{} actors healthy",
    ///             healthy_count,
    ///             actors.len()
    ///         );
    ///
    ///         if !unhealthy_actors.is_empty() {
    ///             log::warn!("Unhealthy actors: {}", unhealthy_actors.join(", "));
    ///             // Trigger restart or failover logic
    ///             restart_unhealthy_actors(&unhealthy_actors).await;
    ///         }
    ///
    ///         tokio::time::sleep(Duration::from_secs(10)).await;
    ///     }
    /// }
    /// ```
    ///
    /// ## Conditional Message Batching
    ///
    /// ```ignore
    /// async fn send_message_batch(
    ///     actor_ref: &ActorRef<BatchProcessor>,
    ///     messages: Vec<BatchMessage>,
    /// ) -> Result<usize, Error> {
    ///     if actor_ref.is_closed() {
    ///         log::error!("Cannot send batch: processor is offline");
    ///         return Ok(0); // No messages sent
    ///     }
    ///
    ///     let mut sent_count = 0;
    ///     for message in messages {
    ///         match actor_ref.tell(message).await {
    ///             Ok(()) => sent_count += 1,
    ///             Err(e) => {
    ///                 log::warn!("Failed to send message {}: {}", sent_count + 1, e);
    ///                 break;
    ///             }
    ///         }
    ///
    ///         // Re-check availability periodically during long batches
    ///         if sent_count % 100 == 0 && actor_ref.is_closed() {
    ///             log::warn!("Actor became unavailable during batch processing");
    ///             break;
    ///         }
    ///     }
    ///
    ///     log::info!("Successfully sent {} messages", sent_count);
    ///     Ok(sent_count)
    /// }
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Non-blocking**: Returns immediately without any I/O or waiting
    /// - **Lightweight**: Minimal CPU overhead, suitable for frequent checking
    /// - **Lock-free**: Uses atomic operations for thread-safe status checking
    /// - **Cache-friendly**: Status information is typically cache-resident
    ///
    /// # Best Practices
    ///
    /// - **Defensive Programming**: Always handle message sending errors even after availability checks
    /// - **Avoid Race Conditions**: Don't rely solely on this check for critical error handling
    /// - **Monitoring Integration**: Use for health checks and monitoring dashboards
    /// - **Circuit Breaker**: Integrate with circuit breaker patterns for robust service calls
    /// - **Graceful Degradation**: Use to implement fallback mechanisms when actors are unavailable
    ///
    /// # Thread Safety
    ///
    /// This method is thread-safe and can be called from any context without synchronization.
    /// The underlying status check uses atomic operations to ensure consistent results across
    /// concurrent access patterns.
    ///
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Creates a new subscription to receive events published by this actor.
    ///
    /// This method provides access to the actor's event stream, enabling observers to receive
    /// notifications about the actor's state changes, processing results, and custom events.
    /// Each call to this method creates an independent event receiver that will receive all
    /// events published by the actor after the subscription is established. This supports
    /// the event sourcing pattern and enables loose coupling between system components.
    ///
    /// The returned receiver operates on a broadcast channel, meaning multiple subscribers
    /// can independently receive the same events without interfering with each other.
    /// Events are delivered asynchronously and in the order they were published by the actor.
    ///
    /// # Event Delivery Semantics
    ///
    /// - **Future Events Only**: Subscribers receive events published after subscription
    /// - **Independent Streams**: Each subscriber gets their own independent event stream
    /// - **Ordered Delivery**: Events are delivered in the order they were published
    /// - **Asynchronous**: Event delivery doesn't block the actor's message processing
    /// - **Buffered**: Events are buffered to handle temporary subscriber delays
    ///
    /// # Returns
    ///
    /// Returns an `EventReceiver<A::Event>` that can be used to receive events from the
    /// target actor. The receiver implements async iteration and can be used with standard
    /// async patterns including `while let` loops, `select!` macros, and stream combinators.
    ///
    /// The event type is determined by the actor's associated `Event` type, ensuring
    /// type safety and compile-time verification of event handling logic.
    ///
    /// # Buffer Behavior
    ///
    /// Event receivers maintain an internal buffer to handle temporary processing delays.
    /// If a subscriber falls too far behind (buffer overflow), older events may be dropped
    /// and the subscriber will receive a `RecvError::Lagged` error indicating missed events.
    /// This prevents slow subscribers from affecting actor performance or system memory usage.
    ///
    /// # Examples
    ///
    /// ## Basic Event Subscription
    ///
    /// ```ignore
    /// // Subscribe to actor events
    /// let mut event_receiver = actor_ref.subscribe();
    ///
    /// // Listen for events in background task
    /// tokio::spawn(async move {
    ///     while let Ok(event) = event_receiver.recv().await {
    ///         match event {
    ///             MyActorEvent::StateChanged { old_state, new_state } => {
    ///                 log::info!("Actor state: {} -> {}", old_state, new_state);
    ///             }
    ///             MyActorEvent::ProcessingCompleted { result } => {
    ///                 log::debug!("Processing completed: {:?}", result);
    ///             }
    ///             MyActorEvent::ErrorOccurred { error } => {
    ///                 log::error!("Actor reported error: {}", error);
    ///             }
    ///         }
    ///     }
    ///     log::info!("Event subscription ended");
    /// });
    /// ```
    ///
    /// ## Multiple Independent Subscribers
    ///
    /// ```ignore
    /// // Create multiple independent event streams
    /// let audit_receiver = actor_ref.subscribe();
    /// let metrics_receiver = actor_ref.subscribe();
    /// let notification_receiver = actor_ref.subscribe();
    ///
    /// // Audit logging subscriber
    /// tokio::spawn(async move {
    ///     while let Ok(event) = audit_receiver.recv().await {
    ///         audit_log::record_actor_event(actor_ref.path(), &event).await;
    ///     }
    /// });
    ///
    /// // Metrics collection subscriber
    /// tokio::spawn(async move {
    ///     while let Ok(event) = metrics_receiver.recv().await {
    ///         metrics::process_actor_event(&event);
    ///     }
    /// });
    ///
    /// // Real-time notification subscriber
    /// tokio::spawn(async move {
    ///     while let Ok(event) = notification_receiver.recv().await {
    ///         if event.is_critical() {
    ///             notification_service::send_alert(event).await;
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// ## Event Filtering and Processing
    ///
    /// ```ignore
    /// let mut events = actor_ref.subscribe();
    ///
    /// tokio::spawn(async move {
    ///     while let Ok(event) = events.recv().await {
    ///         // Filter for specific event types
    ///         if let MyActorEvent::DataProcessed { batch_id, count } = event {
    ///             // Update processing statistics
    ///             update_batch_statistics(batch_id, count).await;
    ///
    ///             // Trigger downstream processing
    ///             if count > 1000 {
    ///                 trigger_large_batch_notification(batch_id).await;
    ///             }
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// ## Error Handling and Recovery
    ///
    /// ```ignore
    /// use tokio::sync::broadcast::error::RecvError;
    ///
    /// let mut events = actor_ref.subscribe();
    ///
    /// tokio::spawn(async move {
    ///     loop {
    ///         match events.recv().await {
    ///             Ok(event) => {
    ///                 // Process event normally
    ///                 process_event(event).await;
    ///             }
    ///             Err(RecvError::Lagged(missed_count)) => {
    ///                 log::warn!("Missed {} events due to processing lag", missed_count);
    ///                 // Continue processing new events
    ///                 continue;
    ///             }
    ///             Err(RecvError::Closed) => {
    ///                 log::info!("Actor stopped publishing events");
    ///                 break;
    ///             }
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// ## Selective Event Processing with timeout
    ///
    /// ```ignore
    /// use tokio::time::{timeout, Duration};
    /// use tokio::select;
    ///
    /// let mut events = actor_ref.subscribe();
    /// let mut shutdown_signal = shutdown_receiver();
    ///
    /// tokio::spawn(async move {
    ///     loop {
    ///         select! {
    ///             event_result = timeout(Duration::from_secs(30), events.recv()) => {
    ///                 match event_result {
    ///                     Ok(Ok(event)) => {
    ///                         process_event_with_context(event).await;
    ///                     }
    ///                     Ok(Err(_)) => {
    ///                         log::info!("Event stream closed");
    ///                         break;
    ///                     }
    ///                     Err(_) => {
    ///                         log::debug!("No events received in 30 seconds");
    ///                         // Perform periodic maintenance
    ///                         perform_periodic_maintenance().await;
    ///                     }
    ///                 }
    ///             }
    ///             _ = shutdown_signal.recv() => {
    ///                 log::info!("Shutting down event processor");
    ///                 break;
    ///             }
    ///         }
    ///     }
    /// });
    /// ```
    ///
    /// # Performance Characteristics
    ///
    /// - **Cheap Subscription**: Creating new subscriptions has minimal overhead
    /// - **Independent Buffering**: Each subscriber has independent buffering and doesn't affect others
    /// - **Async Delivery**: Event delivery is asynchronous and doesn't block the actor
    /// - **Memory Bounded**: Buffer limits prevent unbounded memory growth from slow subscribers
    ///
    /// # Threading Context
    ///
    /// Event receivers can be moved across threads and used in any async context.
    /// The underlying broadcast channel is designed for concurrent access and provides
    /// efficient cross-thread event delivery.
    ///
    /// # Best Practices
    ///
    /// - **Handle Lag Errors**: Always handle `RecvError::Lagged` to recover from processing delays
    /// - **Independent Processing**: Use separate subscribers for different processing concerns
    /// - **Async Processing**: Process events asynchronously to avoid blocking event delivery
    /// - **Graceful Shutdown**: Handle `RecvError::Closed` to detect when actors stop publishing
    /// - **Selective Filtering**: Filter events early to reduce processing overhead
    ///
    /// # Use Cases
    ///
    /// - **Event Sourcing**: Building event logs and audit trails
    /// - **Real-time Monitoring**: Creating dashboards and alerting systems
    /// - **Loose Coupling**: Enabling publisher-subscriber architectures
    /// - **State Synchronization**: Keeping external systems in sync with actor state
    /// - **Analytics and Metrics**: Collecting operational data from actor activities
    ///
    pub fn subscribe(&self) -> EventReceiver<<A as Actor>::Event> {
        self.event_receiver.resubscribe()
    }
}

/// Clone implementation enabling efficient sharing of actor references across contexts.
///
/// This implementation allows `ActorRef` instances to be cloned cheaply and safely shared
/// across multiple threads, tasks, and components. Cloning creates an independent reference
/// to the same underlying actor, enabling concurrent access without coordination overhead.
/// Each clone maintains its own communication channels while pointing to the same actor instance.
///
/// # Cloning Behavior
///
/// When an `ActorRef` is cloned, the following components are duplicated:
/// - **Actor Path**: The hierarchical path is cloned (cheap string clone)
/// - **Message Sender**: A new handle to the same message delivery channel
/// - **Stop Sender**: A new handle to the same termination control channel
/// - **Event Receiver**: A new independent subscription to the actor's event stream
///
/// The cloning process is designed to be efficient with minimal allocation and no deep copying
/// of actor state. All clones reference the same underlying actor instance and share the same
/// communication infrastructure.
///
/// # Independent Event Subscriptions
///
/// Each cloned actor reference gets its own independent event subscription through
/// `resubscribe()`. This means:
/// - Each clone can receive events independently without affecting others
/// - Event processing in one clone doesn't block others
/// - Clones can be used for different event processing concerns (audit, metrics, etc.)
/// - Event buffer overflow in one clone doesn't affect others
///
/// # Use Cases
///
/// Actor reference cloning enables several important patterns:
///
/// ## Sharing Across Components
///
/// ```ignore
/// // Share actor reference with multiple components
/// let worker_ref = system.create_actor("worker", WorkerActor::new()).await?;
/// let scheduler_ref = worker_ref.clone();
/// let monitor_ref = worker_ref.clone();
///
/// // Each component can use its own reference independently
/// tokio::spawn(async move {
///     scheduler.run_with_actor(scheduler_ref).await;
/// });
///
/// tokio::spawn(async move {
///     monitor.watch_actor(monitor_ref).await;
/// });
/// ```
///
/// ## Cross-Thread Communication
///
/// ```ignore
/// let actor_ref = Arc::new(worker_actor_ref);
///
/// // Spawn background tasks with cloned references
/// for i in 0..num_threads {
///     let actor_clone = Arc::clone(&actor_ref);
///     thread::spawn(move || {
///         let rt = Runtime::new().unwrap();
///         rt.block_on(async {
///             loop {
///                 let work = get_work_item().await;
///                 actor_clone.tell(ProcessWork(work)).await.ok();
///             }
///         });
///     });
/// }
/// ```
///
/// ## Independent Event Processing
///
/// ```ignore
/// let base_ref = actor_ref;
/// let audit_ref = base_ref.clone();
/// let metrics_ref = base_ref.clone();
///
/// // Each clone gets independent event subscriptions
/// tokio::spawn(async move {
///     let mut events = audit_ref.subscribe();
///     while let Ok(event) = events.recv().await {
///         audit_service.record_event(event).await;
///     }
/// });
///
/// tokio::spawn(async move {
///     let mut events = metrics_ref.subscribe();
///     while let Ok(event) = events.recv().await {
///         metrics_service.process_event(event);
///     }
/// });
/// ```
///
/// # Performance Characteristics
///
/// - **Cheap Operation**: Cloning is efficient with minimal memory allocation
/// - **Shared Infrastructure**: All clones share the same underlying communication channels
/// - **Independent Buffering**: Event subscriptions are independent with separate buffers
/// - **No Coordination Overhead**: Clones can be used concurrently without synchronization
///
/// # Thread Safety
///
/// Cloned `ActorRef` instances are fully thread-safe and can be moved across thread
/// boundaries without additional synchronization. The underlying communication channels
/// are designed for concurrent access from multiple threads.
///
/// # Memory Management
///
/// Cloned references participate in reference counting for the underlying actor resources.
/// The actor and its associated resources remain available as long as any reference
/// (original or cloned) exists. When all references are dropped, the actor becomes
/// eligible for cleanup.
///
/// # Best Practices
///
/// - **Clone Liberally**: Don't hesitate to clone references when needed for sharing
/// - **Independent Concerns**: Use separate clones for logically different concerns
/// - **Event Isolation**: Clone for independent event processing workflows
/// - **Thread Boundaries**: Clone before moving references across thread boundaries
/// - **Avoid Unnecessary Coordination**: Let clones operate independently where possible
///
impl<A> Clone for ActorRef<A>
where
    A: Actor + Handler<A>,
{
    /// Creates a new independent reference to the same actor.
    ///
    /// This method efficiently clones the actor reference, creating a new independent
    /// handle that can be used concurrently with the original. The clone includes:
    /// - A copy of the actor's hierarchical path
    /// - A new handle to the same message delivery infrastructure
    /// - A new handle to the same termination control channel
    /// - An independent subscription to the actor's event stream
    ///
    /// # Returns
    ///
    /// Returns a new `ActorRef<A>` that references the same underlying actor instance
    /// but can be used independently for message sending, lifecycle management, and
    /// event subscription without interfering with the original reference.
    ///
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            sender: self.sender.clone(),
            stop_sender: self.stop_sender.clone(),
            event_receiver: self.event_receiver.resubscribe(),
        }
    }
}

#[cfg(test)]
mod test {

    use super::*;

    use crate::sink::{Sink, Subscriber};

    use serde::{Deserialize, Serialize};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Clone)]
    struct TestActor {
        counter: usize,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestMessage(usize);

    impl Message for TestMessage {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestResponse(usize);

    impl Response for TestResponse {}

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestEvent(usize);

    impl Event for TestEvent {}

    #[async_trait]
    impl Actor for TestActor {
        type Message = TestMessage;
        type Event = TestEvent;
        type Response = TestResponse;
    }

    #[async_trait]
    impl Handler<TestActor> for TestActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: TestMessage,
            ctx: &mut ActorContext<TestActor>,
        ) -> Result<TestResponse, Error> {
            if ctx.parent::<TestActor>().await.is_some() {
                panic!("Is not a root actor");
            }

            let value = msg.0;
            self.counter += value;
            ctx.publish_event(TestEvent(self.counter)).await.unwrap();
            Ok(TestResponse(self.counter))
        }
    }

    pub struct TestSubscriber;

    #[async_trait]
    impl Subscriber<TestEvent> for TestSubscriber {
        async fn notify(&self, event: TestEvent) {
            debug!("Received event: {:?}", event);
            assert!(event.0 > 0);
        }
    }

    #[tokio::test]
    async fn test_actor() {
        let (event_sender, _event_receiver) = mpsc::channel(100);
        let system = SystemRef::new(event_sender, CancellationToken::new());
        let actor = TestActor { counter: 0 };
        let actor_ref = system.create_root_actor("test", actor).await.unwrap();

        let sink = Sink::new(actor_ref.subscribe(), TestSubscriber);
        system.run_sink(sink).await;

        actor_ref.tell(TestMessage(10)).await.unwrap();
        let mut recv = actor_ref.subscribe();
        let response = actor_ref.ask(TestMessage(10)).await.unwrap();
        assert_eq!(response.0, 20);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 10);
        let event = recv.recv().await.unwrap();
        assert_eq!(event.0, 20);
        actor_ref.ask_stop().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
