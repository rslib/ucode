use std::time::{Duration, Instant};

use ucode_core::{AgentResult, AgentSpec, AgentState, Orchestrator};

fn spec(name: &str) -> AgentSpec {
    AgentSpec {
        name: name.into(),
        description: format!("{name} agent"),
    }
}

#[tokio::test]
async fn spawn_and_wait() {
    let orch = Orchestrator::new();
    let handle = orch
        .spawn(spec("ok"), |_id, _cancel| async { Ok("done".into()) })
        .await;

    match Orchestrator::wait(handle).await {
        AgentResult::Completed { output, .. } => assert_eq!(output, "done"),
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[tokio::test]
async fn spawn_and_wait_failure() {
    let orch = Orchestrator::new();
    let handle = orch
        .spawn(spec("fail"), |_id, _cancel| async { Err("oops".into()) })
        .await;

    match Orchestrator::wait(handle).await {
        AgentResult::Failed { error, .. } => assert_eq!(error, "oops"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn spawn_three_wait_all() {
    let orch = Orchestrator::new();

    let h1 = orch
        .spawn(spec("a"), |id, _| async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(id)
        })
        .await;
    let h2 = orch
        .spawn(spec("b"), |id, _| async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(id)
        })
        .await;
    let h3 = orch
        .spawn(spec("c"), |id, _| async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(id)
        })
        .await;

    let results = Orchestrator::wait_all(vec![h1, h2, h3]).await;
    assert_eq!(results.len(), 3);

    let ids: Vec<&str> = results
        .iter()
        .map(|r| match r {
            AgentResult::Completed { agent_id, .. } => agent_id.as_str(),
            other => panic!("expected Completed, got {other:?}"),
        })
        .collect();

    assert!(ids.contains(&"agent_0"));
    assert!(ids.contains(&"agent_1"));
    assert!(ids.contains(&"agent_2"));
}

#[tokio::test]
async fn wait_any_returns_fastest() {
    let orch = Orchestrator::new();

    let h_slow1 = orch
        .spawn(spec("slow1"), |id, _| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(id)
        })
        .await;
    let h_fast = orch
        .spawn(spec("fast"), |id, _| async move { Ok(id) })
        .await;
    let h_slow2 = orch
        .spawn(spec("slow2"), |id, _| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(id)
        })
        .await;

    let (result, remaining) = Orchestrator::wait_any(vec![h_slow1, h_fast, h_slow2]).await;

    // The fast agent (agent_1) should complete first.
    match &result {
        AgentResult::Completed { agent_id, .. } => assert_eq!(agent_id, "agent_1"),
        other => panic!("expected Completed for fast agent, got {other:?}"),
    }

    assert_eq!(remaining.len(), 2);
}

#[tokio::test]
async fn cancel_agent() {
    let orch = Orchestrator::new();

    let handle = orch
        .spawn(spec("long"), |_id, cancel| async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => Ok("done".into()),
                _ = cancel => Err("cancelled".into()),
            }
        })
        .await;

    let agent_id = handle.id.clone();
    let cancelled_id = Orchestrator::cancel(handle);
    assert_eq!(cancelled_id, agent_id);
}

#[tokio::test]
async fn list_agents() {
    let orch = Orchestrator::new();

    let (tx1, rx1) = tokio::sync::oneshot::channel::<()>();
    let (tx2, rx2) = tokio::sync::oneshot::channel::<()>();

    let _h1 = orch
        .spawn(spec("a"), |_, _| async move {
            let _ = rx1.await;
            Ok("done".into())
        })
        .await;
    let _h2 = orch
        .spawn(spec("b"), |_, _| async move {
            let _ = rx2.await;
            Ok("done".into())
        })
        .await;

    // Give the spawned tasks a moment to register.
    tokio::time::sleep(Duration::from_millis(5)).await;

    let agents = orch.list_agents().await;
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|a| a.state == AgentState::Running));

    let _ = tx1.send(());
    let _ = tx2.send(());
}

#[tokio::test]
async fn parent_responsive_during_child_work() {
    let orch = Orchestrator::new();

    let handle = orch
        .spawn(spec("slow"), |_, _| async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok("done".into())
        })
        .await;

    let start = Instant::now();

    // Parent does work immediately — does not block on the agent.
    let mut counter = 0u32;
    for _ in 0..1000 {
        counter += 1;
    }
    let parent_elapsed = start.elapsed();

    assert!(
        parent_elapsed < Duration::from_millis(50),
        "parent work took too long: {parent_elapsed:?}"
    );
    assert_eq!(counter, 1000);

    match Orchestrator::wait(handle).await {
        AgentResult::Completed { .. } => {}
        other => panic!("expected Completed, got {other:?}"),
    }

    let total = start.elapsed();
    assert!(
        total >= Duration::from_millis(100),
        "total elapsed should be at least 100ms, got {total:?}"
    );
}
