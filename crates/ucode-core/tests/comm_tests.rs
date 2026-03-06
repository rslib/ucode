use serde_json::json;
use ucode_core::{AgentMessage, CommBus, CommError, CommPolicy};

fn enabled_bus(max_message_size: usize) -> CommBus {
    CommBus::new(CommPolicy::Enabled { max_message_size })
}

fn disabled_bus() -> CommBus {
    CommBus::new(CommPolicy::Disabled)
}

// 1. Two agents exchange messages.
#[tokio::test]
async fn two_agents_exchange_messages() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    bus.send(&a, &b, json!({"hello": "world"}))
        .await
        .expect("send should succeed");

    let msgs: Vec<AgentMessage> = bus.recv(&b).await.expect("recv should succeed");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].from, a);
    assert_eq!(msgs[0].to, b);
    assert_eq!(msgs[0].payload, json!({"hello": "world"}));
}

// 2. Communication disabled by policy.
#[tokio::test]
async fn send_disabled_policy() {
    let bus = disabled_bus();
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    let err: CommError = bus
        .send(&a, &b, json!("hi"))
        .await
        .expect_err("send should fail when disabled");
    assert!(matches!(err, CommError::Disabled));
}

// 3. Message size limit.
#[tokio::test]
async fn message_size_limit() {
    let bus = enabled_bus(10); // tiny limit
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    // A string longer than 10 bytes when serialized.
    let big = json!("this string is definitely longer than ten bytes");
    let err: CommError = bus
        .send(&a, &b, big)
        .await
        .expect_err("oversized send should fail");
    assert!(matches!(err, CommError::MessageTooLarge { .. }));
}

// 4. Unknown recipient.
#[tokio::test]
async fn unknown_recipient() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    bus.register(&a).await;

    let err: CommError = bus
        .send(&a, &"ghost".to_string(), json!(42))
        .await
        .expect_err("send to unknown agent should fail");
    assert!(matches!(err, CommError::AgentNotFound(id) if id == "ghost"));
}

// 5. Shared board write and read.
#[tokio::test]
async fn shared_board_write_read() {
    let bus = enabled_bus(1024);

    bus.board_write("key1".into(), json!(99))
        .await
        .expect("board_write should succeed");
    let val: Option<serde_json::Value> = bus
        .board_read("key1")
        .await
        .expect("board_read should succeed");
    assert_eq!(val, Some(json!(99)));

    let missing: Option<serde_json::Value> = bus
        .board_read("no_such_key")
        .await
        .expect("board_read missing key should succeed");
    assert_eq!(missing, None);
}

// 6. Board operations respect disabled policy.
#[tokio::test]
async fn board_disabled_policy() {
    let bus = disabled_bus();

    let write_err: CommError = bus
        .board_write("k".into(), json!(1))
        .await
        .expect_err("board_write should fail when disabled");
    assert!(matches!(write_err, CommError::Disabled));

    let read_err: CommError = bus
        .board_read("k")
        .await
        .expect_err("board_read should fail when disabled");
    assert!(matches!(read_err, CommError::Disabled));
}

// 7. Audit log captures all sent messages.
#[tokio::test]
async fn audit_log_captures_messages() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    bus.send(&a, &b, json!(1))
        .await
        .expect("send a->b should succeed");
    bus.send(&b, &a, json!(2))
        .await
        .expect("send b->a should succeed");

    let log: Vec<AgentMessage> = bus.audit_log().await;
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].payload, json!(1));
    assert_eq!(log[1].payload, json!(2));
}

// 8. Unregistered agent cannot receive messages.
#[tokio::test]
async fn unregistered_agent_cannot_recv() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();

    bus.register(&a).await;
    bus.send(&a, &a, json!("self"))
        .await
        .expect("self-send should succeed");

    bus.unregister(&a).await;

    let err: CommError = bus
        .recv(&a)
        .await
        .expect_err("recv after unregister should fail");
    assert!(matches!(err, CommError::AgentNotFound(id) if id == "agent_a"));
}

// 9. Multiple messages are all drained by recv.
#[tokio::test]
async fn multiple_messages_drained() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    for i in 0..5u32 {
        bus.send(&a, &b, json!(i))
            .await
            .expect("send should succeed");
    }

    let msgs: Vec<AgentMessage> = bus.recv(&b).await.expect("recv should succeed");
    assert_eq!(msgs.len(), 5);
    for (i, msg) in msgs.iter().enumerate() {
        assert_eq!(msg.payload, json!(i as u32));
    }

    // Mailbox is now empty.
    let empty: Vec<AgentMessage> = bus.recv(&b).await.expect("second recv should succeed");
    assert!(empty.is_empty());
}

// 10. Pending count without draining.
#[tokio::test]
async fn pending_count_without_drain() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&a).await;
    bus.register(&b).await;

    assert_eq!(
        bus.pending_count(&b)
            .await
            .expect("pending_count should succeed"),
        0
    );

    bus.send(&a, &b, json!("x"))
        .await
        .expect("send x should succeed");
    bus.send(&a, &b, json!("y"))
        .await
        .expect("send y should succeed");

    assert_eq!(
        bus.pending_count(&b)
            .await
            .expect("pending_count should succeed"),
        2
    );

    // Drain and confirm count resets.
    let _: Vec<AgentMessage> = bus.recv(&b).await.expect("drain should succeed");
    assert_eq!(
        bus.pending_count(&b)
            .await
            .expect("pending_count after drain should succeed"),
        0
    );
}

// Bonus: pending_count on unregistered agent returns AgentNotFound.
#[tokio::test]
async fn pending_count_unregistered() {
    let bus = enabled_bus(1024);
    let err: CommError = bus
        .pending_count(&"ghost".to_string())
        .await
        .expect_err("pending_count on unknown agent should fail");
    assert!(matches!(err, CommError::AgentNotFound(_)));
}

// Bonus: register is idempotent — re-registering doesn't wipe existing messages.
#[tokio::test]
async fn register_idempotent() {
    let bus = enabled_bus(1024);
    let a = "agent_a".to_string();
    let b = "agent_b".to_string();

    bus.register(&b).await;
    bus.register(&a).await;
    bus.send(&a, &b, json!("first"))
        .await
        .expect("send should succeed");

    // Re-register should not clear the inbox.
    bus.register(&b).await;

    let msgs: Vec<AgentMessage> = bus.recv(&b).await.expect("recv should succeed");
    assert_eq!(msgs.len(), 1);
}
