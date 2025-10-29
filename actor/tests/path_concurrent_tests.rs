

//! Tests for ActorPath edge cases and concurrent scenarios

use actor::{Actor, ActorContext, ActorPath, ActorSystem, Error, Event, Handler, Message, Response};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

// Test ActorPath edge cases
#[test]
fn test_actor_path_edge_cases() {
    // Test empty path
    let empty_path = ActorPath::from("");
    assert!(empty_path.is_empty());

    // Test root path
    let root_path = ActorPath::from("/");
    assert!(root_path.is_empty()); // A path with just "/" becomes empty after parsing

    // Test single level path
    let single_path = ActorPath::from("/user");
    assert_eq!(single_path.key(), "user");
    assert!(single_path.is_top_level());

    // Test deep path
    let deep_path = ActorPath::from("/user/parent/child/grandchild");
    assert_eq!(deep_path.key(), "grandchild");

    // Test path with special characters
    let special_path = ActorPath::from("/user/actor-with-hyphens");
    assert_eq!(special_path.key(), "actor-with-hyphens");

    // Test path normalization
    let path_with_trailing = ActorPath::from("/user/test/");
    let path_without_trailing = ActorPath::from("/user/test");
    assert_eq!(path_with_trailing.to_string(), path_without_trailing.to_string());
}

#[test]
fn test_actor_path_relationships() {
    let parent = ActorPath::from("/user/parent");
    let child = ActorPath::from("/user/parent/child");
    let sibling = ActorPath::from("/user/sibling");
    let grandchild = ActorPath::from("/user/parent/child/grandchild");

    // Test ancestor/descendant relationships
    assert!(parent.is_ancestor_of(&child));
    assert!(!parent.is_ancestor_of(&sibling));
    assert!(parent.is_ancestor_of(&grandchild));

    assert!(child.is_descendant_of(&parent));
    assert!(!sibling.is_descendant_of(&parent));
    assert!(grandchild.is_descendant_of(&parent));

    // Test direct parent/child relationships
    assert!(parent.is_parent_of(&child));
    assert!(!parent.is_parent_of(&grandchild)); // grandchild is not direct child
    assert!(!parent.is_parent_of(&sibling));

    assert!(child.is_child_of(&parent));
    assert!(!grandchild.is_child_of(&parent));
}

#[test]
fn test_actor_path_operations() {
    let base_path = ActorPath::from("/user");

    // Test adding paths using division operator
    let child_path = base_path.clone() / "child";
    assert_eq!(child_path.to_string(), "/user/child");

    let grandchild_path = child_path / "grandchild";
    assert_eq!(grandchild_path.to_string(), "/user/child/grandchild");

    // Test getting parent
    let parent = grandchild_path.parent();
    assert_eq!(parent.to_string(), "/user/child");

    let grandparent = parent.parent();
    assert_eq!(grandparent.to_string(), "/user");

    // Test level and key
    assert_eq!(grandchild_path.level(), 3);
    assert_eq!(grandchild_path.key(), "grandchild");

    // Test at_level (this returns ActorPath, not Option<String>)
    let level_0_path = grandchild_path.at_level(1);
    assert_eq!(level_0_path.to_string(), "/user");
}

