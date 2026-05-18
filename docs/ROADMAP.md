# Teramind roadmap

Single source of truth for post-v1.0 work. Each item links back to the spec that defined it; specs continue to own their own Non-goals sections but planning should happen here.

> **Versioning conventions.** v1.0.x = bugfix-only patches. v1.1 = next feature wave (mostly completing trait-shaped extensibility points and rounding out auth/UX gaps). v1.2 = larger surfaces that depend on v1.1 telemetry (weighting loops, multi-tenancy). v2+ = work that requires substantial new architecture or hosted-product investment.

---

## Status

**v1.0 shipped.** Plans A–N are merged into `main`. Capabilities live in [docs/PR-FAQ.md](PR-FAQ.md); per-plan specs live under [docs/superpowers/specs/](superpowers/specs/).

---

## v1.0.1 — patch wave

- **L5 paraphrase corpus from real users.** Original corpus is hand-written. Backfill from anonymised real queries against `mcp__teramind__search` once enough data exists. — [pgvector spec](superpowers/specs/2026-05-16-teramind-pgvector-design.md), [core spec](superpowers/specs/2026-05-13-teramind-core-design.md)

---

## v1.1 — next feature wave

### Cloud egress (gated by `network_egress = true`)

- **Cloud embedding providers** — wire HTTPS to Anthropic / OpenAI / Voyage. v1.0 ships the `CloudProvider` trait shape + config validation only. — [pgvector spec §2.2](superpowers/specs/2026-05-16-teramind-pgvector-design.md)
- **OAuth / SSO / OIDC invite redemption.** v1.0 invite redemption is invite-code only. — [team-sync spec §2.2](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **OpenAI summarizer provider** (trait shape exists; HTTPS wiring deferred). — [session-summarizer spec §2.1](superpowers/specs/2026-05-16-teramind-session-summarizer-design.md)

### Skill codifier UX gap-fill

- **Filesystem materialization** of approved skills to `~/.claude/skills/<name>/SKILL.md`. — [skill-codifier spec §2.2](superpowers/specs/2026-05-17-teramind-skill-codifier-design.md)
- **Interactive review CLI**: `teramind skills review`. — same spec
- **MCP review tools**: `mcp__teramind__list_candidates` + `approve_candidate`. — same spec
- *(Dashboard already covers the GUI review surface — that landed in Plan N.)*

### Embeddings — round-out

- **Input chunking** when content exceeds `provider.max_tokens()`. v1.0 truncates. — [pgvector spec §2.2](superpowers/specs/2026-05-16-teramind-pgvector-design.md)
- **Per-project model overrides** (different model per `project_id`). — same spec
- **Wiki page embeddings** — semantic search over summaries, not just turns/diffs. — [session-summarizer spec §2.2](superpowers/specs/2026-05-16-teramind-session-summarizer-design.md)

### Team mode hardening

- **Hardware-backed signing keys** — Secure Enclave on macOS, TPM on Linux/Windows. — [team-sync spec §2.2](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **`teramind forget`** — server-side hard-delete endpoint + GDPR-shaped flow. Also covers the codifier's "hard delete of skills tied to a removed user" deferral. — same spec; [skill-codifier spec §2.2](superpowers/specs/2026-05-17-teramind-skill-codifier-design.md)
- **Sliding-token rotation** for theft detection — gate on whether DPoP-only behavior shows weakness in practice. — [team-sync spec §2.2](superpowers/specs/2026-05-17-teramind-team-sync-design.md)

### Dashboard auth depth

- **SSO / OAuth admin login.** v1 is password-only. — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)
- **Two-factor** on the password path. — same spec

### Installers

- **Windows code-signing.** v1 ships unsigned Windows artifacts. — [installer plan §7.7](superpowers/plans/2026-05-14-teramind-installers.md)

---

## v1.2 — second wave (depends on v1.1 data)

### Loops and signals

- **Detector weighting / feedback loop.** Downweight `problem_fix` signatures whose previous candidates were rejected, on re-emit. — [skill-codifier spec §2.2](superpowers/specs/2026-05-17-teramind-skill-codifier-design.md)
- **Cross-skill ranking.** Prefer skills the agent actually retrieves via `mcp__teramind__search`. — same spec

### Multi-tenancy

- **Multi-team-per-server** sync server. Schema is already forward-compatible. — [team-sync spec §11.4](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **Dashboard scoping** to those tenants. — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)

### Dashboard depth

- **Audit log view** of who approved which candidate (`reviewer` column already captured). — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)
- **Ingest-rate / error-budget / SLO** view. — same spec

