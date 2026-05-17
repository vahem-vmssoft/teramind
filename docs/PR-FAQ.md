# Teramind — PR/FAQ

*Internal document. Working backwards from launch. Subject to change before GA.*

---

## Press Release

### **Teramind: Stop reinventing the wheel — turn every AI coding session into searchable team knowledge.**

**Local-first by default. Opt-in team sync. Installs in 60 seconds.**

**SAN FRANCISCO — November 4, 2026 —** Today Teramind announces general availability of a knowledge layer for teams that use AI coding agents. Teramind quietly captures everything happening in those sessions — the prompts, the agent's responses, the tool calls, the files it edits — and makes all of it searchable later. The next time the same developer hits a familiar problem, the agent itself can pull up exactly how the team solved it last time. Teramind installs in about a minute. It runs entirely on the developer's machine by default. Teams that want to share knowledge across machines flip a single switch.

For the last two years, AI coding agents have made individual developers extraordinarily productive. They have also created a new and surprisingly expensive problem: **every solved bug, every clever refactor, every hard-won porting trick lives for exactly one session and then evaporates**. A team of six engineers, each running their AI agent daily, re-derives the same answers to the same problems thousands of times per year. The institutional knowledge that used to live in pull-request threads, chat channels, and the heads of senior engineers now lives in conversation transcripts that nobody reads twice.

Teramind closes that loop. Sessions get captured the moment they start. The agent can search prior work mid-conversation, find a session that solved a related problem, and adapt the approach without the developer ever having to remember "I wrote a patch like this three months ago." When a session ends, Teramind automatically writes a short summary of what changed and what was decided, and surfaces those summaries the next time a developer starts work in the same project. Repeated patterns — the build-test-commit dance, the same fix for the same recurring error — get noticed automatically and turned into reusable recipes the agent loads on its own.

> "I ported OpenSSL to a niche legacy platform last quarter. Three months later when I started porting rsync, the agent pulled up the exact patches I'd written, with the surrounding conversation, and asked if it should adapt them for the new project. That used to be a half-day of digging through old branches. Now it surfaces instantly." — *Sergey K., Staff Engineer, fictional porting team*

Teramind ships with two install shapes that share the same product surface. The **local-first install** keeps every byte of data on the developer's own machine — nothing leaves the device unless the developer explicitly opts into a cloud model for richer semantic search or session summaries. The **team install** points everyone at a shared sync server the team self-hosts. Captures forward to the team store; search, recall, and the agent's view all flip to the team's combined knowledge. Privacy is granular and per-project: when a developer starts work in a new project, the agent itself asks once whether to share this project with the team, and remembers the answer for every future session. A live activity feed shows the team's work as it happens.

> "We built Teramind because the most valuable thing about working with an AI agent isn't the agent's output — it's the *context* the agent accumulates as it works. Throwing that context away every 8 hours is a tax we kept paying because no one had built the layer to keep it. Teramind is that layer, and it's local-first by default so it works inside the firewalls of teams who need it most." — *Vahe Momjyan, creator of Teramind*

**Getting started.** One install command, one init command, one connect command. Teramind is open source. Releases are signed and notarized. Semantic search and session summaries default to a model that runs locally on the developer's own hardware — no cloud account required to get the full experience.

---

## Customer FAQs

### What problem does Teramind solve?

AI coding sessions are episodic. Each conversation is isolated from every conversation before it. Engineers re-discover the same patches, re-debug the same edge cases, and re-explain the same project conventions to the agent every time. On teams, this multiplies — different engineers on different continents independently solve identical problems. Teramind keeps every session and makes prior work searchable, both for the human and — more importantly — for the agent itself, mid-session.

### How is this different from saving conversations to a file?

Three differences. **Structure**: Teramind separates what the user asked, what the agent replied, what tools it ran, what files it changed, and which project it was in. You can ask narrow questions like "all the times this project's tests touched the authentication module" rather than scrolling. **Semantic recall**: when the words don't match exactly, Teramind finds the conceptually related prior work anyway. **Agent integration**: the agent itself can query prior context without the user having to remember to do so. The recall happens silently inside the conversation, not as a separate workflow the human has to drive.

### Does my code or my conversations leave my machine?

No, by default. The full local-first experience runs on the developer's own hardware. Optional cloud models for richer search and summaries are wired in but explicitly off by default; a developer turns them on with a one-line config change. Before anything is sent to a cloud model — even one a user has opted into — Teramind strips API keys, certificates, and other common secrets from the payload.

### What does Teramind cost?

Free. Open source. The only running costs are local: some disk for the captured history (modest — a heavy user accumulates a few gigabytes a year), and whatever compute your local model uses. Optional cloud providers cost only what those providers charge against your own account.

### Can my team share knowledge across machines?

Yes. Team mode is shipped at launch alongside the local-first install. A team self-hosts a sync server; each developer connects with a one-time invite code. Captures flow to the team store; search and the agent's recall both flip to the team's combined knowledge. Privacy is per-project — the agent asks the user once whether to share a given project, and remembers the answer thereafter. A live activity feed shows what's happening across the team in real time. Solo developers don't see any of this unless they opt in; team mode is strictly opt-in.

