# Developer Testing Checklist

Ordered by dependency — each section builds on the one before it.
Check off items as you go. Bugs referenced here are documented in `SMOKE_TEST.md`.

---

## Phase 1 — Local daemon (solo mode)

### 1.1 Build & install

- [x] `cargo build --workspace --release`
- [x] Binaries symlinked to `~/.local/bin/` (teramind, teramindd, teramind-hook, teramind-mcp)
- [x] All four binaries on `$PATH` (`which teramind teramind-hook teramind-mcp teramindd`)

### 1.2 Daemon lifecycle

- [x] `teramind start` — embedded Postgres starts, migrations run, socket appears at `/tmp/teramind.sock`
- [x] `teramind status` — shows uptime > 0, `pg connected: true`
- [x] `teramind doctor` — all subsystems healthy
- [x] `teramind stop` then `teramind start` — clean restart, no orphan PG processes
  > Fixed shutdown ordering in `app.rs` (socket → PG shutdown → pid file removal) and `restart.rs` (polls pid-file absence instead of socket ping). `teramind stop` is reliable; the B6 note referred to the Claude Code session-end hook, not this command.
- [x] Config reload: edit `~/.config/teramind/summarize.toml`, restart, confirm `teramind status --format=json` reflects new model

### 1.3 Claude plugin

- [x] Plugin installed and working
- [x] MCP tools verified (`mcp__teramind__recall`, `mcp__teramind__search`)
- [x] `claude plugin details teramind` — shows exactly 6 hooks + 1 MCP server
- [x] `teramind-hook --selftest` — prints `selftest OK`

### 1.4 Session capture

- [ ] Open Claude in a project dir, send ≥3 prompts, use a tool, exit
- [ ] `teramind sessions show` — summary appears (non-empty) within 30s of exit
  > Currently 23/25 summaries are empty (bug B3). Verify Ollama is producing output: `curl http://localhost:11434/api/chat -d '{"model":"qwen2.5:3b","messages":[{"role":"user","content":"say hi"}],"stream":false}'`
- [ ] Raw events present: `jq -r '.event.type' ~/.local/share/teramind/raw/$(date +%F).jsonl | sort | uniq -c`
  - Expected event types: `session_start`, `user_prompt`, `tool_call_start`, `tool_call_end`, `session_end`
- [ ] `teramind search "<word you typed>" --grep` — returns the turn
- [ ] `teramind search "<word you typed>"` (FTS+semantic) — returns hit
  > Note: first query takes ~47s with fastembed (bug B7). Subsequent queries same speed. Use `--grep` for immediate confirmation.
- [ ] Dead letters drain: check `~/.local/share/teramind/dead_letter/` is empty after restart (currently 22 stranded — bug B5)

### 1.5 FS watcher & diff attribution

- [ ] Inside Claude: ask it to edit a file → confirm `file_diffs` row with `attribution = 'agent'`
- [ ] Outside Claude, manually edit a file in the watched dir → `attribution = 'human'`
  ```sh
  PGPASSWORD=teramind ~/.theseus/postgresql/16.13.0/bin/psql \
    -h /tmp -p 54817 -U postgres -d teramind \
    -c "SELECT rel_path, attribution, length(unified_diff) FROM file_diffs ORDER BY captured_at DESC LIMIT 5;"
  ```
- [ ] `teramind status --format=json` → `fs_watcher_gaps_total: 0` after a normal session

### 1.6 Auto-recall

- [ ] After at least one captured session in `$PWD`: open a new Claude session in the same directory
- [ ] Confirm the auto-recall digest appears in Claude's initial output ("Recent context for this project…")

### 1.7 Redaction

- [ ] `teramind redact test "sk-ant-api03-abc123"` — currently **not redacted** (bug B2, no pattern for bare Anthropic keys)
- [ ] `teramind redact test "ANTHROPIC_API_KEY=sk-ant-abc"` — correctly redacted as `«redacted:env_secret»`
- [ ] Check a real captured session doesn't contain bare `sk-ant-*` tokens:
  ```sh
  jq -r '.event | .user_prompt? // .output? // ""' \
    ~/.local/share/teramind/raw/$(date +%F).jsonl | grep 'sk-ant-'
  ```

### 1.8 Skills & codifier

- [ ] `teramind skills observations` — shows rows after sessions are captured
- [ ] `teramind skills observations --min-freq=2` — currently returns all rows regardless (bug B1, filter discarded)
- [ ] Create `~/.config/teramind/codify.toml` with provider + model; restart daemon; run several sessions; check `teramind skills list` for codified candidates
- [ ] `teramind skills show <name>` — prints skill body

---

## Phase 2 — Team mode (sync server, local evaluation)

Use the Docker Compose path first — faster to stand up, no TLS required.

### 2.1 Sync server (Docker Compose)

```sh
cd docker/sync-server
docker compose up --build    # ~30s first run
curl http://localhost:8443/v1/health
# expect: {"status":"ok","db":"ok"}
```

