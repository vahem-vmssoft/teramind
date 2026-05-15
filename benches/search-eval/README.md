# Teramind Search Eval Corpus

This directory holds the L5 search-effectiveness benchmark assets:

- `corpus/sessions.jsonl`, `turns.jsonl`, `tool_calls.jsonl`, `file_diffs.jsonl`
  — 500-session synthetic corpus, regenerable via the generator.
- `queries.toml` — 100 hand-curated queries across 5 intent classes
  (≥20 per class).
- `qrels.toml` — per-query relevance judgments (graded 0/1/2).
- `baseline.json` — committed metrics from `main`. PRs that touch
  search-related paths must keep metrics within spec thresholds:
  - nDCG@10 (overall): ≤ 2 pp drop
  - nDCG@10 (any class): ≤ 5 pp drop
  - MRR (overall): ≤ 0.03 absolute drop
  - p95 query latency: ≤ 3 s
- `eval-results.json` + `eval-scorecard.md` — outputs of the most recent
  local run; gitignored.

## Regenerating

```sh
cargo run --release -p teramind-search-eval -- generate-corpus --scale=500
cargo run --release -p teramind-search-eval -- run
```

## Rebaselining (intentional metric move)

If a PR genuinely improves ranking and the gates trip because the baseline
is stale, attach `[eval-baseline-update]` to the PR description AND commit
the new `baseline.json`:

```sh
cargo run --release -p teramind-search-eval -- run
cargo run --release -p teramind-search-eval -- compare-baseline --update-baseline
git add benches/search-eval/baseline.json
git commit -m "eval: rebaseline (improves nDCG@10 by X.Y pp)"
```

Reviewers can inspect the new numbers in the diff.
