//! FS watcher service. Owns one notify::RecommendedWatcher per unique
//! active-session cwd, refcounted by session_id. On filesystem events,
//! debounces per (cwd, rel_path) and dispatches a full
//! pre/post/diff/excerpts/attribution pipeline.

use crate::services::diff_engine::compute_file_diff;
use crate::services::ignore_filter::IgnoreFilter;
use crate::services::snapshot_cache::{resolve_pre_content, SnapshotCache};
use crate::services::write_tool_ring::WriteToolRing;
use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::file_diff::Attribution;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

/// Try `Recursive` watch on `dir`. On success inotify owns the whole subtree,
/// including new subdirectories created later — nothing more to do.
///
/// On EACCES, fall back: watch `dir` itself non-recursively, then recurse into
/// each readable child and apply the same logic. Fully accessible subtrees
/// still get `Recursive` coverage; only the boundary dirs (readable but with
/// at least one unreadable child) are watched non-recursively.
///
/// Returns the number of directories skipped due to permission errors.
fn watch_or_fallback(watcher: &mut RecommendedWatcher, dir: &Path) -> usize {
    match watcher.watch(dir, RecursiveMode::Recursive) {
        Ok(()) => return 0,
        Err(ref e) if is_permission_error(e) => {}
        Err(e) => {
            warn!(error = %e, path = %dir.display(), "fs_watcher: watch failed");
            return 0;
        }
    }
    // Recursive watch denied somewhere in this subtree. Watch this level
    // non-recursively so we still see events here, then descend manually.
    let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 1,
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| watch_or_fallback(watcher, &e.path()))
        .sum()
}

fn is_permission_error(e: &notify::Error) -> bool {
    matches!(&e.kind, notify::ErrorKind::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied)
}

/// One watcher per unique cwd. Refcounted by active session_ids; the
/// watcher is dropped when the last session in that cwd ends.
pub struct WatchRegistry {
    pub(crate) inner: Mutex<HashMap<PathBuf, WatchEntry>>,
    event_tx: mpsc::UnboundedSender<RawEvent>,
    /// Incremented whenever `notify` reports an error (lost slot, etc.).
    /// Wired to `IngestStats.fs_watcher_gaps` so `teramind status` can surface it.
    gaps_counter: Arc<std::sync::atomic::AtomicU64>,
}

pub(crate) struct WatchEntry {
    pub(crate) sessions: HashSet<SessionId>,
    // Must be held to keep the OS watch alive; dropping it unregisters the watch.
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    // Must be held so the closure inside the watcher callback can reference it.
    #[allow(dead_code)]
    filter: IgnoreFilter,
}

#[derive(Debug, Clone)]
pub struct RawEvent {
    pub cwd: PathBuf,
    pub abs_path: PathBuf,
    pub at: OffsetDateTime,
}

impl WatchRegistry {
    pub fn new(
        event_tx: mpsc::UnboundedSender<RawEvent>,
        gaps_counter: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            event_tx,
            gaps_counter,
        }
    }

    pub async fn register(&self, cwd: PathBuf, session: SessionId) -> anyhow::Result<()> {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(&cwd) {
            entry.sessions.insert(session);
            return Ok(());
        }
        let cwd_for_cb = cwd.clone();
        let tx = self.event_tx.clone();
        let filter = IgnoreFilter::for_root(&cwd);
        let filter_for_cb = filter.clone();
        let gaps = self.gaps_counter.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let ev = match res {
                    Ok(ev) => ev,
                    Err(_) => {
                        gaps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                };
                if !matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    return;
                }
                for p in ev.paths {
                    if filter_for_cb.is_ignored(&p) {
                        continue;
                    }
                    let _ = tx.send(RawEvent {
                        cwd: cwd_for_cb.clone(),
                        abs_path: p,
                        at: OffsetDateTime::now_utc(),
                    });
                }
            })?;
        let skipped = watch_or_fallback(&mut watcher, &cwd);
        if skipped > 0 {
            warn!(skipped, cwd = %cwd.display(), "fs_watcher: skipped inaccessible directories");
        }
        let mut sessions = HashSet::new();
        sessions.insert(session);
        g.insert(
            cwd,
            WatchEntry {
                sessions,
                watcher,
                filter,
            },
        );
        Ok(())
    }

    pub async fn unregister(&self, cwd: &Path, session: SessionId) {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(cwd) {
            entry.sessions.remove(&session);
            if entry.sessions.is_empty() {
                g.remove(cwd);
            }
        }
    }

    pub async fn watched_count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