- [ ] Health check returns `{"status":"ok","db":"ok"}`
- [ ] `docker compose logs sync-server` — no ERROR lines

### 2.2 Admin setup

```sh
# Generate password hash (interactive prompt)
teramind-sync-server admin-password
# Copy the [admin] block it prints into docker/sync-server/config.toml, then restart:
docker compose restart sync-server
```

- [ ] `curl -s -c jar.txt -X POST http://localhost:8443/admin/login -H 'Content-Type: application/json' -d '{"password":"<yourpw>"}' | jq .` — returns `{"ok":true}`
- [ ] `curl -sb jar.txt http://localhost:8443/admin/health | jq .` — returns health JSON

### 2.3 Dashboard

The dashboard is embedded in `teramind-sync-server`. Enable it by adding to `config.toml`:
```toml
[admin]
admin_password_hash   = "..."   # from admin-password command
admin_session_secret  = "..."   # any 32+ char random string
dashboard_enabled     = true    # required — off by default
```

- [ ] Open `http://localhost:8443/` in a browser — admin login page loads
- [ ] Log in → members list, skills, observations, quality panels all render
- [ ] Verify security headers present: `curl -sI http://localhost:8443/ | grep -E "X-Frame|Content-Security|X-Content"`

### 2.4 Invite flow & developer onboarding

```sh
# On the server machine
teramind-sync-server --config docker/sync-server/config.toml \
    invite create --email dev@teracloud.com --name "Dev Name"
# prints: code: TM-XXXX...
```

- [ ] Invite code generated successfully
- [ ] On a developer machine: `teramind init --team --server http://localhost:8443 --invite TM-XXXX`
- [ ] `teramind status --format=json` — shows team config present
- [ ] Run a Claude session; confirm events arrive at the sync server (check `docker compose logs`)
- [ ] Second developer redeems a separate invite; confirm both appear in dashboard Members tab

### 2.5 Team share toggle

- [ ] In a project dir: `teramind team share-set --enable` — creates `.teramind/team-share.toml`
- [ ] Captured events from this project sync to server
- [ ] In a different project: leave share disabled — events stay local only

### 2.6 Live feed

- [ ] `teramind feed --follow` — connects, prints team activity
- [ ] While another developer's session is active, confirm events stream in real time

### 2.7 Revocation

- [ ] `teramind-sync-server member list` — shows all devices
- [ ] Simulate lost device: `teramind-sync-server member revoke-device <id>` — subsequent `teramind status` from that device shows auth failure

---

## Phase 3 — Production deployment (NVidia DPX, data center)

The sync server is the only component that runs in the data center. Each developer's daemon runs locally on their own machine. The DPX needs: system Postgres, the sync server binary, TLS, and a systemd unit.

### 3.1 Provision the DPX host

- [ ] OS: Ubuntu 22.04 LTS (or 24.04) — confirm with `lsb_release -a`
- [ ] Clock sync: `timedatectl status` → `NTP service: active`, offset <1s
  > Required — DPoP proofs are timestamp-bound (±60s window). Clock skew breaks all client auth.
- [ ] Create service account: `sudo useradd -r -s /sbin/nologin teramind`

### 3.2 Postgres on DPX

The sync server uses **system Postgres** (not embedded). The embedded PG is local-daemon-only.

```sh
sudo apt install -y postgresql-16 postgresql-contrib
sudo -u postgres createuser teramind
sudo -u postgres createdb -O teramind teramind
sudo -u postgres psql -c "ALTER USER teramind PASSWORD 'CHANGE_ME';"
```

- [ ] `psql -U teramind -d teramind -c '\conninfo'` — connects cleanly
- [ ] Extensions: `sudo -u postgres psql -d teramind -c "CREATE EXTENSION IF NOT EXISTS pg_trgm; CREATE EXTENSION IF NOT EXISTS pgcrypto;"` — no errors

### 3.3 Build the sync server binary for DPX

The DPX is x86_64 Linux; build on any x86_64 Linux machine (or cross-compile).

```sh
cargo build --release -p teramind-sync-server
# Binary: target/release/teramind-sync-server (~37 MB)
```

- [ ] Copy binary to DPX: `scp target/release/teramind-sync-server dpx:/usr/local/bin/`
- [ ] `ssh dpx teramind-sync-server version` — prints version string
- [ ] `sudo chown root:root /usr/local/bin/teramind-sync-server && sudo chmod 755 /usr/local/bin/teramind-sync-server`

### 3.4 TLS certificate

- [ ] Decide on hostname: e.g. `teramind.teracloud.internal` — add to internal DNS
- [ ] Obtain cert (options):
  - **Internal CA:** issue cert from your existing CA, place PEM at `/etc/teramind/cert.pem` and `/etc/teramind/key.pem`
  - **Let's Encrypt (if public DNS):** `certbot certonly --standalone -d teramind.teracloud.internal`
  - **Self-signed (dev only):** `openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=teramind.teracloud.internal"`
