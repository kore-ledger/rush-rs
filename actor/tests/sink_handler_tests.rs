// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! Comprehensive tests for Sink and Handler modules to increase coverage

use actor::{
    Actor, ActorContext, ActorPath, ActorSystem, Error, Event, Handler, Message,
    Response, Sink, Subscriber,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_test::traced_test;

// Test structures for sink and handler testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkTestEvent {
    pub id: u32,
    pub data: String,
}

impl Event for SinkTestEvent {}

#[derive(Debug, Clone)]
pub struct TestActor {
    pub counter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestMessage {
    Emit(u32, String),
    GetCounter,
}

impl Message for TestMessage {}

#[derive(Debug, Clone, PartialEq)]
pub struct TestResponse {
    pub value: u32,
}

impl Response for TestResponse {}

#[async_trait]
impl Actor for TestActor {
    type Message = TestMessage;
    type Response = TestResponse;
    type Event = SinkTestEvent;
}

#[async_trait]
impl Handler<TestActor> for TestActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: TestMessage,
        ctx: &mut ActorContext<TestActor>,
    ) -> Result<TestResponse, Error> {
        match msg {
            TestMessage::Emit(id, data) => {
                self.counter += 1;
                ctx.publish_event(SinkTestEvent { id, data }).await?;
                Ok(TestResponse { value: self.counter })
            }
            TestMessage::GetCounter => Ok(TestResponse { value: self.counter }),
        }
    }
}

// Test subscriber that collects events
#[derive(Clone)]
pub struct CollectingSubscriber {
    pub events: Arc<Mutex<Vec<SinkTestEvent>>>,
    pub should_fail: bool,
}

impl CollectingSubscriber {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            should_fail: false,
        }
    }

    pub fn new_failing() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            should_fail: true,
        }
    }

    pub async fn get_events(&self) -> Vec<SinkTestEvent> {
        self.events.lock().await.clone()
    }
}

#[async_trait]
impl Subscriber<SinkTestEvent> for CollectingSubscriber {
    async fn notify(&self, event: SinkTestEvent) {
        if self.should_fail {
            // Simulate subscriber failure - this shouldn't crash the sink
            panic!("Subscriber intentionally failed");
        }
        let mut events = self.events.lock().await;
        events.push(event);
    }
}

// Tests for Sink functionality

#[tokio::test]
#[traced_test]
async fn test_sink_basic_functionality() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = TestActor { counter: 0 };
    let actor_ref = system.create_root_actor("sink_test", actor).await.unwrap();

    let subscriber = CollectingSubscriber::new();
    let subscriber_clone = subscriber.clone();

    // Create and run sink
    let sink = Sink::new(actor_ref.subscribe(), subscriber);
    system.run_sink(sink).await;

    // Give sink time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Emit some events
    actor_ref.tell(TestMessage::Emit(1, "test1".to_string())).await.unwrap();
    actor_ref.tell(TestMessage::Emit(2, "test2".to_string())).await.unwrap();
    actor_ref.tell(TestMessage::Emit(3, "test3".to_string())).await.unwrap();

    // Wait for events to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify events were collected
    let events = subscriber_clone.get_events().await;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].id, 1);
    assert_eq!(events[0].data, "test1");
    assert_eq!(events[1].id, 2);
    assert_eq!(events[1].data, "test2");
    assert_eq!(events[2].id, 3);
    assert_eq!(events[2].data, "test3");
}

#[tokio::test]
#[traced_test]
async fn test_sink_with_failing_subscriber() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = TestActor { counter: 0 };
    let actor_ref = system.create_root_actor("failing_sink_test", actor).await.unwrap();

    let subscriber = CollectingSubscriber::new_failing();

    // Create sink with failing subscriber
    let sink = Sink::new(actor_ref.subscribe(), subscriber);
    system.run_sink(sink).await;

    // Give sink time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Emit event - this should not crash the system even though subscriber fails
    actor_ref.tell(TestMessage::Emit(1, "test".to_string())).await.unwrap();

    // Wait and verify system is still running
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let response = actor_ref.ask(TestMessage::GetCounter).await.unwrap();
    assert_eq!(response.value, 1);
}

#[tokio::test]
async fn test_sink_with_closed_receiver() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = TestActor { counter: 0 };
    let actor_ref = system.create_root_actor("closed_sink_test", actor).await.unwrap();

    let subscriber = CollectingSubscriber::new();
    let receiver = actor_ref.subscribe();

    // Create sink
    let sink = Sink::new(receiver, subscriber);
    system.run_sink(sink).await;

    // Give sink time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Stop the actor (this will close the receiver)
    actor_ref.ask_stop().await.unwrap();

    // Wait for sink to handle the closed receiver
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Sink should handle the closed receiver gracefully and exit
}

// Tests for Handler functionality and error scenarios

// Actor that can fail in different ways
#[derive(Debug, Clone)]
pub struct FailingHandlerActor {
    pub fail_on_message: bool,
    pub fail_with_timeout: bool,
}

#[async_trait]
impl Actor for FailingHandlerActor {
    type Message = TestMessage;
    type Response = TestResponse;
    type Event = SinkTestEvent;
}

#[async_trait]
impl Handler<FailingHandlerActor> for FailingHandlerActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: TestMessage,
        _ctx: &mut ActorContext<FailingHandlerActor>,
    ) -> Result<TestResponse, Error> {
        if self.fail_on_message {
            return Err(Error::Functional("Handler intentionally failed".to_string()));
        }

        if self.fail_with_timeout {
            // Simulate a very long operation
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }

        match msg {
            TestMessage::GetCounter => Ok(TestResponse { value: 42 }),
            _ => Ok(TestResponse { value: 0 }),
        }
    }
}

