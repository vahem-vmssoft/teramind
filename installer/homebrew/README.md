# Homebrew tap (scaffold)

This directory contains the Homebrew formula for Teramind. Publication to a
real tap (`https://github.com/teramind-org/homebrew-tap`) is gated on the
first stable release; until then, this is a reference.

## Updating after each release

1. Bump `version` in `teramind.rb`.
2. Replace each `REPLACE_WITH_RELEASE_SUM` with the macOS arm64 / x86_64
   SHA-256 from `teramind-<version>-SHA256SUMS`.
3. Commit & push the tap repo. Users get the new version on `brew upgrade`.

A future CI job will automate steps 1–3 by opening a PR against the tap repo.
