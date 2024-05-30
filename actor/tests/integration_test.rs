// Integrations tests for the actor module

use actor::{
    Actor, ActorContext, ActorPath, ActorRef, ActorSystem, Error, Event,
    Handler, Message, Response,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use tracing_test::traced_test;

// Defines parent actor
#[derive(Debug, Clone)]
pub struct TestActor {
    pub state: usize,
}

// Defines parent command
#[derive(Debug, Clone)]
pub enum TestCommand {
    Increment(usize),
    Decrement(usize),
    GetState,
}

// Implements message for parent command.
impl Message for TestCommand {}

// Defines parent response.
#[derive(Debug, Clone, PartialEq)]
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
        ctx: &ActorContext<Self>,
    ) -> Result<(), Error> {
        let child = ChildActor { state: 0 };
        ctx.create_child("child", child).await?;
        Ok(())
    }
}

// Implements handler for parent actor.
#[async_trait]
impl Handler<TestActor> for TestActor {
    async fn handle(
        &mut self,
        message: TestCommand,
        ctx: &mut ActorContext<TestActor>,
    ) -> TestResponse {
        match message {
            TestCommand::Increment(value) => {
                self.state += value;
                ctx.emit_event(TestEvent(self.state)).await.unwrap();
                let child: ActorRef<ChildActor> =
                    ctx.get_child("child").await.unwrap();
                child
                    .tell(ChildCommand::SetState(self.state))
                    .await
                    .unwrap();
                TestResponse::None
            }
            TestCommand::Decrement(value) => {
                self.state -= value;
                ctx.emit_event(TestEvent(self.state)).await.unwrap();
                TestResponse::None
            }
            TestCommand::GetState => TestResponse::State(self.state),
        }
    }
}

// Defines child actor.
#[derive(Debug, Clone)]
pub struct ChildActor {
    pub state: usize,
}

// Defines child command.
#[derive(Debug, Clone)]
pub enum ChildCommand {
    SetState(usize),
    GetState,
}

// Implements message for child command.
impl Message for ChildCommand {}

// Defines child response.
#[derive(Debug, Clone, PartialEq)]
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
    async fn handle(
        &mut self,
        message: ChildCommand,
        ctx: &mut ActorContext<ChildActor>,
    ) -> ChildResponse {
        match message {
            ChildCommand::SetState(value) => {
                self.state = value;
                ctx.emit_event(ChildEvent(self.state)).await.unwrap();
                ChildResponse::None
            }
            ChildCommand::GetState => ChildResponse::State(self.state),
        }
    }
}

#[tokio::test]
#[traced_test]
async fn test_actor() {
    let system = ActorSystem::default();
    let parent = TestActor { state: 0 };
    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();
    let mut receiver = parent_ref.subscribe();
    parent_ref.tell(TestCommand::Increment(10)).await.unwrap();
    let response = parent_ref.ask(TestCommand::GetState).await.unwrap();
    assert_eq!(response, TestResponse::State(10));
    let event = receiver.recv().await.unwrap();
    assert_eq!(event.0, 10);

    let child: ActorRef<ChildActor> = system
        .get_actor(&ActorPath::from("/user/parent/child"))
        .await
        .unwrap();
    let response = child.ask(ChildCommand::GetState).await.unwrap();
    assert_eq!(response, ChildResponse::State(10));

    system.stop_actor(&parent_ref.path()).await;
}
