use std::io::Read;
use teramind_hook::hook_input::HookInput;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Read all of stdin into a buffer.
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        // Stdin unavailable: silently exit 0. Claude must not see an error.
        std::process::exit(0);
    }
    let parsed: HookInput = match serde_json::from_str(&buf) {
        Ok(p) => p,
        Err(_) => {
            // Malformed hook input: log to stderr (Claude won't see this if hook is properly redirected)
            // and exit 0. Capture is best-effort.
            std::process::exit(0);
        }
    };
    // Dispatch lands in Section 4. For now, just drop the parsed value and exit.
    let _ = parsed;
    std::process::exit(0);
}
