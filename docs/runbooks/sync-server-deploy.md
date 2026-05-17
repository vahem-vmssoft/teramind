# Teramind Sync Server — Deployment Runbook

## Scope

How to deploy `teramind-sync-server` for a team. Covers the **single-binary**
mode (recommended for self-hosting) and the **docker-compose** mode (faster
to evaluate). Targets v1.0 — see the spec at `docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md`
for the architectural picture.

## Prereqs

- Postgres 14+ with `pgcrypto` and `pg_trgm` extensions available. (pgvector
  becomes a hard requirement in Plan K when semantic search routes via the
  server; v1.0 of the sync server itself does not require pgvector yet.)
- A TLS certificate + key for the public hostname. Let's Encrypt via a
  reverse proxy is the typical path; the server can also terminate TLS
  directly via `[tls] cert_file = …, key_file = …`.
- A clock-synced host (`chronyd` / `timedatectl`). DPoP claims include `iat`;
  more than ±60 s skew between client and server rejects every request.

## Single-binary install

1. Build:
   ```bash
   cargo build --release -p teramind-sync-server
   ```

2. Copy `target/release/teramind-sync-server` to your server (eg. `/usr/local/bin/`).

3. Create the config:
   ```toml
   # /etc/teramind-sync-server/config.toml
   listen_addr  = "0.0.0.0:443"
   database_url = "postgres://teramind:REDACTED@127.0.0.1:5432/teramind"

   [tls]
   cert_file = "/etc/teramind/cert.pem"
   key_file  = "/etc/teramind/key.pem"
   ```

4. Run migrations:
   ```bash
   teramind-sync-server migrate
   ```

5. Start the service. A minimal systemd unit:

   ```ini
   [Unit]
   Description=Teramind Sync Server
   After=network-online.target postgresql.service
   Requires=postgresql.service

   [Service]
   ExecStart=/usr/local/bin/teramind-sync-server serve
   Environment=TERAMIND_SYNC_CONFIG=/etc/teramind-sync-server/config.toml
   User=teramind
   Restart=on-failure
   AmbientCapabilities=CAP_NET_BIND_SERVICE
   CapabilityBoundingSet=CAP_NET_BIND_SERVICE

   [Install]
   WantedBy=multi-user.target
   ```

6. Verify:
   ```bash
   curl -sk https://teramind.acme.dev/v1/health
   # {"status":"ok","db":"ok"}
   ```

## Docker Compose install (dev / evaluation)

```bash
cd docker/sync-server
docker compose up --build
# wait ~30s for build + migrations
curl http://localhost:8443/v1/health
```

The compose file uses `--insecure-allow-http` (no TLS). Do not use for
production — a real deployment terminates TLS either in the binary
(`[tls]`) or in a reverse proxy.

## Day-2: issuing invites

```bash
teramind-sync-server invite create \
    --email alice@acme.dev --name "Alice K." --created-by admin@acme.dev
# invite created:
#   code:    TM-…
#   email:   alice@acme.dev
#   expires: 2026-05-24T16:00:00Z
```

Hand the code to the developer; it's one-shot and expires in 7 days by
default. They run:

```bash
teramind init --team \
    --server https://teramind.acme.dev \
    --invite TM-…
```

(Plan J implements the `--team` flag end-to-end.)

## Day-2: revoking

- Lost laptop: `teramind-sync-server member revoke-device <device-id>`.
- Offboarding: `teramind-sync-server member revoke-user <user-id>`. (Devices
  remain rows but every auth lookup fails.)
- Unused invite that hasn't been redeemed: `teramind-sync-server invite revoke <invite-id>`.

## Observability

- The server emits JSON-formatted logs to stdout; pipe to your aggregator.
- `GET /v1/health` for liveness, `GET /v1/version` for build-id.
- Per-request tracing emits a `request_id`, the matched route, and the
  `(user_id, device_id)` if auth succeeded. Failed auth requests log the
  failure reason but not the bearer token.

## Backup

The DB is the source of truth. Standard Postgres backup applies. The server
itself is stateless beyond the in-memory DPoP replay cache, which is
ephemeral (a 60s replay window).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Every request 403 `invalid_proof` after a deploy | Wall clock skew | Run `timedatectl status`; fix NTP. |
| `teramind init --team` fails 410 | Invite expired | Re-issue. |
| `teramind init --team` fails 409 | Invite already redeemed | Re-issue (each device needs its own). |
| Server refuses to start: `TLS not configured` | No `[tls]` in config | Either configure TLS or pass `--insecure-allow-http`. |
