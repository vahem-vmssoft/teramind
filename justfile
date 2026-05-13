default: fmt clippy test

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

build:
    cargo build --workspace

test:
    cargo test --workspace

# Run integration tests (slow — they start embedded Postgres).
test-integration:
    cargo test --workspace --test '*'

# Wipe local Teramind state for the current user (Unix).
reset-local:
    rm -rf "$HOME/.local/share/teramind" "$HOME/.config/teramind"
