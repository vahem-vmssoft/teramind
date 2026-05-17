use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramindd::services::write_tool_ring::WriteToolRing;
use time::OffsetDateTime;

// Exercise the ring directly to lock in the write-tool naming contract.
#[tokio::test]
async fn ring_only_records_write_tool_completions() {
    let ring = WriteToolRing::new(8, time::Duration::seconds(5));
    let sid = SessionId::new();
    let tid = TurnId::new();

    // Push as the ingest layer would.
    if teramindd::services::write_tool_ring::is_write_tool("Edit") {
        ring.push(teramindd::services::write_tool_ring::WriteCompletion {
            session_id: sid,
            turn_id: tid,
            tool_name: "Edit".into(),
            at: OffsetDateTime::now_utc(),
        })
        .await;
    }
    // Non-write tool: do NOT push.
    if teramindd::services::write_tool_ring::is_write_tool("Read") {
        unreachable!("Read should not be a write tool");
    }
    assert!(ring
        .most_recent_for(sid, OffsetDateTime::now_utc())
        .await
        .is_some());

    // Construct an envelope just to make sure the type compiles end-to-end:
    let _env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::ToolCallEnd {
            tool_call_id: ToolCallId::new(),
            output: "ok".into(),
            is_error: false,
            duration_ms: 0,
            session_id: Some(sid),
            turn_id: Some(tid),
            tool_name: Some("Edit".into()),
        },
    };
}