### Which coding agents does Teramind support?

Claude Code at launch. The capture layer is designed to be agent-agnostic from day one, so adding support for other agents is additive — not a rewrite. Additional agents are next on the roadmap.

### How fast is it?

Capture is invisible — the agent never waits on Teramind. Search returns under a second on a corpus of thousands of sessions. Semantic queries against a local model are typically well under half a second. Session summaries appear within a minute or so of a session ending.

### What if I don't want to run a local model?

You don't have to. Teramind ships with a small embedded model that handles semantic search without needing any external service installed. Session summaries are optional; without a model configured for them, Teramind still captures, still searches, still surfaces prior work — it just skips the auto-generated summary at the end of each session.

### How do I see what's been captured?

Built-in commands print the most recent session's summary for the current project, list recent skills the team has accumulated, search the captured history, and show a single diagnostic report you can paste when asking for help. The team-mode activity feed shows live events as they happen.

### What if I want to forget a specific session, or wipe everything?

A single reset command deletes everything Teramind has captured locally. Uninstall removes the program, the configuration, and the data together. In team mode, a future release will add an authoritative "forget" workflow for removing specific data from the team store; until then, the team admin can delete data directly.

### Is the search quality measured?

Yes. Teramind ships with a public benchmark: a synthetic corpus of sessions, a curated set of queries, and a published quality score. Every release runs against this benchmark and surfaces regressions before they reach users. Adding the semantic-search layer measurably improved retrieval quality — by roughly four times on the published baseline.

---

## Internal FAQs

### Why a knowledge layer for AI coding sessions rather than a smarter agent?

Models keep getting better. Their *memory* doesn't. Every release of every coding agent we know about treats each session as starting from zero — no persistent memory of the developer's project, no awareness of what teammates have already figured out. We bet that the highest-leverage product is the substrate that gives agents memory, not yet another agent. The agent vendors will keep shipping smarter agents; we want them to land on a team that remembers what it already knows.

### Why local-first by default rather than cloud-first?

Two reasons. **Compliance**: a meaningful share of professional development happens behind firewalls, in regulated industries, on contracts with strict data-egress rules. A cloud-first product is dead on arrival for those teams. **Trust**: developers form a relationship with their tools over years. A tool that captures every keystroke needs to earn that trust before it asks anyone to send their data somewhere else. Local-first by default makes the easy path the trustworthy path; cloud is opt-in for users who want it.

### Why ship team mode at launch instead of after?

The single-machine version got built first to make sure the capture, search, and agent integration were right for one developer before adding the cross-machine complexity. Once that was solid, team mode landed as a clean opt-in layer on top — without changing anything for solo users. Shipping team mode at launch means a team can pick up Teramind without waiting for a follow-on release, and it means the team-knowledge story (which is the more compelling story to tell) is true from day one.

### What's the cost story if someone enables cloud providers?

The natural rate limit is small — one summary per finished session, one embedding per turn — so the worst case is bounded. Cloud providers expose their own spend caps. We surface running token totals in our diagnostic report, so anyone running with cloud models can see the cost trend at a glance. The default — running locally — has zero per-token cost.

### How is the data secured?

By default the data never leaves the developer's machine, so the surface area for compromise is the developer's own disk. The redaction layer strips secrets before any cloud egress (even when the user opted in). For team mode, every request between a developer's machine and the team server is signed with a key the device holds; a stolen access token alone is not enough — an attacker would need filesystem-level access to the device itself. Team admins can revoke a single developer's device, or an entire user, at any time. Independent security review is on the post-launch roadmap.

### What's on the roadmap after launch?

The next batch of work, in priority order:

1. **Skill codifier** — Teramind notices when the team keeps repeating the same recipe (the same build-test-commit dance, the same fix for the same recurring error) and turns those patterns into reusable shortcuts the agent loads automatically. Reviewers approve candidates before they go live.
2. **More coding agents** — adding first-class support for additional agents beyond Claude Code, on the same capture-and-recall surface.
3. **Web dashboard** — a browser view over the team store for read-only inspection of recent activity, skill catalogs, and search-quality trends.
4. **Team-mode polish** — single-sign-on for invite redemption, hardware-backed signing keys on devices that support them, an authoritative "forget" workflow for compliance use cases, and richer activity surfaces for admins.
5. **Multi-team hosting** — one sync server hosting several isolated teams, for organizations that want to centralize the infrastructure but keep team data separated.
6. **Hosted offering** — a managed team server for organizations that don't want to self-host.

### What does adoption look like?

Designed for grassroots, individual-engineer adoption first. A single developer can install in about a minute, sees value within the first session — the agent surfaces relevant prior context automatically — and pays no coordination cost. Team rollout is then bottom-up: once enough of the team is using it individually, the team-mode proposal becomes obvious to anyone who's used it for a week.

### Why "Teramind"?

A working title that stuck. *Tera* for the volume of trace data captured; *mind* for the layer that makes it usable. Open to a rename before launch if anyone has a better idea.

---

*Document last updated: 2026-05-17. Maintained by the Teramind core team.*
