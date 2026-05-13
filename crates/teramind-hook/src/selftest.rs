use crate::{hook_input::HookInput, translate};

/// Self-test the hook: parse a canned SessionStart payload, verify translation produces an envelope.
/// Returns Ok(()) on success; prints diagnostic to stderr on failure.
pub fn run() -> Result<(), String> {
    let raw = r#"{"hook_event_name":"SessionStart","session_id":"selftest","cwd":"/tmp","source":"startup"}"#;
    let parsed: HookInput = serde_json::from_str(raw)
        .map_err(|e| format!("parse failed: {e}"))?;
    let env = translate::translate(parsed)
        .ok_or_else(|| "translate returned None".to_string())?;
    println!("teramind-hook selftest OK");
    println!("  envelope.client_event_id: {}", env.client_event_id);
    println!("  envelope.ts: {}", env.ts);
    println!("  envelope.event: {:?}", env.event);
    Ok(())
}
