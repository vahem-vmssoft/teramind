//! E2E: `teramind-search-eval run --json` must emit exactly one
//! QualityRunOutput JSON object to stdout. Per dashboard §6 the
//! object is what the sync-server's /admin/quality/runs ingest path
//! consumes, so the field schema is part of the contract.

use std::path::PathBuf;
use std::process::Command;

fn cargo_bin(name: &str) -> PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
                let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
                let workspace_root = PathBuf::from(&manifest)
                    .ancestors()
                    .find(|p| p.join("Cargo.toml").exists() && p.join("Cargo.lock").exists())
                    .unwrap_or_else(|| std::path::Path::new(&manifest))
                    .to_path_buf();
                workspace_root.join("target").to_string_lossy().into_owned()
            });
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            PathBuf::from(target).join(profile).join(name)
        })
}

fn workspace_root() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    PathBuf::from(&manifest)
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("Cargo.lock").exists())
        .expect("workspace root")
        .to_path_buf()
}

// Marked #[ignore] because teramind-search-eval still uses the embedded
// `PgSupervisor` (does not honor `TERAMIND_TEST_PG_URL`), and embedded PG
// init can hit SysV `shmget(ENOSPC)` on hosts where the kernel shared-memory
// quota is exhausted. Run with `cargo test -p teramind-search-eval --
// --ignored` once the harness is migrated to the shared fixture.
#[test]
#[ignore]
fn json_flag_emits_single_quality_run_output_object() {
    // Ensure the binary exists (Cargo builds it before integration tests,
    // but fall back to `cargo build` for robustness in dev environments).
    let bin = cargo_bin("teramind-search-eval");
    if !bin.exists() {
        let _ = Command::new("cargo")
            .args(["build", "-p", "teramind-search-eval"])
            .current_dir(workspace_root())
            .status();
    }

    let corpus = workspace_root().join("benches/search-eval");
    let out_tmp = tempfile::tempdir().unwrap();

    let mut cmd = Command::new(&bin);
    cmd.args([
        "run",
        "--corpus",
        corpus.to_str().unwrap(),
        "--out",
        out_tmp.path().to_str().unwrap(),
        "--json",
    ]);
    // Forward optional external PG URL — embedded PG also works but is
    // slower; the harness picks up the env var only if PgSupervisor is
    // not used. We forward it to keep parity with other CLI E2E tests.
    if let Ok(url) = std::env::var("TERAMIND_TEST_PG_URL") {
        cmd.env("TERAMIND_TEST_PG_URL", url);
    }

    let output = cmd.output().expect("spawn teramind-search-eval");
    assert!(
        output.status.success(),
        "binary failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    // Should be one JSON object on stdout (possibly with trailing newline).
    let trimmed = stdout.trim();
    let parsed: serde_json::Value =
        serde_json::from_str(trimmed).unwrap_or_else(|e| panic!("stdout not JSON: {e}\n{trimmed}"));
    let obj = parsed.as_object().expect("top-level JSON must be an object");

    // QualityRunOutput contract — fields documented in teramind_core::quality.
    for key in [
        "baseline_label",
        "ndcg10",
        "mrr",
        "precision_5",
        "precision_10",
        "recall_10",
        "p50_latency_ms",
        "p95_latency_ms",
        "query_count",
        "corpus_size",
        "per_class",
    ] {
        assert!(
            obj.contains_key(key),
            "QualityRunOutput missing key {key}; got {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    // Numeric metrics must be finite numbers, not null/string.
    for key in [
        "ndcg10",
        "mrr",
        "precision_5",
        "precision_10",
        "recall_10",
        "p50_latency_ms",
        "p95_latency_ms",
    ] {
        assert!(
            obj[key].as_f64().map(|v| v.is_finite()).unwrap_or(false),
            "{key} is not a finite f64: {}",
            obj[key]
        );
    }
    assert!(obj["query_count"].as_u64().is_some());
    assert!(obj["corpus_size"].as_u64().is_some());
}
