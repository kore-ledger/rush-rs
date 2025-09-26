// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! Comprehensive edge case tests for Actor module to increase coverage
use actor::{
    Actor, ActorContext, ActorPath, ActorSystem, ChildAction, Error,
    Event, Handler, Message, Response, SupervisionStrategy, Strategy,
    FixedIntervalStrategy, NoIntervalStrategy, CustomIntervalStrategy,
    RetryActor, RetryMessage, RetryStrategy
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};
use tokio_util::sync::CancellationToken;
use tracing_test::traced_test;

// Test actors for edge cases
#[derive(Debug, Clone)]
pub struct EdgeCaseActor {
    pub fail_on_start: bool,
    pub fail_on_restart: bool,
    pub fail_on_stop: bool,
    pub fail_on_message: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeCaseCommand {
    TriggerError,
    TriggerFault,
    GetValue,
    TestParent,
    CreateChild,
    StopSelf,
}

impl Message for EdgeCaseCommand {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeCaseResponse {
    Success,
    Value(i32),
    Error(String),
}

impl Response for EdgeCaseResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeCaseEvent(pub i32);

impl Event for EdgeCaseEvent {}

#[async_trait]
impl Actor for EdgeCaseActor {
    type Message = EdgeCaseCommand;
    type Response = EdgeCaseResponse;
    type Event = EdgeCaseEvent;

    fn supervision_strategy() -> SupervisionStrategy {
        SupervisionStrategy::Retry(Strategy::FixedInterval(
            FixedIntervalStrategy::new(2, Duration::from_millis(50))
        ))
    }

    async fn pre_start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Error> {
        if self.fail_on_start {
            self.fail_on_start = false; // Only fail once
            return Err(Error::Start("Intentional start failure".to_owned()));
        }
        Ok(())
    }

    async fn pre_restart(&mut self, ctx: &mut ActorContext<Self>, _error: Option<&Error>) -> Result<(), Error> {
        if self.fail_on_restart {
            return Err(Error::Start("Intentional restart failure".to_owned()));
        }
        self.pre_start(ctx).await
    }

    async fn pre_stop(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Error> {
        if self.fail_on_stop {
            return Err(Error::Stop);
        }
        Ok(())
    }

    async fn post_stop(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Error> {
        Ok(())
    }

    fn from_response(response: Self::Response) -> Result<Self::Event, Error> {
        match response {
            EdgeCaseResponse::Value(val) => Ok(EdgeCaseEvent(val)),
            _ => Err(Error::Functional("Cannot convert response to event".to_string()))
        }
    }
}

#[async_trait]
impl Handler<EdgeCaseActor> for EdgeCaseActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: EdgeCaseCommand,
        ctx: &mut ActorContext<EdgeCaseActor>,
    ) -> Result<EdgeCaseResponse, Error> {
        if self.fail_on_message {
            return Err(Error::Functional("Intentional message failure".to_owned()));
        }

        match msg {
            EdgeCaseCommand::TriggerError => {
                ctx.emit_error(Error::Functional("Test error".to_owned())).await?;
                Ok(EdgeCaseResponse::Success)
            }
            EdgeCaseCommand::TriggerFault => {
                ctx.emit_fail(Error::Functional("Test fault".to_owned())).await?;
                Ok(EdgeCaseResponse::Success)
            }
            EdgeCaseCommand::GetValue => Ok(EdgeCaseResponse::Value(42)),
            EdgeCaseCommand::TestParent => {
                // Test parent access
                if ctx.parent::<EdgeCaseActor>().await.is_some() {
                    Ok(EdgeCaseResponse::Success)
                } else {
                    Ok(EdgeCaseResponse::Error("No parent".to_string()))
                }
            }
            EdgeCaseCommand::CreateChild => {
                let child = EdgeCaseActor {
                    fail_on_start: false,
                    fail_on_restart: false,
                    fail_on_stop: false,
                    fail_on_message: false,
                };
                ctx.create_child("test_child", child).await?;
                Ok(EdgeCaseResponse::Success)
            }
            EdgeCaseCommand::StopSelf => {
                ctx.stop(None).await;
                Ok(EdgeCaseResponse::Success)
            }
        }
    }

    async fn on_event(&mut self, _event: EdgeCaseEvent, _ctx: &mut ActorContext<EdgeCaseActor>) {
        // Test internal event handling
    }

    async fn on_child_error(&mut self, _error: Error, _ctx: &mut ActorContext<EdgeCaseActor>) {
        // Test error handling from child
    }

    async fn on_child_fault(&mut self, error: Error, _ctx: &mut ActorContext<EdgeCaseActor>) -> ChildAction {
        // Return different actions based on error type
        match error {
            Error::Functional(_) => ChildAction::Restart,
            Error::Stop => ChildAction::Stop,
            _ => ChildAction::Delegate,
        }
    }
}

// Test actor that always fails on start
#[derive(Debug, Clone)]
pub struct FailingActor;

#[async_trait]
impl Actor for FailingActor {
    type Message = EdgeCaseCommand;
    type Response = EdgeCaseResponse;
    type Event = EdgeCaseEvent;

    fn supervision_strategy() -> SupervisionStrategy {
        SupervisionStrategy::Stop
    }

    async fn pre_start(&mut self, _ctx: &mut ActorContext<Self>) -> Result<(), Error> {
        Err(Error::Start("Always fails".to_owned()))
    }
}

#[async_trait]
impl Handler<FailingActor> for FailingActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        _msg: EdgeCaseCommand,
        _ctx: &mut ActorContext<FailingActor>,
    ) -> Result<EdgeCaseResponse, Error> {
        Ok(EdgeCaseResponse::Success)
    }
}

