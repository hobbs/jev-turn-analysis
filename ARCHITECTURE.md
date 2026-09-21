# Architecture

`jta` is a Rust CLI. Discovery, parsing, redaction, aggregation, validation, and
persistence run locally. Jev supplies categorical judgments; a selected coding
agent CLI investigates those judgments and proposes improvements.

## Analysis

`jta analyze` discovers native Codex and Claude Code sessions for the current
project, or imports explicit logs. Project identity is the canonical Git checkout
root (including worktrees), or the current directory outside Git. Source agent
identity stays `codex` or `claude_code`; sessions are not merged across agents.

Normalized sessions retain events, stable turn IDs, source lines, tool links,
resource coverage, and warnings. Transcript instructions are untrusted data.
Redaction precedes persistence and scoring. Revision hashes and configuration
fingerprints allow unchanged Jev analyses to be reused.

`services::prepare_analysis` constructs Jev requests; `score_session` validates
complete probability distributions before storing results. Short sessions share
context across question batches; long sessions use bounded outcome/turn packets.
`analytics` computes exact counts, coverage, confidence exclusions, and patterns.
These statistics never come from a report agent's prose.

## Report pipeline

1. `cli` loads the filtered scored cohort and computes full-corpus statistics.
2. `review_pipeline::project_patterns` splits rubric buckets by project before
   recurrence and priority selection. Default selection is five groups, with a
   configurable hard cap of twenty investigations.
3. `Plan::new` creates an initial sample for each selected project/category and
   supplies the full normalized project cohort for retrieval. `review_context`
   collects bounded, redacted current instruction, skill, reference, and source
   file snapshots. One project's file budget does not consume another's.
4. `review_pipeline::review` runs investigations sequentially. Each agent starts
   with samples, retrieves more staged evidence as needed, and returns findings
   and grounded proposals. Findings do not require an edit.
5. A synthesis invocation reconciles validated investigation results. It can
   inspect the same staged evidence to resolve contradictions and duplicate edits.
6. JTA validates the final result and saves proposals plus Markdown/JSON reports.
   Report generation never applies project edits.

Every agent invocation gets a temporary working directory containing:

- `initial.json`: category samples, or validated investigations for synthesis.
- `manifest.json`: session metadata and original file paths mapped to snapshots.
- `sessions/*.json`: redacted full normalized sessions and Jev judgments.
- `files/*.json`: redacted current project text with original paths and hashes.
- `schema.json`: the required final result schema.

Paths inside evidence are citation identities. Agents are instructed to read only
staged files. The runtime enforces read-only tool behavior; the staging directory
is not represented as a complete filesystem read sandbox. All evidence remains
untrusted data, including project instruction files.

## CLI adapters and authentication

`services/review_cli.rs` starts `codex exec` or `claude --print` directly using
Tokio processes, without shell interpolation. Codex uses a read-only sandbox and
ignores user configuration/rules. Claude uses safe mode, preserving login, with
only Read/Glob/Grep tools. Both disable persistent sessions and customizations for
these runs. Recent CLI versions supporting the documented flags are required.

JTA manages only the Jev key. The agent CLI owns its authentication. Configuration
contains `review.backend` (`codex` or `claude`), optional `review.model`,
`review.timeout_secs`, and `review.max_investigations`. Old HTTP review settings
migrate to CLI defaults on load; `init` persists migration. No review credential
is resolved, requested, or copied into JTA storage.

An invocation must exit successfully and return a complete structured result.
Event streams supply usage and an execution trace. Wall-clock time and captured
output are bounded; Unix process groups prevent orphan tool processes after
cancellation or timeout. JTA does not enforce a model-token or dollar budget.

## Validation and caching

The common schema includes summary, themes, findings, recommendations, inspected
turn references, and inspected file paths. References must exist in the same
project's staged evidence. Proposal targets and quotations must match current
snapshots exactly. Recurrence counts distinct sessions supporting each proposal.
All citations must appear in the agent's inspection record. Overlapping edits
are rejected. These checks establish traceability, not causal correctness;
inspection is self-reported.

Each stage gets at most one correction invocation after local validation failure.
Execution failures stop immediately. Successful stages are cached independently
against the evidence, files, configured CLI/model/budgets, CLI version, redaction,
prompt/schema, and pipeline version. Cache hits are revalidated. `--refresh`
forces new work; unpinned model alias changes otherwise cannot be detected.

`review_cache/` retains redacted event traces, usage, validated responses, and
source revisions. Reports separate fresh invocations from historical cached usage.
`purge` follows source revisions to remove dependent caches and reports.

## Storage and verification

Default storage is `~/.jta/projects/<name>-<canonical-path-hash>/`, relocated by
`JTA_HOME`. An explicit `--workspace DIR` uses `DIR/.jta` without changing native
discovery's project selection. Credentials are separate in `~/.jta/credentials.json`.
Persistence is atomic; old revision evidence stays available to snapshots/runs.

Tests use public fixtures, temporary simulated native stores, Jev HTTP mocks, and
fake Codex/Claude executables. They do not inspect personal session directories or
make paid model calls. Python 3 is needed for the fake CLI fixtures.

Required checks: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
and `cargo test --locked`. See `VALIDATION.md` for evidence and remaining limits.
