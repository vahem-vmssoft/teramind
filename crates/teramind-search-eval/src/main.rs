use clap::{Parser, Subcommand};

/// Teramind search-effectiveness benchmark.
#[derive(Debug, Parser)]
#[command(name = "teramind-search-eval", about = "Run the L5 search benchmark.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Regenerate `benches/search-eval/corpus/*.jsonl` and `qrels.toml`
    /// from the deterministic seed.
    GenerateCorpus {
        /// Number of synthetic sessions to emit. Default 500.
        #[arg(long, default_value = "500")]
        scale: u32,
        /// Output root. Defaults to `benches/search-eval/`.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Load the corpus into a throwaway DB, run every query, write
    /// `eval-results.json` + a Markdown scorecard.
    Run {
        /// Path to the corpus root.
        #[arg(long, default_value = "benches/search-eval")]
        corpus: std::path::PathBuf,
        /// Output directory for `eval-results.json` and the scorecard.
        #[arg(long, default_value = "benches/search-eval")]
        out: std::path::PathBuf,
        /// Enable the semantic blend; writes outputs to *-semantic.{json,md}.
        #[arg(long)]
        semantic: bool,
        /// Weight to apply to the semantic score when --semantic is set.
        #[arg(long, default_value = "0.4")]
        semantic_weight: f32,
    },
    /// Compare `eval-results.json` against `baseline.json` and exit
    /// non-zero if any regression gate trips.
    CompareBaseline {
        #[arg(long, default_value = "benches/search-eval/eval-results.json")]
        results: std::path::PathBuf,
        #[arg(long, default_value = "benches/search-eval/baseline.json")]
        baseline: std::path::PathBuf,
        /// Rewrite baseline.json from results (use only with [eval-baseline-update]).
        #[arg(long)]
        update_baseline: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::GenerateCorpus { scale, out } => {
            let dest = out.unwrap_or_else(|| "benches/search-eval".into());
            teramind_search_eval::generator::generate_to(&dest, scale)
        }
        Cmd::Run {
            corpus,
            out,
            semantic,
            semantic_weight,
        } => teramind_search_eval::harness::run(&corpus, &out, semantic, semantic_weight).await,
        Cmd::CompareBaseline {
            results,
            baseline,
            update_baseline,
        } => teramind_search_eval::gates::compare(&results, &baseline, update_baseline),
    }
}
