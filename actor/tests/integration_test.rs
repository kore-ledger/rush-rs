// Integrations tests for the actor module

use actor::{
    ActorSystem, Actor, ActorContext, ActorPath, ActorRef, ChildAction, Error, Event, 
    Handler, Message, Response,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use tracing::error;
use tracing_subscriber::EnvFilter;
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
        message: TestCommand,
        ctx: &mut ActorContext<TestActor>,
    ) -> TestResponse {
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
                TestResponse::None
            }
            TestCommand::Decrement(value) => {
                self.state -= value;
                ctx.publish_event(TestEvent(self.state)).await.unwrap();
                TestResponse::None
            }
            TestCommand::GetState => TestResponse::State(self.state),
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
    async fn handle_message(
        &mut self,
        message: ChildCommand,
        ctx: &mut ActorContext<ChildActor>,
    ) -> ChildResponse {
        match message {
            ChildCommand::SetState(value) => {
                if value <= 10 {
                    self.state = value;
                    ctx.publish_event(ChildEvent(self.state)).await.unwrap();
                    ChildResponse::None
                } else if value > 10 && value < 100 {
                    if ctx
                        .emit_error(Error::Functional(
                            "Value is too high".to_owned(),
                        ))
                        .await
                        .is_err()
                    {
                        error!("Error emitting error");
                    }
                    ChildResponse::State(100)
                } else {
                    if ctx
                        .emit_fail(Error::Functional(
                            "Value produces a fault".to_owned(),
                        ))
                        .await
                        .is_err()
                    {
                        error!("Error emitting fault");
                    }
                    ChildResponse::None
                }
            }
            ChildCommand::GetState => ChildResponse::State(self.state),
        }
    }
}


#[tokio::test]
async fn test_actor() {
    let (system, mut runner) = ActorSystem::create();
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

    
}

#[tokio::test]
async fn test_actor_error() {
    let (system, mut runner) = ActorSystem::create();
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
#[traced_test]
async fn test_actor_fault() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let (system, mut runner) = ActorSystem::create();
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

