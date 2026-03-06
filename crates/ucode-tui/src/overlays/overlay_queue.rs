use std::collections::VecDeque;

/// Priority levels for system-initiated overlays.
/// Lower number = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayPriority {
    /// Safety-critical: approval modals.
    Approval = 0,
    /// Needs user decision: diff/patch review.
    Diff = 1,
}

/// A pending overlay request that hasn't been opened yet.
#[derive(Debug, Clone)]
pub enum OverlayRequest {
    Approval {
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
        tool_call_index: Option<usize>,
    },
    Diff {
        file_path: String,
        raw_diff: String,
        patch_id: Option<String>,
    },
}

impl OverlayRequest {
    pub fn priority(&self) -> OverlayPriority {
        match self {
            Self::Approval { .. } => OverlayPriority::Approval,
            Self::Diff { .. } => OverlayPriority::Diff,
        }
    }
}

/// Tracks the currently active overlay type (for preemption decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveOverlay {
    Approval,
    Diff,
}

impl ActiveOverlay {
    pub fn priority(self) -> OverlayPriority {
        match self {
            Self::Approval => OverlayPriority::Approval,
            Self::Diff => OverlayPriority::Diff,
        }
    }
}

/// Priority queue for system-initiated overlays with preemption support.
#[derive(Debug, Clone)]
pub struct OverlayQueue {
    /// What's currently showing (if anything).
    active: Option<ActiveOverlay>,
    /// Suspended overlays (preempted but state preserved in their modal structs).
    /// Most recently suspended is at the back.
    suspended: Vec<ActiveOverlay>,
    /// Pending requests not yet opened. Stored in arrival order.
    pending: VecDeque<OverlayRequest>,
}