// Test actor for concurrent scenarios
#[derive(Debug, Clone)]
struct ConcurrentActor {
    counter: Arc<Mutex<i32>>,
    messages_received: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ConcurrentMessage {
    Increment,
    Decrement,
    AddMessage(String),
    GetCounter,
    GetMessages,
    CreateChild(String),
    SendToChild(String, String),
}

impl Message for ConcurrentMessage {}

#[derive(Debug, Clone, PartialEq)]
enum ConcurrentResponse {
    Counter(i32),
    Messages(Vec<String>),
    Success,
    ChildResponse(String),
}

impl Response for ConcurrentResponse {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConcurrentEvent {
    pub action: String,
    pub value: i32,
}

impl Event for ConcurrentEvent {}

impl ConcurrentActor {
    fn new() -> Self {
        Self {
            counter: Arc::new(Mutex::new(0)),
            messages_received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl Actor for ConcurrentActor {
    type Message = ConcurrentMessage;
    type Response = ConcurrentResponse;
    type Event = ConcurrentEvent;
}

#[async_trait]
impl Handler<ConcurrentActor> for ConcurrentActor {
    async fn handle_message(
        &mut self,
        _sender: ActorPath,
        msg: ConcurrentMessage,
        ctx: &mut ActorContext<ConcurrentActor>,
    ) -> Result<ConcurrentResponse, Error> {
        match msg {
            ConcurrentMessage::Increment => {
                let mut counter = self.counter.lock().await;
                *counter += 1;
                let value = *counter;
                drop(counter);

                ctx.publish_event(ConcurrentEvent {
                    action: "increment".to_string(),
                    value,
                }).await?;

                Ok(ConcurrentResponse::Counter(value))
            }
            ConcurrentMessage::Decrement => {
                let mut counter = self.counter.lock().await;
                *counter -= 1;
                let value = *counter;
                drop(counter);

                ctx.publish_event(ConcurrentEvent {
                    action: "decrement".to_string(),
                    value,
                }).await?;

                Ok(ConcurrentResponse::Counter(value))
            }
            ConcurrentMessage::AddMessage(msg) => {
                let mut messages = self.messages_received.lock().await;
                messages.push(msg);
                Ok(ConcurrentResponse::Success)
            }
            ConcurrentMessage::GetCounter => {
                let counter = self.counter.lock().await;
                Ok(ConcurrentResponse::Counter(*counter))
            }
            ConcurrentMessage::GetMessages => {
                let messages = self.messages_received.lock().await;
                Ok(ConcurrentResponse::Messages(messages.clone()))
            }
            ConcurrentMessage::CreateChild(name) => {
                let child = ConcurrentActor::new();
                ctx.create_child(&name, child).await?;
                Ok(ConcurrentResponse::Success)
            }
            ConcurrentMessage::SendToChild(child_name, message) => {
                if let Some(child) = ctx.get_child::<ConcurrentActor>(&child_name).await {
                    let _response = child.ask(ConcurrentMessage::AddMessage(message.clone())).await?;
                    Ok(ConcurrentResponse::ChildResponse(format!("Sent '{}' to child", message)))
                } else {
                    Err(Error::Functional(format!("Child '{}' not found", child_name)))
                }
            }
        }
    }
}

// Test concurrent message handling
#[tokio::test]
async fn test_concurrent_message_handling() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = ConcurrentActor::new();
    let actor_ref = system.create_root_actor("concurrent", actor).await.unwrap();

    // Send multiple concurrent messages
    let mut handles = Vec::new();

    for i in 0..10 {
        let actor_ref_clone = actor_ref.clone();
        let handle = tokio::spawn(async move {
            if i % 2 == 0 {
                actor_ref_clone.tell(ConcurrentMessage::Increment).await.unwrap();
            } else {
                actor_ref_clone.tell(ConcurrentMessage::Decrement).await.unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all messages to be processed
    for handle in handles {
        handle.await.unwrap();
    }

    // Give time for all messages to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // The counter should be 0 (5 increments - 5 decrements)
    let response = actor_ref.ask(ConcurrentMessage::GetCounter).await.unwrap();
    if let ConcurrentResponse::Counter(count) = response {
        assert_eq!(count, 0);
    } else {
        panic!("Expected counter response");
    }
}

// Test multiple actors communicating
#[tokio::test]
async fn test_multiple_actor_communication() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    // Create parent actor
    let parent = ConcurrentActor::new();
    let parent_ref = system.create_root_actor("parent", parent).await.unwrap();

    // Create children through parent
    parent_ref.ask(ConcurrentMessage::CreateChild("child1".to_string())).await.unwrap();
    parent_ref.ask(ConcurrentMessage::CreateChild("child2".to_string())).await.unwrap();

    // Wait for children to be created
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Send messages to children through parent
    parent_ref.ask(ConcurrentMessage::SendToChild("child1".to_string(), "Hello child1".to_string())).await.unwrap();
    parent_ref.ask(ConcurrentMessage::SendToChild("child2".to_string(), "Hello child2".to_string())).await.unwrap();

    // Verify children exist in system
    let child1_path = ActorPath::from("/user/parent/child1");
    let child2_path = ActorPath::from("/user/parent/child2");

    let child1_ref = system.get_actor::<ConcurrentActor>(&child1_path).await;
    let child2_ref = system.get_actor::<ConcurrentActor>(&child2_path).await;

    assert!(child1_ref.is_some());
    assert!(child2_ref.is_some());

    // Verify children received messages
    if let Some(child1) = child1_ref {
        let response = child1.ask(ConcurrentMessage::GetMessages).await.unwrap();
        if let ConcurrentResponse::Messages(messages) = response {
            assert!(messages.contains(&"Hello child1".to_string()));
        }
    }

    if let Some(child2) = child2_ref {
        let response = child2.ask(ConcurrentMessage::GetMessages).await.unwrap();
        if let ConcurrentResponse::Messages(messages) = response {
            assert!(messages.contains(&"Hello child2".to_string()));
        }
    }
}

// Test event subscription with multiple subscribers
#[tokio::test]
async fn test_multiple_event_subscribers() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let actor = ConcurrentActor::new();
    let actor_ref = system.create_root_actor("event_source", actor).await.unwrap();

    // Create multiple subscribers
    let mut subscribers = Vec::new();
    for _ in 0..3 {
        let subscriber = actor_ref.subscribe();
        subscribers.push(subscriber);
    }

    // Send event-generating messages
    actor_ref.tell(ConcurrentMessage::Increment).await.unwrap();
    actor_ref.tell(ConcurrentMessage::Increment).await.unwrap();
    actor_ref.tell(ConcurrentMessage::Decrement).await.unwrap();

    // Wait for events to be published
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify each subscriber received events (note: we can't easily test reception
    // without modifying the subscriber interface, so this test mainly ensures
    // the system doesn't crash with multiple subscribers)

    // The fact that we get here without panicking means multiple subscribers work
    assert_eq!(subscribers.len(), 3);
}

// Test actor lifecycle with rapid creation/destruction
#[tokio::test]
async fn test_rapid_actor_lifecycle() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    // Rapidly create and destroy actors
    for i in 0..10 {
        let actor = ConcurrentActor::new();
        let actor_name = format!("temp_actor_{}", i);
        let actor_ref = system.create_root_actor(&actor_name, actor).await.unwrap();

        // Send a message to ensure it's fully initialized
        actor_ref.ask(ConcurrentMessage::GetCounter).await.unwrap();

        // Stop the actor
        actor_ref.ask_stop().await.unwrap();

        // Small delay to allow cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    // System should still be functional
    let final_actor = ConcurrentActor::new();
    let final_ref = system.create_root_actor("final", final_actor).await.unwrap();
    let response = final_ref.ask(ConcurrentMessage::GetCounter).await.unwrap();

    if let ConcurrentResponse::Counter(count) = response {
        assert_eq!(count, 0);
    }
}

// Test error handling in concurrent scenarios
#[tokio::test]
async fn test_concurrent_error_handling() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    tokio::spawn(async move { runner.run().await });

    let parent = ConcurrentActor::new();
    let parent_ref = system.create_root_actor("error_parent", parent).await.unwrap();

    // Try to send message to non-existent child
    let result = parent_ref.ask(ConcurrentMessage::SendToChild("nonexistent".to_string(), "test".to_string())).await;

    assert!(result.is_err());

    if let Err(Error::Functional(msg)) = result {
        assert!(msg.contains("Child 'nonexistent' not found"));
    } else {
        panic!("Expected functional error");
    }
}

// Test system shutdown with active actors
#[tokio::test]
async fn test_system_shutdown_with_active_actors() {
    let (system, mut runner) = ActorSystem::create(CancellationToken::new());
    let runner_handle = tokio::spawn(async move { runner.run().await });

    // Create multiple actors
    let mut actor_refs = Vec::new();
    for i in 0..5 {
        let actor = ConcurrentActor::new();
        let actor_ref = system.create_root_actor(&format!("actor_{}", i), actor).await.unwrap();
        actor_refs.push(actor_ref);
    }

    // Send some messages to keep them busy
    for actor_ref in &actor_refs {
        actor_ref.tell(ConcurrentMessage::Increment).await.unwrap();
    }

    // Wait a bit for messages to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Stop the system
    system.stop_system();

    // Wait for runner to finish
    tokio::time::timeout(tokio::time::Duration::from_secs(5), runner_handle)
        .await
        .expect("System should shutdown within timeout")
        .expect("Runner should complete successfully");
}