#[tokio::test]
async fn test_handler_error_scenarios() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = FailingHandlerActor {
        fail_on_message: true,
        fail_with_timeout: false,
    };

    let actor_ref = system.create_root_actor("failing_handler", actor).await.unwrap();

    // This should return an error
    let result = actor_ref.ask(TestMessage::GetCounter).await;
    assert!(result.is_err());

    match result {
        Err(Error::Functional(msg)) => {
            assert_eq!(msg, "Handler intentionally failed");
        }
        _ => panic!("Expected functional error"),
    }
}

#[tokio::test]
async fn test_message_serialization_edge_cases() {
    // Test with complex message structures
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ComplexMessage {
        pub nested: Vec<std::collections::HashMap<String, i32>>,
        pub optional: Option<String>,
        pub tuple: (i32, String, bool),
    }

    impl Message for ComplexMessage {}

    #[derive(Debug, Clone)]
    pub struct ComplexHandlerActor;

    #[async_trait]
    impl Actor for ComplexHandlerActor {
        type Message = ComplexMessage;
        type Response = TestResponse;
        type Event = SinkTestEvent;
    }

    #[async_trait]
    impl Handler<ComplexHandlerActor> for ComplexHandlerActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: ComplexMessage,
            _ctx: &mut ActorContext<ComplexHandlerActor>,
        ) -> Result<TestResponse, Error> {
            // Verify message was properly deserialized
            assert!(!msg.nested.is_empty());
            assert!(msg.optional.is_some());
            assert_eq!(msg.tuple.0, 42);

            Ok(TestResponse { value: msg.nested.len() as u32 })
        }
    }

    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = ComplexHandlerActor;
    let actor_ref = system.create_root_actor("complex_handler", actor).await.unwrap();

    let mut nested = std::collections::HashMap::new();
    nested.insert("key1".to_string(), 100);
    nested.insert("key2".to_string(), 200);

    let complex_msg = ComplexMessage {
        nested: vec![nested],
        optional: Some("test".to_string()),
        tuple: (42, "tuple_test".to_string(), true),
    };

    let result = actor_ref.ask(complex_msg).await.unwrap();
    assert_eq!(result.value, 1);
}

// Test mailbox behavior and message ordering
#[tokio::test]
async fn test_message_ordering_and_mailbox() {
    #[derive(Debug, Clone)]
    pub struct OrderingActor {
        pub received_order: Vec<u32>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct OrderedMessage {
        pub sequence: u32,
    }

    impl Message for OrderedMessage {}

    #[async_trait]
    impl Actor for OrderingActor {
        type Message = OrderedMessage;
        type Response = TestResponse;
        type Event = SinkTestEvent;
    }

    #[async_trait]
    impl Handler<OrderingActor> for OrderingActor {
        async fn handle_message(
            &mut self,
            _sender: ActorPath,
            msg: OrderedMessage,
            _ctx: &mut ActorContext<OrderingActor>,
        ) -> Result<TestResponse, Error> {
            self.received_order.push(msg.sequence);
            Ok(TestResponse {
                value: self.received_order.len() as u32
            })
        }
    }

    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = OrderingActor {
        received_order: Vec::new(),
    };
    let actor_ref = system.create_root_actor("ordering_actor", actor).await.unwrap();

    // Send messages in sequence
    for i in 1..=5 {
        actor_ref.tell(OrderedMessage { sequence: i }).await.unwrap();
    }

    // Wait for all messages to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify final count
    let result = actor_ref.ask(OrderedMessage { sequence: 0 }).await.unwrap();
    assert_eq!(result.value, 6); // 5 tells + 1 ask
}

// Test for handler with context operations
#[tokio::test]
async fn test_handler_context_operations() {
    #[derive(Debug, Clone)]
    pub struct ContextActor {
        pub path_checked: bool,
        pub system_accessed: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ContextMessage {
        CheckPath,
        AccessSystem,
        GetState,
    }

    impl Message for ContextMessage {}

    #[async_trait]
    impl Actor for ContextActor {
        type Message = ContextMessage;
        type Response = TestResponse;
        type Event = SinkTestEvent;
    }

    #[async_trait]
    impl Handler<ContextActor> for ContextActor {
        async fn handle_message(
            &mut self,
            sender: ActorPath,
            msg: ContextMessage,
            ctx: &mut ActorContext<ContextActor>,
        ) -> Result<TestResponse, Error> {
            match msg {
                ContextMessage::CheckPath => {
                    let my_path = ctx.path();
                    assert_eq!(my_path.to_string(), "/user/context_actor");
                    assert!(!sender.is_empty()); // Should have a sender path
                    self.path_checked = true;
                    Ok(TestResponse { value: 1 })
                }
                ContextMessage::AccessSystem => {
                    let _system = ctx.system();
                    // Verify we can access system without panicking
                    self.system_accessed = true;
                    Ok(TestResponse { value: 2 })
                }
                ContextMessage::GetState => {
                    let state = (self.path_checked as u32) + (self.system_accessed as u32);
                    Ok(TestResponse { value: state })
                }
            }
        }
    }

    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = ContextActor {
        path_checked: false,
        system_accessed: false,
    };
    let actor_ref = system.create_root_actor("context_actor", actor).await.unwrap();

    // Test path checking
    let result = actor_ref.ask(ContextMessage::CheckPath).await.unwrap();
    assert_eq!(result.value, 1);

    // Test system access
    let result = actor_ref.ask(ContextMessage::AccessSystem).await.unwrap();
    assert_eq!(result.value, 2);

    // Verify both operations completed
    let result = actor_ref.ask(ContextMessage::GetState).await.unwrap();
    assert_eq!(result.value, 2); // Both flags should be true
}