### Summarizer UX

- **Wiki page editing** via `$EDITOR`. — [session-summarizer spec §2.2](superpowers/specs/2026-05-16-teramind-session-summarizer-design.md)

---

## v2+ — long-horizon

### Retrieval

- **Hybrid retrieval re-ranking** via cross-encoder over top-K. — [pgvector spec §2.2](superpowers/specs/2026-05-16-teramind-pgvector-design.md)

### Skill lifecycle

- **Automated promotion** for high-confidence candidates (skip the admin gate). — [skill-codifier spec §2.2](superpowers/specs/2026-05-17-teramind-skill-codifier-design.md)
- **Skill versioning + rollback.** — same spec

### Team

- **Federation across sync servers.** — [team-sync spec §2.2](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **End-to-end encryption** between local daemon and server. Currently the server reads cleartext; users trust the server like a self-hosted GitLab. — same spec
- **Read-only viewer role** for non-admin team members in the dashboard. — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)

### Distribution

- **Hosted SaaS offering** alongside the self-hosted path. — [team-sync spec §11.4](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **Embeddable widgets** — iframe of "latest codified skills" for the team wiki. — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)

---

## Out of scope (not planned)

These are explicit non-goals across the specs. Listed here so they don't reappear as drive-by feature requests:

- **Real-time co-debugging.** Live cursors, shared sessions. — [team-sync spec §2.2](superpowers/specs/2026-05-17-teramind-team-sync-design.md)
- **Web "send a chat to the agent" surface.** Dashboard is observational. — [web-dashboard spec §2.2](superpowers/specs/2026-05-17-teramind-web-dashboard-design.md)
- **Mobile-native client.** — same spec
- **Multi-language UI.** English only. — same spec
- **Editing a skill body after promotion.** Skills are append-only; supersede with a new skill instead. — same spec
- **Embedding model fine-tuning.** Not Teramind's job. — [pgvector spec §8.5](superpowers/specs/2026-05-16-teramind-pgvector-design.md)
- **Map-reduce chunking for very long sessions** in the summarizer. The digest's char budget already caps input. — [session-summarizer spec §2.2](superpowers/specs/2026-05-16-teramind-session-summarizer-design.md)
- **Streaming summarizer output.** v1.0 is one-shot per session and stays that way. — same spec
- **LLM-generated structured tags / decisions tables** as a separate output. Markdown body is the contract. — same spec
- **Cross-session "project digest"** that summarizes the wiki pages themselves. Would be its own spec, not a backlog item.
- **Auto-registration as an OS service** (launchd / systemd / Windows Service) bundled into `teramind init`. — [core spec §2.2](superpowers/specs/2026-05-13-teramind-core-design.md)

---

## How to use this document

- **Adding work:** open a spec under `docs/superpowers/specs/`, write the design, then add a roadmap entry here pointing at it. Don't move work in/out of versions without updating both this file and the source spec.
- **Picking up work:** v1.1 items are roughly ordered by how much they unblock real users. Cloud embedding providers and skill-codifier review CLI are the most-requested gaps; team-mode hardening matters for self-hosters; dashboard auth depth matters once we have non-trivial deployments.
- **Promoting items between versions:** OK to do as data arrives — that's the whole point of dating the spec entries. Re-grade items here and leave a one-line note next to them in the source spec's deferral list.
