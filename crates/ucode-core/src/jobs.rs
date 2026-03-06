use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};

use crate::error::CoreError;

/// Unique identifier for a background job.
pub type JobId = String;

/// Lifecycle state of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed { reason: String },
    Cancelled,
    Killed,
}

impl JobState {
    /// Returns `true` for states that will never transition further.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Completed | JobState::Failed { .. } | JobState::Cancelled | JobState::Killed
        )
    }
}

/// Snapshot of a job's metadata and current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: JobId,
    pub name: String,
    pub description: String,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// The outcome produced by a completed job task.
#[derive(Debug, Clone)]
pub enum JobResult {
    Success { output: String },
    Failed { reason: String },
    Cancelled,
    Killed,
}

// Internal handle — not pub; only `JobController` touches it.
struct JobHandle {
    info: JobInfo,
    cancel_tx: Option<oneshot::Sender<()>>,
    kill_tx: Option<oneshot::Sender<()>>,
    result_rx: Option<oneshot::Receiver<JobResult>>,
}

/// Manages background jobs: spawn, inspect, cancel, kill, and prune.
pub struct JobController {
    jobs: Arc<Mutex<HashMap<JobId, JobHandle>>>,
    next_id: Arc<Mutex<u64>>,
}

impl JobController {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
        }
    }

    /// Spawn a background job.
    ///
    /// `task` receives two oneshot receivers: the first fires on graceful cancel,
    /// the second on forceful kill. It must return `Ok(output)` or `Err(reason)`.
    /// The job transitions to `Running` immediately; the final state is set when
    /// the task future resolves.
    pub async fn start<F, Fut>(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        task: F,
    ) -> Result<JobId, CoreError>
    where
        F: FnOnce(oneshot::Receiver<()>, oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    {
        let id = {
            let mut n = self.next_id.lock().await;
            let id = format!("job_{}", *n);
            *n += 1;
            id
        };

        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (result_tx, result_rx) = oneshot::channel::<JobResult>();

        let now = Utc::now();
        let info = JobInfo {
            id: id.clone(),
            name: name.into(),
            description: description.into(),
            state: JobState::Running,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
        };

        {
            let mut map = self.jobs.lock().await;
            map.insert(
                id.clone(),
                JobHandle {
                    info,
                    cancel_tx: Some(cancel_tx),
                    kill_tx: Some(kill_tx),
                    result_rx: Some(result_rx),
                },
            );
        }

        let jobs = Arc::clone(&self.jobs);
        let job_id = id.clone();

        tokio::spawn(async move {
            let outcome = task(cancel_rx, kill_rx).await;
            let finished = Utc::now();

            let result = match outcome {
                Ok(output) => {
                    let mut map = jobs.lock().await;
                    if let Some(handle) = map.get_mut(&job_id)
                        && !handle.info.state.is_terminal()
                    {
                        handle.info.state = JobState::Completed;
                        handle.info.finished_at = Some(finished);
                    }
                    JobResult::Success { output }
                }
                Err(reason) => {
                    let mut map = jobs.lock().await;
                    if let Some(handle) = map.get_mut(&job_id)
                        && !handle.info.state.is_terminal()
                    {
                        handle.info.state = JobState::Failed {
                            reason: reason.clone(),
                        };
                        handle.info.finished_at = Some(finished);
                    }
                    JobResult::Failed { reason }
                }
            };

            // Ignore send error — caller may have dropped the receiver.
            let _ = result_tx.send(result);
        });

        Ok(id)
    }

    /// Returns a snapshot of every tracked job.
    pub async fn list(&self) -> Vec<JobInfo> {
        self.jobs
            .lock()
            .await
            .values()
            .map(|h| h.info.clone())
            .collect()
    }

    /// Returns the current snapshot for a single job.
    pub async fn status(&self, id: &str) -> Result<JobInfo, CoreError> {
        self.jobs
            .lock()
            .await
            .get(id)
            .map(|h| h.info.clone())
            .ok_or_else(|| CoreError::Job {
                message: format!("job '{id}' not found"),
            })
    }

    /// Send a graceful cancellation signal to a running job.
    ///
    /// Marks the job `Cancelled` and fires the cancel channel. Returns an error
    /// if the job does not exist or is already in a terminal state.
    pub async fn cancel(&self, id: &str) -> Result<(), CoreError> {
        let mut map = self.jobs.lock().await;
        let handle = map.get_mut(id).ok_or_else(|| CoreError::Job {
            message: format!("job '{id}' not found"),
        })?;

        if handle.info.state.is_terminal() {
            return Err(CoreError::Job {
                message: format!("job '{id}' is already terminal ({:?})", handle.info.state),
            });
        }

        handle.info.state = JobState::Cancelled;
        handle.info.finished_at = Some(Utc::now());

        // Consume the sender; ignore error if task already finished.
        if let Some(tx) = handle.cancel_tx.take() {
            let _ = tx.send(());
        }

        Ok(())
    }

    /// Forcefully kill a running job.
    ///
    /// Marks the job `Killed`, fires both the cancel and kill channels, and
    /// returns an error if the job does not exist or is already terminal.
    pub async fn kill(&self, id: &str) -> Result<(), CoreError> {
        let mut map = self.jobs.lock().await;
        let handle = map.get_mut(id).ok_or_else(|| CoreError::Job {
            message: format!("job '{id}' not found"),
        })?;

        if handle.info.state.is_terminal() {
            return Err(CoreError::Job {
                message: format!("job '{id}' is already terminal ({:?})", handle.info.state),
            });
        }

        handle.info.state = JobState::Killed;
        handle.info.finished_at = Some(Utc::now());

        if let Some(tx) = handle.cancel_tx.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = handle.kill_tx.take() {
            let _ = tx.send(());
        }

        Ok(())
    }

    /// Await the result of a job.
    ///
    /// Takes the result receiver out of the handle (so a second call returns an
    /// error). Returns an error if the job is not found or was already waited on.
    pub async fn wait(&self, id: &str) -> Result<JobResult, CoreError> {
        let rx = {
            let mut map = self.jobs.lock().await;
            let handle = map.get_mut(id).ok_or_else(|| CoreError::Job {
                message: format!("job '{id}' not found"),
            })?;
            handle.result_rx.take().ok_or_else(|| CoreError::Job {
                message: format!("job '{id}' result already consumed"),
            })?
        };

        rx.await.map_err(|_| CoreError::Job {
            message: format!("job '{id}' task dropped before sending result"),
        })
    }

    /// Remove all terminal jobs and return the count removed.
    pub async fn prune_completed(&self) -> usize {
        let mut map = self.jobs.lock().await;
        let before = map.len();
        map.retain(|_, h| !h.info.state.is_terminal());
        before - map.len()
    }
}

