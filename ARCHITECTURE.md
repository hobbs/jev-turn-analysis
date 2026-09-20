# Implementation plan

Implemented in the shared workspace by three GPT-6 Astra agents at medium effort.
The root agent owned architecture, interface reviews, public-data acquisition, and
independent validation. See `VALIDATION.md` for evidence and remaining limits.

`jta` is a Rust CLI with a local-first, two-stage analysis pipeline. Transcript
content is untrusted data: recorded commands are never executed. Explicit imports
remain supported. Project discovery reads native session metadata and imports only
matching sessions when the user invokes discovery or no-path analysis.

## Project discovery extension

Default analysis unit: a Git checkout root (nearest `.git` directory or file), or
the current directory outside Git. Nested repositories and separate worktrees are
distinct projects. Canonical paths resolve symlinks; matching uses path components,
never string prefixes. Keep each session's recorded `repository` cwd and add an
optional `project_root` (serde default for old evidence). Source `agent` remains
`codex` or `claude_code`; neither sessions nor tasks are merged across agents.

- `jta analyze` with no path discovers both sources for cwd's project and initializes
  a project-local workspace if needed. Explicit `--workspace` chooses storage only.
- `jta discover` lists matching files and source counts offline; no service calls.
- Discovery supports `--project PATH`, `--agent codex|claude|all`,
  `--codex-home PATH`, and `--claude-config-dir PATH`. Explicit roots completely
  replace that source's defaults. Respect CODEX_HOME and CLAUDE_CONFIG_DIR otherwise.
- Native sources: Codex `sessions/` plus `archived_sessions/`, Claude `projects/`.
  Read bounded metadata first; only fully parse selected session files. Never match
  Claude's lossy encoded directory names alone. Exclude helper/subagent transcripts
  where deterministically identifiable, and expose skipped/malformed counts.
- Keep explicit `analyze PATH` and `analyze --all` behavior. `--agent` applies to
  these too. `--project`/source roots are only valid for native discovery.
- Reports include project and agent breakdowns (sessions, turns, outcomes,
  verification, resource coverage), and `--agent` filters report/snapshot/review.
  Patterns retain source-agent counts. Agent comparisons describe observed cohorts,
  not causal evidence that one agent is better.
- Testing MUST use existing public fixtures and temporary simulated stores with
  explicit roots. Never invoke default discovery against the user's actual home.

Extension ownership: discovery agent owns `src/discovery.rs`, model/ingestion
project identity additions, library export and unit tests. Application agent owns
CLI/workspace behavior and integration tests. Analytics agent owns analytics
filters/breakdowns, docs, and analytics tests. Root owns architecture and independent
validation. Reuse the requested GPT-6 Astra / medium agents.

## Tasks and ownership

1. **Core:** Cargo scaffolding; shared schema; configuration; deterministic Codex
   and Claude Code adapters; redaction; evidence and dependency extraction;
   content-addressed revisions; atomic local persistence. Owner: core agent.
2. **Services:** verified Jev API adapter; bounded outcome-aware evidence packets;
   complete categorical probability validation; optional OpenAI/OpenRouter review;
   mock HTTP tests and error handling. Owner: services agent.
3. **Application:** CLI command tree; imports and checkpointed scoring; aggregation,
   pattern selection, review, recommendations, snapshots/comparisons, human labels,
   calibration, retention; readable and versioned JSON output. Owner: application agent.
4. **Integration and validation:** public-data acquisition with provenance; integration
   tests; real API smoke tests using environment credentials; architecture review;
   installation and compatibility documentation. Owner: root, delegating fixes.

## Shared module contracts

One package `jev-turn-analysis`, library `jta`, binary `jta`. Modules: `model`,
`config`, `redact`, `ingest`, `store`, `services`, `analytics`, `cli`.
Use `anyhow::Result`, serde types, tokio/reqwest for HTTP, clap for CLI.

The core agent owns `model.rs` and `config.rs`, creates them first, and sends concrete
types to other agents. All agents coordinate interface changes directly.

Shared domain model (all serializable, cloneable):

- `Session`: `id`, `revision`, `source_path`, `agent`, optional recorded cwd `repository`
  and normalized `project_root`,
  `imported_at`, optional `started_at`/`ended_at` (RFC3339 strings), `events`,
  `turns`, `warnings`, `parser_version`, `redaction_fingerprint`.