- [ ] `sudo mkdir -p /etc/teramind && sudo chown teramind: /etc/teramind`
- [ ] Cert and key placed, permissions: `chmod 640 /etc/teramind/key.pem && chown teramind: /etc/teramind/key.pem`

### 3.5 Server config

```sh
sudo mkdir -p /etc/teramind-sync-server
```

`/etc/teramind-sync-server/config.toml`:
```toml
listen_addr  = "0.0.0.0:443"
database_url = "postgres://teramind:CHANGE_ME@127.0.0.1:5432/teramind"

[tls]
cert_file = "/etc/teramind/cert.pem"
key_file  = "/etc/teramind/key.pem"

[admin]
admin_password_hash  = ""   # fill in from: teramind-sync-server admin-password
admin_session_secret = ""   # 32+ chars random: openssl rand -hex 32
dashboard_enabled    = true
```

- [ ] Generate password hash: `teramind-sync-server admin-password` → copy hash into config
- [ ] Generate session secret: `openssl rand -hex 32` → copy into config
- [ ] Config owned by service user: `sudo chown teramind: /etc/teramind-sync-server/config.toml && sudo chmod 600 /etc/teramind-sync-server/config.toml`

### 3.6 Migrations

```sh
sudo -u teramind TERAMIND_SYNC_CONFIG=/etc/teramind-sync-server/config.toml \
    teramind-sync-server migrate
# expect: migrations OK
```

- [ ] Migrations complete cleanly
- [ ] `psql -U teramind -d teramind -c '\dt'` — shows expected tables (sessions, turns, skills, …)

### 3.7 Systemd unit

`/etc/systemd/system/teramind-sync-server.service`:
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
RestartSec=5s
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now teramind-sync-server
```

- [ ] `sudo systemctl status teramind-sync-server` — `active (running)`
- [ ] `sudo journalctl -u teramind-sync-server -n 50` — no ERROR lines
- [ ] `curl -sk https://teramind.teracloud.internal/v1/health` — `{"status":"ok","db":"ok"}`

### 3.8 Firewall

- [ ] Port 443 open to the developer network (DPX firewall or corporate switch ACL)
- [ ] Port 5432 (Postgres) NOT exposed externally — localhost only
- [ ] Verify from a developer workstation: `curl -sk https://teramind.teracloud.internal/v1/health`

### 3.9 Postgres backup

The sync server DB is the source of truth for all team captures.

- [ ] Configure `pg_dump` cron or WAL archiving:
  ```sh
  # Example: daily dump to /var/backups/teramind/
  0 3 * * * pg_dump -U teramind teramind | gzip > /var/backups/teramind/$(date +\%F).sql.gz
  ```
- [ ] Test restore: `gunzip -c <backup>.sql.gz | psql -U teramind teramind` into a scratch DB
- [ ] Retention policy defined (suggest: 30 days daily, 12 months monthly)

### 3.10 Onboard each developer

Repeat per developer. Each person needs their own invite code.

```sh
# On the DPX (admin)
teramind-sync-server --config /etc/teramind-sync-server/config.toml \
    invite create --email dev@teracloud.com --name "Dev Name" --created-by admin
# gives a one-time code: TM-XXXX
```

On the developer's machine:
```sh
# Build and install binaries first (or distribute a release archive)
teramind init --team \
    --server https://teramind.teracloud.internal \
    --invite TM-XXXX
teramind start
```

- [ ] Developer: `teramind status --format=json` — shows team fields populated
- [ ] Admin: dashboard Members tab shows new device

### 3.11 Production smoke test

After onboarding ≥2 developers:

- [ ] Both appear in dashboard Members tab
- [ ] `teramind feed --follow` on one machine shows events from the other (in a team-shared project)
- [ ] Dashboard Skills tab shows observations accumulating
- [ ] `GET /v1/health` returns `ok` from all developer workstations (confirms TLS + DNS)
- [ ] `sudo journalctl -u teramind-sync-server --since "1 hour ago" | grep ERROR` — empty

---

## Known bugs to fix before inviting more people

These are in `SMOKE_TEST.md` with root causes:

| Bug | Impact | File |
|---|---|---|
| B1: `--min-freq` filter silently ignored | Codifier observations filter is a no-op | `rpc_dispatch.rs:270` |
| B2: No `sk-ant-*` redaction pattern | Bare Anthropic keys stored unredacted | `redact/patterns.rs` |
| B3: Ollama summarizer writes empty wiki_pages | 92% of summaries are blank | `summarizer_worker.rs:122` |
| B5: Dead-letter files not drained on restart | Events lost across daemon gaps | ingest drain logic |
| B6: 25% of sessions missing `ended_at` | Those sessions never get summarized | stop hook reliability |
| B7: Search takes ~47s per query | Unusable for interactive recall | embedding on query path |
