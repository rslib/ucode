use std::path::PathBuf;

use ucode_core::budget::CompactionRecord;
use ucode_core::{
    BudgetCheck, BudgetManager, CharEstimator, CompactionPolicy, CompactionStep, Event, Message,
    Session, TokenBudget, TokenCounter,
};

fn make_large_tool_result(size: usize) -> Message {
    Message::tool_result(
        "id",
        "tool",
        serde_json::Value::String("x".repeat(size)),
        false,
    )
}

fn estimator() -> CharEstimator {
    CharEstimator::default()
}

// ── Test 1 ────────────────────────────────────────────────────────────────────

/// An oversized transcript (many large tool results) must compact down to fit.
#[test]
fn oversized_transcript_compacts_and_fits() {
    // Budget: available = 500 - 100 = 400 tokens.
    let budget = TokenBudget::new(500, 100);
    let policy = CompactionPolicy::default();
    let manager = BudgetManager::new(budget, policy);
    let est = estimator();

    // Build ~20 messages: alternating user/assistant turns plus several large
    // tool results (3000+ chars each) that will blow the 400-token budget.
    let mut msgs: Vec<Message> = Vec::new();
    for i in 0..8 {
        msgs.push(Message::user(format!("user turn {i}")));
        msgs.push(Message::assistant(format!("assistant turn {i}")));
        msgs.push(make_large_tool_result(3000));
    }

    // Pre-condition: transcript is over budget.
    assert!(
        matches!(manager.check(&msgs, &est), BudgetCheck::OverBudget { .. }),
        "pre-condition: transcript must be over budget"
    );

    let records = manager.ensure_fits(&mut msgs, &est).unwrap();

    assert!(!records.is_empty(), "at least one compaction step must run");
    assert!(
        matches!(manager.check(&msgs, &est), BudgetCheck::Fits { .. }),
        "transcript must fit after compaction"
    );
}

// ── Test 2 ────────────────────────────────────────────────────────────────────

/// The last `pinned_recent_turns` messages must survive compaction unchanged.
#[test]
fn pinned_turns_preserved_after_compaction() {
    let est = estimator();

    // 15 messages total; the last 4 have distinctive text.
    let mut msgs: Vec<Message> = (0..11)
        .map(|i| make_large_tool_result(3000 + i * 50))
        .collect();

    let pinned = vec![
        Message::user("PINNED_USER_A unique marker alpha"),
        Message::assistant("PINNED_ASSISTANT_B unique marker beta"),
        Message::user("PINNED_USER_C unique marker gamma"),
        Message::assistant("PINNED_ASSISTANT_D unique marker delta"),
    ];
    msgs.extend(pinned.clone());

    // Budget tight enough to force compaction but large enough to hold the
    // 4 pinned messages plus a small placeholder.
    let (pinned_tokens, _) = est.count_messages(&pinned);
    let budget = TokenBudget::new(pinned_tokens + 200, 0);
    let policy = CompactionPolicy {
        pinned_recent_turns: 4,
        ..Default::default()
    };
    let manager = BudgetManager::new(budget, policy);

    manager.ensure_fits(&mut msgs, &est).unwrap();

    let tail: Vec<Message> = msgs[msgs.len() - 4..].to_vec();
    assert_eq!(
        tail, pinned,
        "last 4 messages must be the original pinned turns"
    );
}

// ── Test 3 ────────────────────────────────────────────────────────────────────

/// Every CompactionRecord in the audit trail must be internally consistent.
#[test]
fn compaction_records_are_auditable() {
    let budget = TokenBudget::new(200, 0);
    let policy = CompactionPolicy {
        pinned_recent_turns: 2,
        ..Default::default()
    };
    let manager = BudgetManager::new(budget, policy);
    let est = estimator();

    let mut msgs: Vec<Message> = (0..10).map(|_| make_large_tool_result(3000)).collect();

    let records = manager.ensure_fits(&mut msgs, &est).unwrap();

    assert!(!records.is_empty(), "must have at least one record");

    let valid_steps = [
        CompactionStep::TrimToolOutputs,
        CompactionStep::CompactOlderTurns,
        CompactionStep::DistillLongOutputs,
    ];

    for rec in &records {
        assert!(
            rec.tokens_before > 0,
            "tokens_before must be positive, got {}",
            rec.tokens_before
        );
        assert!(
            rec.tokens_after <= rec.tokens_before,
            "tokens_after ({}) must not exceed tokens_before ({})",
            rec.tokens_after,
            rec.tokens_before
        );
        assert!(
            valid_steps.contains(&rec.step),
            "unexpected step variant: {:?}",
            rec.step
        );
    }
}

// ── Test 4 ────────────────────────────────────────────────────────────────────

/// Compaction records written to a Session survive a save/load round-trip.
#[test]
fn session_compaction_log_persists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");

    let mut session = Session::new(PathBuf::from("/tmp"));

    // Populate the transcript with oversized messages.
    let mut msgs: Vec<Message> = (0..10).map(|_| make_large_tool_result(3000)).collect();
    for m in &msgs {
        session.push_message(m.clone());
    }

    let budget = TokenBudget::new(300, 0);
    let policy = CompactionPolicy {
        pinned_recent_turns: 2,
        ..Default::default()
    };
    let manager = BudgetManager::new(budget, policy);
    let est = estimator();

    let records = manager.ensure_fits(&mut msgs, &est).unwrap();
    assert!(
        !records.is_empty(),
        "pre-condition: compaction must have run"
    );

    session.record_compaction(records.clone());
    session.save(&path).expect("save");

    let loaded = Session::load(&path).expect("load");

    assert_eq!(
        loaded.compaction_log.len(),
        records.len(),
        "compaction_log length must survive round-trip"
    );
    assert_eq!(
        loaded.compaction_log, records,
        "compaction_log contents must survive round-trip"
    );
}

// ── Test 5 ────────────────────────────────────────────────────────────────────

/// A Session serialized without `compaction_log` deserializes with an empty vec.
#[test]
fn session_backward_compat_no_compaction_log() {
    // Minimal valid Session JSON that omits the compaction_log field entirely.
    let json = r#"{
        "meta": {
            "id": "ses_old_format",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "active_model": null,
            "active_skill": null,
            "working_dir": "/tmp"
        },
        "transcript": [],
        "tool_audit": []
    }"#;

    let session: Session = serde_json::from_str(json).expect("deserialize old-format session");
    assert!(
        session.compaction_log.is_empty(),
        "compaction_log must default to empty vec when absent from JSON"
    );
}

// ── Test 6 ────────────────────────────────────────────────────────────────────

/// Event::Compaction serializes and deserializes without loss.
#[test]
fn compaction_event_serde_roundtrip() {
    let record = CompactionRecord {
        step: CompactionStep::TrimToolOutputs,
        tokens_before: 1500,
        tokens_after: 600,
        messages_removed: 0,
        messages_added: 0,
    };
    let event = Event::Compaction(record.clone());

    let json = serde_json::to_string(&event).expect("serialize");
    let back: Event = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(
        back, event,
        "Event::Compaction must survive serde round-trip"
    );

    // Also verify the inner record is intact.
    if let Event::Compaction(r) = back {
        assert_eq!(r.step, CompactionStep::TrimToolOutputs);
        assert_eq!(r.tokens_before, 1500);
        assert_eq!(r.tokens_after, 600);
        assert_eq!(r.messages_removed, 0);
        assert_eq!(r.messages_added, 0);
    } else {
        panic!("expected Event::Compaction");
    }
}