- `Event`: one-based source `line`, `kind`, optional `timestamp`, `text`, optional
  `tool_name`/`call_id`, optional JSON `input`, optional `is_error`, optional token
  usage, optional source `message_id`.
- `Turn`: one-based numeric `id`, `event_indices`, `intent`, `tools`, `commands`,
  `files_read`, `files_changed`, `errors`, `retry_of`, `candidate_downstream`,
  `verification`, optional usage and duration. Preserve unknowns rather than zeros.
- `Distribution`: `selected: String`, `probabilities: BTreeMap<String,f64>`;
  helper `confidence()` returns selected probability. Shared rubric constants.
- `Analysis`: `id`, `session_id`, `revision`, `created_at`, `config_fingerprint`,
  `rubric_version`, `session: BTreeMap<String,Distribution>`,
  `turns: BTreeMap<u32,TurnJudgment>`, `warnings`, optional service usage.
- `TurnJudgment`: `answers: BTreeMap<String,Distribution>`,
  `secondary_opportunities: Vec<String>`, optional `downstream_turn: u32`,
  `inconsistencies: Vec<String>`.
- Review/pattern/report/snapshot/label artifacts use application-owned structs or
  JSON, persisted through generic store methods.

Config: `Config { schema_version, jev: JevConfig, review: ReviewConfig,
redaction: RedactionConfig, retention_days: u32, min_pattern_sessions: usize }`.
`JevConfig { endpoint, api_key_env, model, max_context_chars: usize,
max_questions: usize, timeout_secs: u64 }`.
`ReviewConfig { provider, endpoint, api_key_env, model, timeout_secs: u64 }`;
provider is a string (`openai`, `openrouter`, or `none`).
`RedactionConfig { enabled: bool, patterns: Vec<String> }`.
Never serialize actual credentials. Environment/.env loading is CLI-owned.

Core public API:

```text
ingest::import_path(path: &Path, config: &Config) -> Result<Vec<Session>>
ingest::parse_file(path: &Path, config: &Config) -> Result<Session>
store::Workspace::init(root: &Path, config: &Config) -> Result<Workspace>
store::Workspace::discover(explicit: Option<&Path>) -> Result<Workspace>
Workspace { pub root: PathBuf } // root containing .jta, not .jta itself
Workspace::config() -> Result<Config>
Workspace::save_session(&Session) -> Result<()>
Workspace::sessions() -> Result<Vec<Session>> // current revision of each session
Workspace::session(id: &str, revision: Option<&str>) -> Result<Session>
Workspace::save_analysis(&Analysis) -> Result<()>
Workspace::analyses() -> Result<Vec<Analysis>>
Workspace::save_json<T: Serialize>(collection: &str, id: &str, value: &T) -> Result<()>
Workspace::load_json<T: DeserializeOwned>(collection: &str, id: &str) -> Result<T>
Workspace::list_json<T: DeserializeOwned>(collection: &str) -> Result<Vec<T>>
Workspace::data_dir() -> PathBuf
config::analysis_fingerprint(&Config) -> String
```

Services public API (coordinate any refinements):

```text
services::prepare_analysis(session: &Session, config: &Config) -> Result<Vec<Value>>
services::score_session(session: &Session, config: &Config) -> Result<Analysis> // async
services::prepare_review(evidence: &Value, config: &Config) -> Result<Value>
services::review(evidence: &Value, config: &Config) -> Result<Value> // async
```

`prepare_*` never performs network calls or requires credentials. Scoring accepts
only validated full Jev distributions, never synthetic LLM judgments or heuristics.
Review returns structured proposals with support references, scope, risk, and
evaluation plan. CLI validates references against selected evidence before saving.

## Acceptance checks

- Public Claude Code and Codex logs import deterministically, with provenance.
- Duplicate content is not counted twice; revisions preserve prior analyses.
- Malformed records, unknown formats, orphan results and truncation are surfaced.
- Redaction precedes persistence and remote payloads; .env remains ignored.
- Tests exercise HTTP contracts without paid calls; real smoke tests are bounded.
- All README command families work, JSON is pipe-safe, offline commands need no key.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` pass.
- No automatic harness changes. Validation uses public fixtures and simulated
  native stores exclusively; no personal session directories are inspected.