impl OverlayQueue {
    pub fn new() -> Self {
        Self {
            active: None,
            suspended: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    /// Returns the currently active overlay type, if any.
    pub fn active(&self) -> Option<ActiveOverlay> {
        self.active
    }

    /// Returns the number of pending + suspended overlays waiting.
    pub fn waiting_count(&self) -> usize {
        self.suspended.len() + self.pending.len()
    }

    /// Submit a new overlay request. Returns what action the caller should take.
    pub fn submit(&mut self, request: OverlayRequest) -> OverlayAction {
        let new_priority = request.priority();

        match self.active {
            None => {
                let overlay_type = active_overlay_for(&request);
                self.active = Some(overlay_type);
                OverlayAction::Open(request)
            }
            Some(current) => {
                if new_priority < current.priority() {
                    // Higher priority — preempt current.
                    self.suspended.push(current);
                    let overlay_type = active_overlay_for(&request);
                    self.active = Some(overlay_type);
                    OverlayAction::Preempt(request)
                } else {
                    // Equal or lower priority — enqueue.
                    self.pending.push_back(request);
                    OverlayAction::Queued
                }
            }
        }
    }

    /// Called when the user dismisses the current overlay.
    /// Returns the next overlay to show (if any).
    pub fn dismiss_active(&mut self) -> Option<OverlayNext> {
        self.active = None;

        let best_pending_priority = self.best_pending_priority();
        let best_suspended_priority = self.suspended.last().map(|s| s.priority());

        match (best_pending_priority, best_suspended_priority) {
            (Some(pp), Some(sp)) => {
                if pp < sp {
                    // Pending has strictly higher priority — open it.
                    let request = self.pop_best_pending()?;
                    let overlay_type = active_overlay_for(&request);
                    self.active = Some(overlay_type);
                    Some(OverlayNext::Open(request))
                } else {
                    // Suspended has equal or higher priority — resume it.
                    let resumed = self.suspended.pop()?;
                    self.active = Some(resumed);
                    Some(OverlayNext::Resume(resumed))
                }
            }
            (Some(_), None) => {
                let request = self.pop_best_pending()?;
                let overlay_type = active_overlay_for(&request);
                self.active = Some(overlay_type);
                Some(OverlayNext::Open(request))
            }
            (None, Some(_)) => {
                let resumed = self.suspended.pop()?;
                self.active = Some(resumed);
                Some(OverlayNext::Resume(resumed))
            }
            (None, None) => None,
        }
    }

    fn best_pending_priority(&self) -> Option<OverlayPriority> {
        self.pending.iter().map(|r| r.priority()).min()
    }

    /// Remove and return the highest-priority pending request (FIFO within tier).
    fn pop_best_pending(&mut self) -> Option<OverlayRequest> {
        let best = self.best_pending_priority()?;
        let pos = self.pending.iter().position(|r| r.priority() == best)?;
        self.pending.remove(pos)
    }
}

impl Default for OverlayQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn active_overlay_for(request: &OverlayRequest) -> ActiveOverlay {
    match request {
        OverlayRequest::Approval { .. } => ActiveOverlay::Approval,
        OverlayRequest::Diff { .. } => ActiveOverlay::Diff,
    }
}

/// What the caller should do after submitting a request.
#[derive(Debug)]
pub enum OverlayAction {
    /// Open this overlay immediately (nothing was showing).
    Open(OverlayRequest),
    /// Preempt the current overlay: suspend it (set visible=false), then open this one.
    Preempt(OverlayRequest),
    /// Request was queued; do nothing now.
    Queued,
}

/// What to do after dismissing the active overlay.
#[derive(Debug)]
pub enum OverlayNext {
    /// Open a new pending request.
    Open(OverlayRequest),
    /// Resume a previously suspended overlay (just set visible=true).
    Resume(ActiveOverlay),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_request() -> OverlayRequest {
        OverlayRequest::Diff {
            file_path: "src/lib.rs".to_owned(),
            raw_diff: "+line".to_owned(),
            patch_id: None,
        }
    }

    fn approval_request() -> OverlayRequest {
        OverlayRequest::Approval {
            tool_name: "run_cmd".to_owned(),
            command: "cargo test".to_owned(),
            cwd: "/tmp".to_owned(),
            sandbox_label: "workspace".to_owned(),
            tool_call_index: Some(0),
        }
    }

    #[test]
    fn open_when_empty() {
        let mut q = OverlayQueue::new();
        assert!(q.active().is_none());
        let action = q.submit(diff_request());
        assert!(matches!(action, OverlayAction::Open(_)));
        assert_eq!(q.active(), Some(ActiveOverlay::Diff));
    }

    #[test]
    fn queue_equal_priority() {
        let mut q = OverlayQueue::new();
        q.submit(diff_request()); // opens
        let action = q.submit(diff_request()); // same priority → queued
        assert!(matches!(action, OverlayAction::Queued));
        assert_eq!(q.waiting_count(), 1);
    }

    #[test]
    fn queue_lower_priority() {
        let mut q = OverlayQueue::new();
        q.submit(approval_request()); // opens (P0)
        let action = q.submit(diff_request()); // lower priority → queued
        assert!(matches!(action, OverlayAction::Queued));
        assert_eq!(q.active(), Some(ActiveOverlay::Approval));
    }

    #[test]
    fn preempt_higher_priority() {
        let mut q = OverlayQueue::new();
        q.submit(diff_request()); // opens (P1)
        let action = q.submit(approval_request()); // higher priority → preempt
        assert!(matches!(action, OverlayAction::Preempt(_)));
        assert_eq!(q.active(), Some(ActiveOverlay::Approval));
        assert_eq!(q.waiting_count(), 1); // diff is suspended
    }

    #[test]
    fn dismiss_resumes_suspended() {
        let mut q = OverlayQueue::new();
        q.submit(diff_request()); // opens diff
        q.submit(approval_request()); // preempts → approval active, diff suspended
        let next = q.dismiss_active(); // dismiss approval
        assert!(matches!(
            next,
            Some(OverlayNext::Resume(ActiveOverlay::Diff))
        ));
        assert_eq!(q.active(), Some(ActiveOverlay::Diff));
    }

    #[test]
    fn dismiss_opens_pending() {
        let mut q = OverlayQueue::new();
        q.submit(approval_request()); // opens
        q.submit(diff_request()); // queued (lower priority)
        let next = q.dismiss_active(); // dismiss approval
        assert!(matches!(next, Some(OverlayNext::Open(_))));
        assert_eq!(q.active(), Some(ActiveOverlay::Diff));
    }

    #[test]
    fn dismiss_empty_returns_none() {
        let mut q = OverlayQueue::new();
        q.submit(diff_request());
        let next = q.dismiss_active();
        assert!(next.is_none());
        assert!(q.active().is_none());
    }

    #[test]
    fn fifo_within_same_priority() {
        let mut q = OverlayQueue::new();
        q.submit(approval_request()); // opens
        // Queue two diffs
        q.submit(OverlayRequest::Diff {
            file_path: "first.rs".to_owned(),
            raw_diff: "+a".to_owned(),
            patch_id: None,
        });
        q.submit(OverlayRequest::Diff {
            file_path: "second.rs".to_owned(),
            raw_diff: "+b".to_owned(),
            patch_id: None,
        });
        // Dismiss approval → first diff opens
        let next = q.dismiss_active();
        match next {
            Some(OverlayNext::Open(OverlayRequest::Diff { file_path, .. })) => {
                assert_eq!(file_path, "first.rs");
            }
            other => panic!("expected Open(Diff first.rs), got {other:?}"),
        }
        // Dismiss first diff → second diff opens
        let next2 = q.dismiss_active();
        match next2 {
            Some(OverlayNext::Open(OverlayRequest::Diff { file_path, .. })) => {
                assert_eq!(file_path, "second.rs");
            }
            other => panic!("expected Open(Diff second.rs), got {other:?}"),
        }
    }

    #[test]
    fn pending_higher_priority_beats_suspended() {
        let mut q = OverlayQueue::new();
        q.submit(diff_request()); // opens diff (P1)
        q.submit(approval_request()); // preempts → approval active, diff suspended
        // Queue another approval
        q.submit(approval_request()); // queued (same priority as active)
        // Dismiss active approval → pending approval (P0) beats suspended diff (P1)
        let next = q.dismiss_active();
        assert!(matches!(
            next,
            Some(OverlayNext::Open(OverlayRequest::Approval { .. }))
        ));
        // Dismiss that → now suspended diff resumes
        let next2 = q.dismiss_active();
        assert!(matches!(
            next2,
            Some(OverlayNext::Resume(ActiveOverlay::Diff))
        ));
    }

    #[test]
    fn suspended_preferred_over_pending_at_same_priority() {
        // If suspended and pending have same priority, prefer suspended (less jarring)
        let mut q = OverlayQueue::new();
        q.submit(diff_request()); // opens diff (P1)
        q.submit(approval_request()); // preempts → approval active, diff suspended
        // Queue another diff (same priority as suspended diff)
        q.submit(OverlayRequest::Diff {
            file_path: "other.rs".to_owned(),
            raw_diff: "+x".to_owned(),
            patch_id: None,
        });
        // Dismiss approval → suspended diff (P1) resumes (preferred over pending diff P1)
        let next = q.dismiss_active();
        assert!(matches!(
            next,
            Some(OverlayNext::Resume(ActiveOverlay::Diff))
        ));
    }

    #[test]
    fn priority_ordering() {
        assert!(OverlayPriority::Approval < OverlayPriority::Diff);
    }
}
