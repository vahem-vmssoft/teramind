//! Hand-curated query bank: 100 queries across 5 intent classes (>=20 each).
//!
//! Each entry's `triggers` slice is the corpus generator's contract — the
//! generator plants one of those tokens in any session it tags as
//! "relevant to this query".

use crate::types::QueryClass;

pub struct QueryBankEntry {
    pub id: &'static str,
    pub class: QueryClass,
    pub text: &'static str,
    pub triggers: &'static [&'static str],
}

pub const QUERIES: &[QueryBankEntry] = &[
    // Natural Language (nl-*)
    nl(
        "nl-01",
        "how did we fix the JWT expiry bug",
        &["JWT expiry fix", "expiry bug"],
    ),
    nl(
        "nl-02",
        "what changed about the rate limiter last week",
        &["rate limiter rewrite", "rate limit fix"],
    ),
    nl(
        "nl-03",
        "did anyone solve the redis connection pool leak",
        &["redis pool leak", "connection pool fix"],
    ),
    nl(
        "nl-04",
        "explain the migration to tokio 1.x",
        &["tokio upgrade", "tokio 1.0 migration"],
    ),
    nl(
        "nl-05",
        "where do we set the read replica routing",
        &["read replica", "replica routing"],
    ),
    nl(
        "nl-06",
        "how is the websocket reconnect handled",
        &["websocket reconnect", "ws reconnect backoff"],
    ),
    nl(
        "nl-07",
        "summary of the auth refactor",
        &["auth refactor", "auth middleware rewrite"],
    ),
    nl(
        "nl-08",
        "why did we switch from reqwest to ureq",
        &["reqwest to ureq", "switched http client"],
    ),
    nl(
        "nl-09",
        "what does the worker pool sizing logic do",
        &["worker pool sizing", "pool sizing heuristic"],
    ),
    nl(
        "nl-10",
        "fix the cors preflight failure",
        &["cors preflight", "preflight fix"],
    ),
    nl(
        "nl-11",
        "why does the smoke test flake on macos",
        &["macos smoke flake", "smoke test flake"],
    ),
    nl(
        "nl-12",
        "where is the postgres backoff configured",
        &["postgres backoff", "pg reconnect backoff"],
    ),
    nl(
        "nl-13",
        "how do we generate sitemaps",
        &["sitemap generation", "sitemap pipeline"],
    ),
    nl(
        "nl-14",
        "explain the new pricing column",
        &["pricing column", "price tier column"],
    ),
    nl(
        "nl-15",
        "what does the cli doctor do",
        &["doctor command", "diagnostic doctor"],
    ),
    nl(
        "nl-16",
        "trace the audit log entries",
        &["audit log entries", "audit trail"],
    ),
    nl(
        "nl-17",
        "why is the cache invalidation so slow",
        &["cache invalidation slow", "slow cache flush"],
    ),
    nl(
        "nl-18",
        "what was the fix for the pagination off-by-one",
        &["pagination off by one", "off by one fix"],
    ),
    nl(
        "nl-19",
        "explain the new error envelope",
        &["error envelope", "structured error envelope"],
    ),
    nl(
        "nl-20",
        "where are http retries configured",
        &["http retry config", "retry middleware"],
    ),
    // Stack Trace (st-*)
    st(
        "st-01",
        "thread main panicked at serializer.rs:142",
        &["panicked at serializer.rs:142", "serializer panic"],
    ),
    st(
        "st-02",
        "NullPointerException at UserController.java:88",
        &[
            "NullPointerException at UserController",
            "NPE at UserController",
        ],
    ),
    st(
        "st-03",
        "TypeError: cannot read properties of undefined",
        &[
            "TypeError: cannot read properties",
            "undefined property error",
        ],
    ),
    st(
        "st-04",
        "RuntimeError: dictionary changed size during iteration",
        &[
            "dictionary changed size during iteration",
            "dict size change",
        ],
    ),
    st(
        "st-05",
        "panic: runtime error: index out of range",
        &["index out of range", "panic index out of range"],
    ),
    st(
        "st-06",
        "sqlx error: column already exists",
        &["sqlx column already exists", "column exists error"],
    ),
    st(
        "st-07",
        "thread tokio-runtime-worker panicked",
        &["tokio-runtime-worker panicked", "runtime worker panic"],
    ),
    st(
        "st-08",
        "AttributeError: NoneType object has no attribute",
        &["NoneType has no attribute", "AttributeError None"],
    ),
    st(
        "st-09",
        "ECONNREFUSED 127.0.0.1:5432",
        &["ECONNREFUSED 127.0.0.1:5432", "connection refused pg"],
    ),
    st(
        "st-10",
        "unrecoverable error: heap out of memory",
        &["heap out of memory", "OOM heap"],
    ),
    st(
        "st-11",
        "ImportError: cannot import name foo",
        &["cannot import name foo", "ImportError foo"],
    ),
    st(
        "st-12",
        "Exception in thread http-handler",
        &["http-handler exception", "thread http-handler"],
    ),
    st(
        "st-13",
        "fatal: not a git repository",
        &["not a git repository", "git fatal repo"],
    ),
    st(
        "st-14",
        "ValueError: too many values to unpack",
        &["too many values to unpack", "ValueError unpack"],
    ),
    st(
        "st-15",
        "unhandled rejection: socket hang up",
        &["socket hang up", "unhandled rejection socket"],
    ),
    st(
        "st-16",
        "panic: assignment to entry in nil map",
        &["assignment to entry in nil map", "nil map go"],
    ),
    st(
        "st-17",
        "OperationalError: server closed the connection",
        &["server closed the connection", "OperationalError"],
    ),
    st(
        "st-18",
        "stack overflow",
        &["stack overflow", "max call stack"],
    ),
    st(
        "st-19",
        "Permission denied publickey",
        &["Permission denied (publickey)", "ssh permission denied"],
    ),
    st(
        "st-20",
        "fatal: Authentication failed",
        &["Authentication failed", "git authentication failed"],
    ),
    // Code Snippet (cs-*)
    cs(
        "cs-01",
        "if let Some headers Authorization",
        &[
            "self.headers.get(Authorization)",
            "Authorization header check",
        ],
    ),
    cs(
        "cs-02",
        "tokio::spawn async move",
        &["tokio::spawn(async move", "spawn async move"],
    ),
    cs(
        "cs-03",
        "let mut conn pool acquire await",
        &["pool.acquire().await", "conn = pool.acquire"],
    ),
    cs(
        "cs-04",
        "useState initial value",
        &["useState(initialValue)", "useState hook init"],
    ),
    cs(
        "cs-05",
        "async fn handler Request Response",
        &["async fn handler", "fn handler Request Response"],
    ),
    cs(
        "cs-06",
        "df groupby user_id agg",
        &["df.groupby user_id", "pandas groupby user_id"],
    ),
    cs(
        "cs-07",
        "ctx Done channel",
        &["ctx.Done()", "select ctx.Done"],
    ),
    cs(
        "cs-08",
        "match self state Active",
        &["State::Active", "match self.state"],
    ),
    cs(
        "cs-09",
        "axios interceptors response use",
        &["axios.interceptors.response.use", "axios interceptors"],
    ),
    cs(
        "cs-10",
        "derive Debug Serialize Deserialize",
        &[
            "derive(Debug, Serialize, Deserialize)",
            "derive Debug Serialize Deserialize",
        ],
    ),
    cs(
        "cs-11",
        "for i item vec iter enumerate",
        &["vec.iter().enumerate()", "iter enumerate loop"],
    ),
    cs(
        "cs-12",
        "redis PubSub channel",
        &["redis.PubSub()", "redis pubsub init"],
    ),
    cs(
        "cs-13",
        "Component selector tag",
        &["Component selector", "Angular Component selector"],
    ),
    cs(
        "cs-14",
        "DB exec SQL_INSERT_USER",
        &["DB.exec(SQL_INSERT_USER)", "exec SQL_INSERT_USER"],
    ),
    cs(
        "cs-15",
        "if err nil return nil err go",
        &["if err != nil { return nil, err }", "go err return"],
    ),
    cs(
        "cs-16",
        "try JSON parse body catch",
        &["JSON.parse(body)", "try JSON.parse"],
    ),
    cs(
        "cs-17",
        "ChaCha20Rng seed_from_u64",
        &["ChaCha20Rng::seed_from_u64", "chacha20 seed"],
    ),
    cs(
        "cs-18",
        "flask route api v1 health",
        &["app.route(/api/v1/health)", "flask route health"],
    ),
    cs(
        "cs-19",
        "ErrorKind WouldBlock io",
        &["io::ErrorKind::WouldBlock", "WouldBlock ErrorKind"],
    ),
    cs(
        "cs-20",
        "return new Promise resolve reject",
        &["new Promise((resolve, reject)", "Promise resolve reject"],
    ),
    // Tool-typed (tt-*)
    tt(
        "tt-01",
        "tool Edit path src parser.rs",
        &["src/parser.rs Edit", "Edit src/parser.rs"],
    ),
    tt(
        "tt-02",
        "tool Bash command cargo test",
        &["cargo test bash", "bash cargo test"],
    ),
    tt(
        "tt-03",
        "tool Read path Cargo.toml",
        &["Cargo.toml Read", "Read Cargo.toml"],
    ),
    tt(
        "tt-04",
        "tool Grep pattern TODO",
        &["Grep TODO", "grep pattern TODO"],
    ),
    tt(
        "tt-05",
        "tool Write path README.md",
        &["README.md Write", "Write README.md"],
    ),
    tt(
        "tt-06",
        "tool Edit path src main.rs",
        &["src/main.rs Edit", "Edit src/main.rs"],
    ),
    tt(
        "tt-07",
        "tool Bash command git status",
        &["git status bash", "bash git status"],
    ),
    tt(
        "tt-08",
        "tool Read path package.json",
        &["package.json Read", "Read package.json"],
    ),
    tt(
        "tt-09",
        "tool Grep pattern fixme",
        &["Grep fixme", "grep pattern fixme"],
    ),
    tt(
        "tt-10",
        "tool Edit path tests integration.rs",
        &["tests/integration.rs Edit", "Edit integration test"],
    ),
    tt(
        "tt-11",
        "tool Bash command npm install",
        &["npm install bash", "bash npm install"],
    ),
    tt(
        "tt-12",
        "tool Read path src lib.rs",
        &["src/lib.rs Read", "Read src/lib.rs"],
    ),
    tt(
        "tt-13",
        "tool Write path src config.rs",
        &["src/config.rs Write", "Write src/config.rs"],
    ),
    tt(
        "tt-14",
        "tool Edit path Cargo.lock",
        &["Cargo.lock Edit", "Edit Cargo.lock"],
    ),
    tt(
        "tt-15",
        "tool Bash command docker build",
        &["docker build bash", "bash docker build"],
    ),
    tt(
        "tt-16",
        "tool Grep pattern unwrap",
        &["Grep unwrap", "grep pattern unwrap"],
    ),
    tt(
        "tt-17",
        "tool MultiEdit path src repo.rs",
        &["src/repo.rs MultiEdit", "MultiEdit repo.rs"],
    ),
    tt(
        "tt-18",
        "tool Bash command make test",
        &["make test bash", "bash make test"],
    ),
    tt(
        "tt-19",
        "tool Read path docker-compose.yml",
        &["docker-compose.yml Read", "Read docker-compose"],
    ),
    tt(
        "tt-20",
        "tool Edit path src db.rs",
        &["src/db.rs Edit", "Edit src/db.rs"],
    ),
    // Symbolic / file path (sp-*)
    sp(
        "sp-01",
        "serialize_with_options",
        &["fn serialize_with_options", "serialize_with_options self"],
    ),
    sp(
        "sp-02",
        "crates teramind-core src redact.rs",
        &["crates/teramind-core/src/redact.rs", "redact.rs"],
    ),
    sp(
        "sp-03",
        "validate_input",
        &["fn validate_input", "validate_input("],
    ),
    sp(
        "sp-04",
        "AuthMiddleware",
        &["class AuthMiddleware", "AuthMiddleware {"],
    ),
    sp("sp-05", "openapi yaml", &["openapi.yaml", "openapi spec"]),
    sp(
        "sp-06",
        "FeatureFlag enum",
        &["enum FeatureFlag", "FeatureFlag::"],
    ),
    sp(
        "sp-07",
        "scripts release.sh",
        &["scripts/release.sh", "release.sh"],
    ),
    sp(
        "sp-08",
        "Datadog_client",
        &["Datadog_client", "Datadog_client.send"],
    ),
    sp(
        "sp-09",
        "k8s deployment.yaml",
        &["k8s/deployment.yaml", "deployment.yaml"],
    ),
    sp(
        "sp-10",
        "format_event_payload",
        &["fn format_event_payload", "format_event_payload("],
    ),
    sp(
        "sp-11",
        "Prometheus Counter",
        &["Prometheus.Counter", "Counter("],
    ),
    sp(
        "sp-12",
        "lib cache invalidator.go",
        &["lib/cache/invalidator.go", "invalidator.go"],
    ),
    sp(
        "sp-13",
        "RateLimitExceeded",
        &["RateLimitExceeded", "Err::RateLimitExceeded"],
    ),
    sp(
        "sp-14",
        "src utils url.ts",
        &["src/utils/url.ts", "utils/url.ts"],
    ),
    sp(
        "sp-15",
        "parse_iso8601",
        &["fn parse_iso8601", "parse_iso8601("],
    ),
    sp(
        "sp-16",
        "WorkerHealth struct",
        &["struct WorkerHealth", "WorkerHealth {"],
    ),
    sp(
        "sp-17",
        ".github workflows ci.yml",
        &[".github/workflows/ci.yml", "ci.yml"],
    ),
    sp("sp-18", "redactPII", &["redactPII", "function redactPII"]),
    sp(
        "sp-19",
        "internal auth jwt.go",
        &["internal/auth/jwt.go", "auth/jwt.go"],
    ),
    sp(
        "sp-20",
        "EventBus publish",
        &["EventBus.publish", "EventBus.publish("],
    ),
];

const fn nl(
    id: &'static str,
    text: &'static str,
    triggers: &'static [&'static str],
) -> QueryBankEntry {
    QueryBankEntry {
        id,
        class: QueryClass::NaturalLanguage,
        text,
        triggers,
    }
}
const fn st(
    id: &'static str,
    text: &'static str,
    triggers: &'static [&'static str],
) -> QueryBankEntry {
    QueryBankEntry {
        id,
        class: QueryClass::StackTrace,
        text,
        triggers,
    }
}
const fn cs(
    id: &'static str,
    text: &'static str,
    triggers: &'static [&'static str],
) -> QueryBankEntry {
    QueryBankEntry {
        id,
        class: QueryClass::CodeSnippet,
        text,
        triggers,
    }
}
const fn tt(
    id: &'static str,
    text: &'static str,
    triggers: &'static [&'static str],
) -> QueryBankEntry {
    QueryBankEntry {
        id,
        class: QueryClass::ToolTyped,
        text,
        triggers,
    }
}
const fn sp(
    id: &'static str,
    text: &'static str,
    triggers: &'static [&'static str],
) -> QueryBankEntry {
    QueryBankEntry {
        id,
        class: QueryClass::SymbolicPath,
        text,
        triggers,
    }
}