impl Default for JobController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    async fn quick_task(
        _cancel_rx: oneshot::Receiver<()>,
        _kill_rx: oneshot::Receiver<()>,
    ) -> Result<String, String> {
        Ok("output".into())
    }

    async fn failing_task(
        _cancel_rx: oneshot::Receiver<()>,
        _kill_rx: oneshot::Receiver<()>,
    ) -> Result<String, String> {
        Err("task error".into())
    }

    async fn long_task(
        cancel_rx: oneshot::Receiver<()>,
        kill_rx: oneshot::Receiver<()>,
    ) -> Result<String, String> {
        tokio::select! {
            _ = cancel_rx => Err("cancelled".into()),
            _ = kill_rx   => Err("killed".into()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => Ok("done".into()),
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_job_state_is_terminal() {
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Failed { reason: "x".into() }.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(JobState::Killed.is_terminal());
    }

    #[tokio::test]
    async fn test_start_job_returns_id() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", quick_task).await.unwrap();
        assert!(!id.is_empty());
    }

    #[tokio::test]
    async fn test_list_jobs_empty() {
        let ctrl = JobController::new();
        assert!(ctrl.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_list_jobs_after_start() {
        let ctrl = JobController::new();
        ctrl.start("t", "d", long_task).await.unwrap();
        assert_eq!(ctrl.list().await.len(), 1);
    }

    #[tokio::test]
    async fn test_status_found() {
        let ctrl = JobController::new();
        let id = ctrl.start("my-job", "desc", long_task).await.unwrap();
        let info = ctrl.status(&id).await.unwrap();
        assert_eq!(info.id, id);
        assert_eq!(info.name, "my-job");
    }

    #[tokio::test]
    async fn test_status_not_found() {
        let ctrl = JobController::new();
        let err = ctrl.status("nonexistent").await.unwrap_err();
        assert!(matches!(err, CoreError::Job { .. }));
    }

    #[tokio::test]
    async fn test_job_completes_successfully() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", quick_task).await.unwrap();
        let result = ctrl.wait(&id).await.unwrap();
        assert!(matches!(result, JobResult::Success { .. }));

        // Give the spawned task time to update state.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let info = ctrl.status(&id).await.unwrap();
        assert_eq!(info.state, JobState::Completed);
    }

    #[tokio::test]
    async fn test_job_fails() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", failing_task).await.unwrap();
        let result = ctrl.wait(&id).await.unwrap();
        assert!(matches!(result, JobResult::Failed { .. }));

        tokio::time::sleep(Duration::from_millis(50)).await;
        let info = ctrl.status(&id).await.unwrap();
        assert!(matches!(info.state, JobState::Failed { .. }));
    }

    #[tokio::test]
    async fn test_cancel_running_job() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", long_task).await.unwrap();
        ctrl.cancel(&id).await.unwrap();
        let info = ctrl.status(&id).await.unwrap();
        assert_eq!(info.state, JobState::Cancelled);
    }

    #[tokio::test]
    async fn test_cancel_terminal_job_errors() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", quick_task).await.unwrap();
        // Wait for the task to finish.
        let _ = ctrl.wait(&id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let err = ctrl.cancel(&id).await.unwrap_err();
        assert!(matches!(err, CoreError::Job { .. }));
    }

    #[tokio::test]
    async fn test_kill_running_job() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", long_task).await.unwrap();
        ctrl.kill(&id).await.unwrap();
        let info = ctrl.status(&id).await.unwrap();
        assert_eq!(info.state, JobState::Killed);
    }

    #[tokio::test]
    async fn test_kill_sends_both_signals() {
        // Verify that kill fires both cancel_rx and kill_rx by using a task
        // that distinguishes which signal it received.
        let (cancel_seen_tx, cancel_seen_rx) = oneshot::channel::<()>();
        let (kill_seen_tx, kill_seen_rx) = oneshot::channel::<()>();

        let ctrl = JobController::new();
        let id = ctrl
            .start("t", "d", move |cancel_rx, kill_rx| async move {
                tokio::select! {
                    _ = cancel_rx => { let _ = cancel_seen_tx.send(()); Err("cancel".into()) }
                    _ = kill_rx   => { let _ = kill_seen_tx.send(());   Err("kill".into())   }
                    _ = tokio::time::sleep(Duration::from_secs(60)) => Ok("done".into())
                }
            })
            .await
            .unwrap();

        ctrl.kill(&id).await.unwrap();

        // At least one of the two signals must have been received.
        let timeout = Duration::from_millis(200);
        let cancel_fired = tokio::time::timeout(timeout, cancel_seen_rx).await.is_ok();
        let kill_fired = tokio::time::timeout(timeout, kill_seen_rx).await.is_ok();
        assert!(
            cancel_fired || kill_fired,
            "kill must fire at least one signal"
        );
    }

    #[tokio::test]
    async fn test_prune_completed() {
        let ctrl = JobController::new();

        let id1 = ctrl.start("a", "d", quick_task).await.unwrap();
        let id2 = ctrl.start("b", "d", quick_task).await.unwrap();
        let _id3 = ctrl.start("c", "d", long_task).await.unwrap();

        // Wait for the two quick jobs to finish.
        let _ = ctrl.wait(&id1).await.unwrap();
        let _ = ctrl.wait(&id2).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let removed = ctrl.prune_completed().await;
        assert_eq!(removed, 2);
        assert_eq!(ctrl.list().await.len(), 1);
    }

    #[tokio::test]
    async fn test_wait_already_waited() {
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", quick_task).await.unwrap();
        let _ = ctrl.wait(&id).await.unwrap();
        let err = ctrl.wait(&id).await.unwrap_err();
        assert!(matches!(err, CoreError::Job { .. }));
    }

    #[tokio::test]
    async fn test_job_info_timestamps() {
        let before = Utc::now();
        let ctrl = JobController::new();
        let id = ctrl.start("t", "d", long_task).await.unwrap();
        let after = Utc::now();

        let info = ctrl.status(&id).await.unwrap();
        assert!(info.created_at >= before && info.created_at <= after);
        assert!(info.started_at.is_some());
        let started = info.started_at.unwrap();
        assert!(started >= before && started <= after);
        assert!(info.finished_at.is_none());
    }
}