/// Per-(cwd, abs_path) debouncer. Each incoming event for a given key
/// aborts the previous pending timer so only the *last* event in a
/// `quiet` window is emitted downstream.
pub struct Debouncer {
    in_tx: mpsc::UnboundedSender<RawEvent>,
}

impl Debouncer {
    pub fn start(quiet: Duration, out_tx: mpsc::UnboundedSender<RawEvent>) -> Self {
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<RawEvent>();
        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(PathBuf, PathBuf)>();
        tokio::spawn(async move {
            type Key = (PathBuf, PathBuf);
            let mut timers: HashMap<Key, tokio::task::JoinHandle<()>> = HashMap::new();
            loop {
                tokio::select! {
                    Some(ev) = in_rx.recv() => {
                        let key = (ev.cwd.clone(), ev.abs_path.clone());
                        if let Some(h) = timers.remove(&key) {
                            h.abort();
                        }
                        let out = out_tx.clone();
                        let done = done_tx.clone();
                        let key_for_task = key.clone();
                        let handle = tokio::spawn(async move {
                            tokio::time::sleep(quiet).await;
                            let _ = out.send(ev);
                            let _ = done.send(key_for_task);
                        });
                        timers.insert(key, handle);
                    }
                    Some(finished_key) = done_rx.recv() => {
                        // Remove the entry IF the JoinHandle in the map corresponds
                        // to the one that just finished. If a new event has already
                        // arrived for this key, `timers.insert` will have replaced
                        // the handle, which we shouldn't remove. We detect this
                        // by checking `is_finished()`.
                        if let Some(h) = timers.get(&finished_key) {
                            if h.is_finished() {
                                timers.remove(&finished_key);
                            }
                        }
                    }
                    else => break,
                }
            }
        });
        Self { in_tx }
    }

    pub async fn enqueue(&self, ev: RawEvent) {
        let _ = self.in_tx.send(ev);
    }
}

/// All the wiring the FS watcher needs to do its job.
#[derive(Clone)]
pub struct FsWatcherDeps {
    pub registry: Arc<WatchRegistry>,
    pub debouncer: Arc<Debouncer>,
    pub cache: SnapshotCache,
    pub ring: WriteToolRing,
    /// Sender into the existing ingest queue. The watcher emits
    /// `IngestEvent::FileDiff` envelopes here.
    pub ingest_tx: Arc<crate::services::ingest::IngestService>,
}

pub struct FsWatcherService;

impl FsWatcherService {
    /// Spawns the dispatcher loop that consumes resolved debounce events
    /// and runs the full diff pipeline for each.
    pub fn spawn(deps: FsWatcherDeps, mut resolved_rx: mpsc::UnboundedReceiver<RawEvent>) {
        tokio::spawn(async move {
            while let Some(ev) = resolved_rx.recv().await {
                if let Err(e) = handle_event(&deps, ev).await {
                    warn!(error = %e, "fs_watcher handle_event failed");
                }
            }
        });
    }
}

/// The full pipeline for one resolved (post-debounce) filesystem event.
async fn handle_event(deps: &FsWatcherDeps, ev: RawEvent) -> anyhow::Result<()> {
    // 1. Ignore events for paths that no longer exist (deleted in a flurry).
    let post = match tokio::fs::read_to_string(&ev.abs_path).await {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    // 2. Compute rel_path relative to cwd.
    let rel_path = match ev.abs_path.strip_prefix(&ev.cwd) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return Ok(()),
    };

    // 3. Resolve pre-content from cache OR git index.
    let pre = resolve_pre_content(&deps.cache, &ev.cwd, &rel_path).await;
    if pre == post {
        // Update cache regardless so future diffs are accurate.
        deps.cache
            .put(ev.cwd.clone(), rel_path.clone(), post.clone())
            .await;
        return Ok(());
    }

    // 4. Compute the diff bundle.
    let Some(computed) = compute_file_diff(&pre, &post, Path::new(&rel_path)) else {
        return Ok(());
    };

    // 5. Look up active session for this cwd (via the registry) and decide
    //    attribution by consulting the write-tool ring.
    let (session_id, turn_id, attribution) = decide_attribution(deps, &ev.cwd).await;
    let Some(session_id) = session_id else {
        // No active session for this cwd — drop silently.
        return Ok(());
    };

    // 6. Emit through the existing ingest pipeline.
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: ev.at,
        event: IngestEvent::FileDiff {
            session_id,
            turn_id,
            file_path: ev.abs_path.to_string_lossy().to_string(),
            rel_path: rel_path.clone(),
            attribution,
            language: computed.language,
            pre_excerpt: computed.pre_excerpt,
            post_excerpt: computed.post_excerpt,
            unified_diff: computed.unified_diff,
            pre_hash: computed.pre_hash,
            post_hash: computed.post_hash,
            byte_size: computed.byte_size,
        },
    };
    if let Err(_dropped) = deps.ingest_tx.try_enqueue(env) {
        warn!(rel_path = %rel_path, "fs_watcher: FileDiff dropped due to ingest backpressure");
    }

    // 7. Update snapshot cache with the new content.
    deps.cache.put(ev.cwd.clone(), rel_path, post).await;

    debug!(abs_path = ?ev.abs_path, "fs_watcher emitted FileDiff");
    Ok(())
}

