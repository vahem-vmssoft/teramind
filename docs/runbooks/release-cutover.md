# Release cutover runbook

How to cut a new Teramind release.

## Prereqs

- You have push access to the repo.
- The `get.teramind.dev` static host (S3 / GCS bucket) is configured with the
  `/<version>/` layout the installer scripts expect.
- GitHub Actions has secrets set: `APPLE_DEVELOPER_ID_P12`,
  `APPLE_DEVELOPER_ID_P12_PASSWORD`, `APPLE_ID`, `APPLE_ID_APP_PASSWORD`,
  `APPLE_TEAM_ID`. Vars: `APPLE_NOTARIZE_ENABLED=true`, `COSIGN_ENABLED=true`,
  `APPLE_DEVELOPER_ID_NAME=Developer ID Application: <Org> (<TEAMID>)`.

## Steps

1. Bump `version` in the workspace `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. Commit: `chore(release): vX.Y.Z`.
4. Tag: `git tag -s vX.Y.Z -m "vX.Y.Z"`.
5. Push: `git push origin main vX.Y.Z`.
6. Watch `.github/workflows/release.yml`:
   - 6 build jobs (one per target triple)
   - 1 sums job (aggregates SHA256SUMS + cosign signature)
   - 2 notarize jobs (macOS arm64 + x86_64) — optional, controlled by vars
   - 1 release job (publishes GH release)
7. Once the GH release is up, copy all artifacts to `get.teramind.dev`:
   ```
   aws s3 sync ./out/ s3://get.teramind.dev/vX.Y.Z/
   ```
8. Update `s3://get.teramind.dev/releases.json` with the new `latest` and
   sha256s. Use `installer/release-index.example.json` as a template; the
   sha256s come from the `teramind-vX.Y.Z-SHA256SUMS` file in the release.
9. Smoke-test the installer:
   ```
   sh installer/install.sh  # picks up the new latest
   teramind --version  # should print X.Y.Z
   teramind self-update --check-only  # should report up-to-date
   ```
10. Bump the Homebrew formula in `installer/homebrew/teramind.rb` and
    open a PR against the tap repo (manual for now).

## Rollback

- If the new release has a bug, revert `s3://get.teramind.dev/releases.json`
  to the previous version (keep the `0.X.Y/` artifacts in S3 indefinitely).
- Push a patched build as `vX.Y.Z+1`.