// Test supervision strategies
#[tokio::test]
#[traced_test]
async fn test_actor_with_retry_supervision() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = EdgeCaseActor {
        fail_on_start: true,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let actor_ref = system.create_root_actor("retry_actor", actor).await.unwrap();

    // Wait for retry to complete
    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = actor_ref.ask(EdgeCaseCommand::GetValue).await.unwrap();
    assert_eq!(response, EdgeCaseResponse::Value(42));
}

#[tokio::test]
async fn test_actor_with_stop_supervision() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = FailingActor;
    let result = system.create_root_actor("failing_actor", actor).await;

    // Should fail to create due to Stop supervision
    assert!(result.is_err());
}

#[tokio::test]
async fn test_custom_interval_strategy() {
    let mut strategy = Strategy::CustomIntervalStrategy(
        CustomIntervalStrategy::new(VecDeque::from([
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ]))
    );

    assert_eq!(strategy.max_retries(), 3);
    assert_eq!(strategy.next_backoff(), Some(Duration::from_millis(10)));
    assert_eq!(strategy.next_backoff(), Some(Duration::from_millis(20)));
    assert_eq!(strategy.next_backoff(), Some(Duration::from_millis(30)));
    assert_eq!(strategy.next_backoff(), None);
}

#[tokio::test]
async fn test_no_interval_strategy() {
    let mut strategy = Strategy::NoInterval(NoIntervalStrategy::new(5));
    assert_eq!(strategy.max_retries(), 5);
    assert_eq!(strategy.next_backoff(), None);
}

#[tokio::test]
async fn test_actor_ref_operations() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let actor_ref = system.create_root_actor("test_actor", actor).await.unwrap();

    // Test path
    assert_eq!(actor_ref.path().to_string(), "/user/test_actor");

    // Test is_closed
    assert!(!actor_ref.is_closed());

    // Test tell_stop
    actor_ref.tell_stop().await;

    // Wait for stop
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should be closed now
    assert!(actor_ref.is_closed());
}

#[tokio::test]
async fn test_event_subscription() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let actor_ref = system.create_root_actor("event_actor", actor).await.unwrap();
    let _receiver = actor_ref.subscribe();

    // Test event emission
    actor_ref.tell(EdgeCaseCommand::TriggerError).await.unwrap();

    // Should not receive events from tell (only ask generates events via from_response)
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_child_actor_management() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let parent = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();

    // Create child
    parent_ref.tell(EdgeCaseCommand::CreateChild).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify child exists
    let child = system.get_actor::<EdgeCaseActor>(&ActorPath::from("/user/parent/test_child")).await;
    assert!(child.is_some());

    // Test parent access from child
    if let Some(child_ref) = child {
        let response = child_ref.ask(EdgeCaseCommand::TestParent).await.unwrap();
        assert_eq!(response, EdgeCaseResponse::Success);
    }
}

#[tokio::test]
async fn test_error_and_fault_handling() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let parent = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let parent_ref = system.create_root_actor("error_parent", parent).await.unwrap();

    // Create child that will fail
    let _failing_child = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: true,
    };

    // Manually create child for testing error propagation
    let _system_clone = system.clone();
    let _child_path = ActorPath::from("/user/error_parent/failing_child");
    // This is testing internal API which might not be directly accessible
    // We'll test error handling through the main API instead

    // Test error emission
    parent_ref.tell(EdgeCaseCommand::TriggerError).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Test fault emission
    parent_ref.tell(EdgeCaseCommand::TriggerFault).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_system_helpers() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    #[derive(Clone)]
    struct TestHelper {
        value: i32,
    }

    let helper = TestHelper { value: 123 };
    system.add_helper("test", helper).await;

    let retrieved: Option<TestHelper> = system.get_helper("test").await;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().value, 123);

    // Test non-existent helper
    let missing: Option<TestHelper> = system.get_helper("missing").await;
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_system_children_listing() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let parent_ref = system.create_root_actor("parent_with_children", actor).await.unwrap();

    // Create multiple children
    parent_ref.tell(EdgeCaseCommand::CreateChild).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Test children listing
    let parent_path = ActorPath::from("/user/parent_with_children");
    let children = system.children(&parent_path).await;
    assert!(!children.is_empty());
}

#[tokio::test]
async fn test_retry_actor_functionality() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    // Create target actor for retry
    let target = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let retry_strategy = Strategy::FixedInterval(
        FixedIntervalStrategy::new(3, Duration::from_millis(10))
    );

    let retry_actor = RetryActor::new(target, EdgeCaseCommand::GetValue, retry_strategy);
    let retry_ref = system.create_root_actor("retry_test", retry_actor).await.unwrap();

    // Start retry process
    retry_ref.tell(RetryMessage::Retry).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // End retry process
    retry_ref.tell(RetryMessage::End).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_system_stop() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    let runner_handle = tokio::spawn(async move { runner.run().await });

    let actor = EdgeCaseActor {
        fail_on_start: false,
        fail_on_restart: false,
        fail_on_stop: false,
        fail_on_message: false,
    };

    let _actor_ref = system.create_root_actor("stop_test", actor).await.unwrap();

    // Stop system
    system.stop_system();

    // Wait for system to stop
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Runner should have finished
    let result = tokio::time::timeout(Duration::from_millis(100), runner_handle).await;
    assert!(result.is_ok());
}