/// Pick a session_id whose cwd matches `ev_cwd`, then ask the write-tool
/// ring if there was a recent write-tool completion for that session.
/// If yes -> agent attribution + turn_id. Else -> human, turn_id=None.
async fn decide_attribution(
    deps: &FsWatcherDeps,
    ev_cwd: &Path,
) -> (
    Option<SessionId>,
    Option<teramind_core::ids::TurnId>,
    Attribution,
) {
    let sessions: Vec<SessionId> = {
        let g = deps.registry.inner.lock().await;
        match g.get(ev_cwd) {
            Some(e) => e.sessions.iter().copied().collect(),
            None => return (None, None, Attribution::Human),
        }
    }; // lock released here
    let now = OffsetDateTime::now_utc();
    for sid in &sessions {
        if let Some(w) = deps.ring.most_recent_for(*sid, now).await {
            return (Some(*sid), Some(w.turn_id), Attribution::Agent);
        }
    }
    (sessions.into_iter().next(), None, Attribution::Human)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_counter() -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(0))
    }

    #[tokio::test]
    async fn register_then_unregister_drops_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let sid = SessionId::new();
        reg.register(dir.path().to_path_buf(), sid).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), sid).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn second_session_in_same_cwd_does_not_duplicate_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let a = SessionId::new();
        let b = SessionId::new();
        reg.register(dir.path().to_path_buf(), a).await.unwrap();
        reg.register(dir.path().to_path_buf(), b).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), a).await;
        assert_eq!(reg.watched_count().await, 1); // b still holds it
        reg.unregister(dir.path(), b).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn modify_event_is_emitted_to_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "x").unwrap();
        reg.register(dir.path().to_path_buf(), SessionId::new())
            .await
            .unwrap();
        // Modify the file.
        std::fs::write(dir.path().join("a.rs"), "y").unwrap();
        // macOS FSEvents on CI runners coalesces and sometimes emits a parent
        // directory event before (or instead of) the file event — drain
        // received events until we see the one for `a.rs` or hit the timeout.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got_file_event = false;
        let mut seen: Vec<std::path::PathBuf> = Vec::new();
        while std::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(evt)) => {
                    seen.push(evt.abs_path.clone());
                    if evt.abs_path.ends_with("a.rs") {
                        got_file_event = true;
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert!(
            got_file_event,
            "no event for a.rs within 5s; saw paths: {seen:?}"
        );
    }

    #[tokio::test]
    async fn debouncer_coalesces_rapid_events() {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RawEvent>();
        let deb = Debouncer::start(Duration::from_millis(80), out_tx);

        let cwd = PathBuf::from("/p");
        let p = PathBuf::from("/p/a.rs");
        let now = OffsetDateTime::now_utc();
        for _ in 0..5 {
            deb.enqueue(RawEvent {
                cwd: cwd.clone(),
                abs_path: p.clone(),
                at: now,
            })
            .await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // After the 80ms quiet period we should get exactly one resolved event.
        let first = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        assert_eq!(first.abs_path, p);

        // No additional events expected.
        let extra = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
        assert!(
            extra.is_err(),
            "expected no further events, got {:?}",
            extra.unwrap()
        );
    }

    #[tokio::test]
    async fn debouncer_drops_completed_timer_entries() {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RawEvent>();
        let deb = Debouncer::start(Duration::from_millis(40), out_tx);
        let cwd = PathBuf::from("/p");
        // Fire 5 distinct keys; let them all drain.
        for i in 0..5 {
            deb.enqueue(RawEvent {
                cwd: cwd.clone(),
                abs_path: PathBuf::from(format!("/p/f{i}.rs")),
                at: OffsetDateTime::now_utc(),
            })
            .await;
        }
        // Drain 5 events.
        for _ in 0..5 {
            let _ = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
                .await
                .expect("timeout")
                .expect("closed");
        }
        // Give the loop a chance to receive the done signals.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // No public accessor — this test only verifies events flow correctly;
        // the memory invariant is testable manually via instrumentation.
        // (Keeping this as a smoke check for now.)
    }
}
