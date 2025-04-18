// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

// Integrations tests for the actor module

use actor::{
    Actor, ActorContext, ActorPath, ActorRef, ActorSystem, ChildAction, Error,
    Event, Handler, Message, Response,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

// Defines parent actor
#[derive(Debug, Clone)]
pub struct TestActor {
    pub state: usize,
}

// Defines parent command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestCommand {
    Increment(usize),
    Decrement(usize),
    GetState,
}

// Implements message for parent command.
impl Message for TestCommand {}

// Defines parent response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TestResponse {
    State(usize),
    None,
}

// Implements response for parent response.
impl Response for TestResponse {}

// Defines parent event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvent(usize);

// Implements event for parent event.
impl Event for TestEvent {}

// Implements actor for parent actor.
#[async_trait]
impl Actor for TestActor {
    type Message = TestCommand;
    type Response = TestResponse;
    type Event = TestEvent;

    async fn pre_start(
        &mut self,
        ctx: &mut ActorContext<Self>,
    ) -> Result<(), Error> {
        let child = ChildActor { state: 0 };
        ctx.create_child("child", child).await?;
        Ok(())
    }
}

// Implements handler for parent actor.
#[async_trait]
impl Handler<TestActor> for TestActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        message: TestCommand,
        ctx: &mut ActorContext<TestActor>,
    ) -> Result<TestResponse, Error> {
        match message {
            TestCommand::Increment(value) => {
                self.state += value;
                //ctx.emit_event(TestEvent(self.state)).await.unwrap();
                let child: ActorRef<ChildActor> =
                    ctx.get_child("child").await.unwrap();
                child
                    .tell(ChildCommand::SetState(self.state))
                    .await
                    .unwrap();
                Ok(TestResponse::None)
            }
            TestCommand::Decrement(value) => {
                self.state -= value;
                ctx.publish_event(TestEvent(self.state)).await.unwrap();

                let child: ActorRef<ChildActor> =
                    ctx.get_child("child").await.unwrap();
                child
                    .tell(ChildCommand::SetState(self.state))
                    .await
                    .unwrap();
                Ok(TestResponse::None)
            }
            TestCommand::GetState => Ok(TestResponse::State(self.state)),
        }
    }

    // Handles child error.
    async fn on_child_error(
        &mut self,
        error: Error,
        ctx: &mut ActorContext<TestActor>,
    ) {
        assert_eq!(error, Error::Functional("Value is too high".to_owned()));
        ctx.publish_event(TestEvent(0)).await.unwrap();
    }

    // Handles child fault.
    async fn on_child_fault(
        &mut self,
        error: Error,
        ctx: &mut ActorContext<TestActor>,
    ) -> ChildAction {
        assert_eq!(
            error,
            Error::Functional("Value produces a fault".to_owned())
        );
        ctx.publish_event(TestEvent(100)).await.unwrap();
        ChildAction::Stop
    }
}

// Defines child actor.
#[derive(Debug, Clone)]
pub struct ChildActor {
    pub state: usize,
}

// Defines child command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChildCommand {
    SetState(usize),
    GetState,
}

// Implements message for child command.
impl Message for ChildCommand {}

// Defines child response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChildResponse {
    State(usize),
    None,
}

// Implements response for child response.
impl Response for ChildResponse {}

// Defines child event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildEvent(usize);

// Implements event for child event.
impl Event for ChildEvent {}

// Implements actor for child actor.
#[async_trait]
impl Actor for ChildActor {
    type Message = ChildCommand;
    type Response = ChildResponse;
    type Event = ChildEvent;
}

// Implements handler for child actor.
#[async_trait]
impl Handler<ChildActor> for ChildActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        message: ChildCommand,
        ctx: &mut ActorContext<ChildActor>,
    ) -> Result<ChildResponse, Error> {
        match message {
            ChildCommand::SetState(value) => {
                if value <= 10 {
                    self.state = value;
                    ctx.publish_event(ChildEvent(self.state)).await.unwrap();
                    Ok(ChildResponse::None)
                } else if value > 10 && value < 100 {
                    ctx.emit_error(Error::Functional(
                        "Value is too high".to_owned(),
                    ))
                    .await
                    .unwrap();
                    Ok(ChildResponse::State(100))
                } else {
                    ctx.emit_fail(Error::Functional(
                        "Value produces a fault".to_owned(),
                    ))
                    .await
                    .unwrap();
                    Ok(ChildResponse::None)
                }
            }
            ChildCommand::GetState => Ok(ChildResponse::State(self.state)),
        }
    }
}

#[tokio::test]
async fn test_actor() {
    let (system, mut runner) = ActorSystem::create(None);
    // Init runner.
    tokio::spawn(async move {
        runner.run().await;
    });

    let parent = TestActor { state: 0 };
    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let child_actor = system
        .get_actor::<ChildActor>(&ActorPath::from("/user/parent/child"))
        .await
        .unwrap();
    let mut recv = child_actor.subscribe();

    parent_ref.tell(TestCommand::Increment(10)).await.unwrap();
    let response = parent_ref.ask(TestCommand::GetState).await.unwrap();
    assert_eq!(response, TestResponse::State(10));

    let event = recv.recv().await.unwrap();
    assert_eq!(event.0, 10);
    let response = child_actor.ask(ChildCommand::GetState).await.unwrap();
    assert_eq!(response, ChildResponse::State(10));

    parent_ref.tell(TestCommand::Decrement(2)).await.unwrap();
    let response = parent_ref.ask(TestCommand::GetState).await.unwrap();
    assert_eq!(response, TestResponse::State(8));

    let event = recv.recv().await.unwrap();
    assert_eq!(event.0, 8);
    let response = child_actor.ask(ChildCommand::GetState).await.unwrap();
    assert_eq!(response, ChildResponse::State(8));
}

#[tokio::test]
async fn test_actor_error() {
    let (system, mut runner) = ActorSystem::create(None);
    // Init runner.
    tokio::spawn(async move {
        runner.run().await;
    });

    let parent = TestActor { state: 0 };
    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();
    let mut receiver = parent_ref.subscribe();
    parent_ref.tell(TestCommand::Increment(50)).await.unwrap();
    let response = parent_ref.ask(TestCommand::GetState).await.unwrap();
    assert_eq!(response, TestResponse::State(50));

    while let Some(event) = receiver.recv().await.ok() {
        assert_eq!(event.0, 0);
        break;
    }
    //system.stop_actor(&parent_ref.path()).await;
}

#[tokio::test]
async fn test_actor_fault() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let (system, mut runner) = ActorSystem::create(None);
    // Init runner.
    tokio::spawn(async move {
        runner.run().await;
    });
    let parent = TestActor { state: 0 };
    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let child_ref = system
        .get_actor::<ChildActor>(&ActorPath::from("/user/parent/child"))
        .await;
    assert!(child_ref.is_some());

    let mut receiver = parent_ref.subscribe();
    parent_ref.tell(TestCommand::Increment(110)).await.unwrap();
    let response = parent_ref.ask(TestCommand::GetState).await.unwrap();
    assert_eq!(response, TestResponse::State(110));

    while let Some(event) = receiver.recv().await.ok() {
        assert_eq!(event.0, 100);
        break;
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let child_ref = system
        .get_actor::<ChildActor>(&ActorPath::from("/user/parent/child"))
        .await;
    assert!(child_ref.is_none());
}
