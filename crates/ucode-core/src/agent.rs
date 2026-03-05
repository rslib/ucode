use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::select_all;
use tokio::sync::{Mutex, oneshot};

/// Unique agent identifier.
pub type AgentId = String;

/// Specification for spawning an agent.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
}

/// The result produced by a completed agent.
#[derive(Debug, Clone)]
pub enum AgentResult {
    Completed { agent_id: AgentId, output: String },
    Failed { agent_id: AgentId, error: String },
    Cancelled { agent_id: AgentId },
}

/// Current state of an agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Info about a tracked agent.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: AgentId,
    pub spec: AgentSpec,
    pub state: AgentState,
}

/// Handle to a spawned agent, used for waiting or cancelling.
pub struct AgentHandle {
    pub id: AgentId,
    result_rx: oneshot::Receiver<AgentResult>,
    cancel_tx: oneshot::Sender<()>,
}

/// Manages spawned agents and their lifecycle.
pub struct Orchestrator {
    agents: Arc<Mutex<HashMap<AgentId, AgentInfo>>>,
    next_id: Arc<Mutex<u64>>,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
        }
    }

    /// Spawn an agent that runs the given async task.
    ///
    /// The task receives the agent ID and a cancellation receiver. It must return
    /// `Ok(output)` on success or `Err(message)` on failure. Checking the cancel
    /// receiver is the task's responsibility.
    pub async fn spawn<F, Fut>(&self, spec: AgentSpec, task: F) -> AgentHandle
    where
        F: FnOnce(AgentId, oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        let agents = Arc::clone(&self.agents);

        let id = {
            let mut n = self.next_id.lock().await;
            let id = format!("agent_{}", *n);
            *n += 1;
            id
        };

        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (result_tx, result_rx) = oneshot::channel::<AgentResult>();

        {
            let mut map = agents.lock().await;
            map.insert(
                id.clone(),
                AgentInfo {
                    id: id.clone(),
                    spec: spec.clone(),
                    state: AgentState::Running,
                },
            );
        }

        let agent_id = id.clone();
        tokio::spawn(async move {
            let outcome = task(agent_id.clone(), cancel_rx).await;
            let result = match outcome {
                Ok(output) => {
                    let mut map = agents.lock().await;
                    if let Some(info) = map.get_mut(&agent_id) {
                        info.state = AgentState::Completed;
                    }
                    AgentResult::Completed {
                        agent_id: agent_id.clone(),
                        output,
                    }
                }
                Err(error) => {
                    let mut map = agents.lock().await;
                    if let Some(info) = map.get_mut(&agent_id) {
                        info.state = AgentState::Failed;
                    }
                    AgentResult::Failed {
                        agent_id: agent_id.clone(),
                        error,
                    }
                }
            };
            // Ignore send error — handle may have been dropped (e.g. cancelled).
            let _ = result_tx.send(result);
        });

        AgentHandle {
            id,
            result_rx,
            cancel_tx,
        }
    }

    /// Wait for a single agent to complete. Consumes the handle.
    pub async fn wait(handle: AgentHandle) -> AgentResult {
        match handle.result_rx.await {
            Ok(r) => r,
            Err(_) => AgentResult::Failed {
                agent_id: handle.id,
                error: "agent task dropped unexpectedly".into(),
            },
        }
    }

    /// Wait for all agents to complete. Returns results in completion order.
    pub async fn wait_all(handles: Vec<AgentHandle>) -> Vec<AgentResult> {
        let futs: Vec<_> = handles.into_iter().map(Self::wait).collect();
        futures_util::future::join_all(futs).await
    }

    /// Wait for the first agent to complete. Returns its result and the remaining handles.
    pub async fn wait_any(handles: Vec<AgentHandle>) -> (AgentResult, Vec<AgentHandle>) {
        assert!(!handles.is_empty(), "wait_any requires at least one handle");

        // Decompose handles so we can reconstruct the remaining ones after select_all.
        let mut ids: Vec<AgentId> = Vec::with_capacity(handles.len());
        let mut cancel_txs: Vec<oneshot::Sender<()>> = Vec::with_capacity(handles.len());
        let mut rxs: Vec<oneshot::Receiver<AgentResult>> = Vec::with_capacity(handles.len());

        for h in handles {
            ids.push(h.id);
            cancel_txs.push(h.cancel_tx);
            rxs.push(h.result_rx);
        }

        // select_all returns (output, index, remaining_futures).
        let (outcome, completed_idx, remaining_rxs) = select_all(rxs).await;

        let result = match outcome {
            Ok(r) => r,
            Err(_) => AgentResult::Failed {
                agent_id: ids[completed_idx].clone(),
                error: "agent task dropped unexpectedly".into(),
            },
        };

        // Reconstruct handles for the remaining agents.
        // remaining_rxs is in the original order minus the completed index.
        let mut remaining = Vec::with_capacity(remaining_rxs.len());
        let mut rx_iter = remaining_rxs.into_iter();
        for (i, (id, cancel_tx)) in ids.into_iter().zip(cancel_txs).enumerate() {
            if i == completed_idx {
                continue;
            }
            let result_rx = rx_iter.next().expect("remaining_rxs count matches");
            remaining.push(AgentHandle {
                id,
                result_rx,
                cancel_tx,
            });
        }

        (result, remaining)
    }

    /// Cancel an agent by sending the cancellation signal. Returns the agent ID.
    ///
    /// The agent task is responsible for checking the signal via its cancel receiver.
    pub fn cancel(handle: AgentHandle) -> AgentId {
        let id = handle.id.clone();
        // Dropping result_rx signals the task that nobody is waiting.
        // Sending on cancel_tx signals the task to stop.
        let _ = handle.cancel_tx.send(());
        id
    }

    /// List all tracked agents and their current state.
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents.lock().await.values().cloned().collect()
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
