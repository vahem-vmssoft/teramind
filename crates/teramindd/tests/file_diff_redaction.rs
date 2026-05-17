use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::file_diff::Attribution;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[test]
fn redactor_strips_aws_keys_from_file_diff_excerpts() {
    let r = Redactor::with_default_rules();
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::FileDiff {
            session_id: SessionId::new(),
            turn_id: None,
            file_path: "/p/a.rs".into(),
            rel_path: "a.rs".into(),
            attribution: Attribution::Human,
            language: Some("rust".into()),
            pre_excerpt: "let key = \"AKIAIOSFODNN7EXAMPLE\";".into(),
            post_excerpt: "let key = \"AKIAIOSFODNN7EXAMPLE\";".into(),
            unified_diff: " let key = \"AKIAIOSFODNN7EXAMPLE\";\n".into(),
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: 32,
        },
    };
    // We exercise the redactor on the strings directly to lock in
    // the expectation; the daemon's ingest layer wires it in.
    assert!(!r
        .apply(match &env.event {
            IngestEvent::FileDiff { pre_excerpt, .. } => pre_excerpt,
            _ => unreachable!(),
        })
        .contains("AKIAIOSFODNN7EXAMPLE"));
}
