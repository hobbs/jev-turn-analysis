# Jev Turn Analysis

Understand what helped an agent finish a task, what got in its way, and what to improve in the harness next.

Jev Turn Analysis is a Rust CLI for analyzing completed Claude Code and Codex sessions. It turns session logs into outcome assessments, evidence-backed turn judgments, recurring patterns, and concrete proposals for improving `AGENTS.md`, skills, prompts, tools, and runtime behavior.

Task success and verification come first. Token and wall-clock efficiency come second. A long investigation can be essential; a short session can still leave the task unfinished.

> The Rust CLI implements the workflow below. Console transcripts and outcome numbers are illustrative, not measured accuracy claims. See compatibility notes under Implementation for current limits.

## Install `jta` once

From this repository's source checkout, with a current stable Rust toolchain:

```sh
cd /path/to/jev-turn-analysis
cargo install --path . --locked
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
jta --help
```

Cargo installs the `jta` executable into `${CARGO_HOME:-$HOME/.cargo}/bin`. Add
that `export PATH=...` line to your shell startup file (`~/.zshrc` for Zsh or
`~/.bashrc` for Bash), then open a new terminal. You can now run `jta` from any
codebase. To update after pulling source changes, run
`cargo install --path . --locked --force` from the source checkout.

If you already built `target/release/jta`, you can instead install that binary in
a stable user directory. This is useful when the Rust toolchain or `CARGO_HOME`
lives inside this checkout's `.tools/` directory:

```sh
cd /path/to/jev-turn-analysis
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/jta "$HOME/.local/bin/jta"
export PATH="$HOME/.local/bin:$PATH"
```

Persist this alternative PATH line in your shell startup file as well. The copied
binary runs independently of the source checkout; repeat the copy after rebuilding
to update it. See [implementation](#implementation) for the project-local build
commands.

## Analyze your current codebase

Run `jta init` to save `JEV_API_KEY` globally, or set it in your shell environment
or a `.env` file at the target project root before scoring. Then run:

```sh
cd /path/to/your/codebase

# Set up the Jev key and choose codex or claude for reports.
jta init

# Find and score this project's Codex and Claude Code sessions.
jta analyze

# See outcomes, verification gaps, and recurring patterns.
jta report
```

Both agents are included by default. Bare `jta analyze` discovers sessions for the
current project and creates storage under `~/.jta/projects/` if needed; no initial `jta init` or
manual transcript export is required. Use `jta analyze --dry-run --format json`
to inspect the redacted scoring payload before sending it. Restrict a source with
`jta analyze --agent codex` or `jta analyze --agent claude`; the same filters work
with `discover` and `report`.

The installed binary does **not** carry this source repository's `.env` with it.
Scoring and review load `.env` from the analysis workspace root, normally the target
codebase's root. With `--workspace`, that explicit workspace's `.env` is used.
Existing shell environment values take precedence. Use the source checkout's
`.env.example` as a template and add the needed keys to your project's existing
`.env`, or create one if absent. Keep `.env` out of version control; analysis data lives outside your checkout by default. Offline discovery, import, and previews need no key; analysis sends evidence to Jev.
Report investigations run through the selected `codex` or `claude` CLI.
Install and authenticate that CLI separately. JTA only manages Jev authentication.

Choose with `jta init --review-backend codex` (the default) or
`jta init --review-backend claude`. Supplied flags update existing configuration;
unspecified settings are preserved. Switching backends clears the optional model
override unless `--review-model` is also supplied. Run plain `jta init` in a
terminal for backend selection and hidden Jev key entry; use `--no-input` in scripts.
The Jev key is stored globally in `~/.jta/credentials.json` (or under `JTA_HOME`).
Shell and project `.env` values override the saved key. Keys stay out of project
config and reports; the credentials file uses user-only permissions on Unix.

```sh
# Follow a finding back to its turns and source events.
jta show s_014 --turn 18

# Preview selected evidence, then review with the configured LLM.
jta report --dry-run
jta report --top 5
jta recommendations
```

Jev supplies categorical judgments for `analyze`. The report CLI investigates
possible causes and proposes improvements. See [validation results](../VALIDATION.md) for checks performed.

## Start with the outcome

```console
$ jta analyze
Imported 20 sessions: 12 Claude Code, 8 Codex
Extracted 486 turns and their evidence
Scored 20 sessions with Jev

Task outcome          Sessions
complete                    12
partially_complete           4
failed                       2
abandoned                    1
unclear                      1

Outcome verification  Sessions
verified                    10
claimed_but_unverified        4
known_incomplete              5
unclear                      1

Highest-priority patterns
  p_01  Missing verification before completion claims    4 sessions
  p_02  Repeated retries after the same tool failure     3 sessions
  p_03  Early targeted tests helped guide implementation 5 sessions

Analysis saved as run_001.
Inspect a pattern: jta show p_01
View the report:   jta report
```

Completion and verification are separate judgments. An agent saying “done” is evidence of a claim; recorded checks and their relevance to the user’s request determine whether that claim is supported. When the transcript cannot establish the outcome, the result is `unclear`.

Reports show user corrections and redirections alongside outcomes. They include both behaviors to improve and effective behaviors to reinforce.

## Discovery scope and source filters

Discovery and analysis include both agents unless you select one. You can scope
later reports, baselines, and reviews independently:

```sh
jta discover --agent codex
jta analyze --agent claude
jta report --agent codex
jta snapshot codex-baseline --agent codex
jta report --agent claude --dry-run
```

`claude` and `claude_code` are equivalent filter names; stored source identity stays
`claude_code`. `--agent all` restores the default mixed-agent view. Sessions remain
separate across agents, even when their native IDs happen to match. Reports include
project and agent breakdowns with session/turn counts, outcomes, verification, and
resource coverage. Pattern evidence retains its source agent, and each pattern
counts distinct supporting sessions by agent. These are observed cohorts, not
causal rankings of agent quality.

The project is the nearest Git checkout root, including a worktree `.git` file, or
the current directory outside Git. Nested repositories and separate worktrees stay
separate. Symlinks are canonicalized and containment uses path components, so
`/work/app` does not match `/work/app-other`. The recorded session cwd remains in
`repository`; `project_root` separately records normalized project identity.
`--repo` matches the project root while retaining exact recorded-cwd matching for
older filters. Older stored sessions lacking `project_root` remain readable.

Use `--project` to select a project explicitly. `--workspace` chooses storage and
does not change the discovery project. Discovery reads Codex `sessions/` and
`archived_sessions/` beneath `CODEX_HOME` (default `~/.codex`), and Claude `projects/`
beneath `CLAUDE_CONFIG_DIR` (default `~/.claude`). These source locations follow the
[Codex log documentation](https://learn.chatgpt.com/docs/reference/troubleshooting#feedback-and-logs)
and [Claude session documentation](https://code.claude.com/docs/en/sessions).
Explicit source-root flags replace their source's default completely:

```sh
jta discover --project /work/payments \
  --codex-home /exports/codex --claude-config-dir /exports/claude
jta analyze --project /work/payments \
  --codex-home /exports/codex --claude-config-dir /exports/claude --dry-run
```

Discovery inspects at most 1 MiB or 256 records of metadata per file and fully
imports only selected sessions. It attributes each session using its first recorded
cwd, never Claude's encoded directory name alone, and separately counts missing
cwd, metadata limits, skipped files, and malformed records. Read errors are reported
separately. Removed historical child directories can still match an existing project
root; deleted repository boundaries cannot be reconstructed. Session-store symlinks
are not followed. Recognizable helper and subagent transcripts are excluded. `discover` makes no service calls and
requires no API key; `analyze --dry-run` can initialize local storage and import
redacted evidence but sends no request. Exported files and `analyze --all` remain
available below; native discovery flags apply only to no-path analysis.

## Bring your own session logs

`jta` accepts supported Claude Code and Codex session logs and detects their format. A directory is scanned recursively. It analyzes the supplied logs; it does not execute recorded commands or modify the repositories described in them.

```sh
jta analyze ./session.jsonl
jta analyze ./sessions/

# Import and inspect evidence before making any scoring request.
jta import ./sessions/
jta show s_014 --evidence
jta analyze --all
```

To try the bundled public examples instead of native discovery, explicitly import
them into your current workspace (storage is created automatically):

```sh
jta import /path/to/jev-turn-analysis/tests/fixtures/public
jta analyze --all --dry-run --format json > jev-preview.json
```

This preview makes no service request. After inspecting it, `jta analyze --all`
scores the imported corpus, including any sessions already in that workspace.

Imports receive stable session IDs. Importing the same content again does not duplicate it. If a source log changes, the new revision is recorded and its previous analyses remain available. Unchanged sessions reuse compatible cached analysis unless you pass `--refresh`.

Unsupported formats, malformed records, and missing context are reported with source locations. A damaged session is never silently treated as a complete one. Partial logs can be inspected, but analysis marks their evidence gaps and limits conclusions accordingly.

Every analysis records its source revisions, parser version, rubric, Jev configuration, and redaction settings so you can tell which results are comparable.

## Understand a turn in context

An analysis turn is one assistant response and its associated tool calls and results. User messages supply goals, corrections, and constraints. Source message boundaries remain available, and mechanically linked turns supply context for deeper review. Concurrent activity retains recorded timestamps and call-ID associations.

```console
$ jta show s_014 --turn 18
Session s_014 · turn 18 · gather_context
Source: sessions/codex-014.jsonl · records 92–97

Intent       Locate the existing retry policy
Action       rg 'retry|backoff' src/
Observed     Found src/client/retry.rs

Judgment                  Selected answer          Probability
Immediate effect          enabled_later_work              0.88
Downstream use            consumed_later                  0.92
Counterfactual necessity  loop_less_informed               0.81
Usefulness                useful                          0.90
Outcome contribution      helped_success                  0.86
Opportunity               none                            0.79
Remediation surface       none                            0.84

Later evidence
  t_23  Reads the retry policy found here
  t_27  Changes that policy
  t_31  Runs the relevant retry tests successfully

Session outcome: complete · verified
```

Turn evidence includes explicit intent, tool inputs, relevant output, files read or changed, failures, retries, timing, token usage, and recorded verification. Mechanical links identify candidate later references. Artifact survival and actual consumption remain unverified unless the evidence and semantic judgments support them.

The analyzer distinguishes immediate progress from eventual contribution. Finding a file that becomes useful five turns later should receive credit. An unsuccessful experiment can still reduce uncertainty or prevent a bad change.

For short sessions, Jev receives the normalized session and judges the turns with the final outcome in view. For long sessions, deterministic code builds evidence packets containing the goal, corrections, nearby turns, final state, verification, and mechanically linked later events. Evidence views expose what was included or omitted. Mechanical links are candidates for Jev to judge, not proof that a turn was useful.

## Find patterns across sessions

The report combines corpus statistics with proposed workflow changes and saves
them to one Markdown file. Standard output contains the generated file's path;
status messages on stderr explain preliminary or skipped reviews:

```console
$ jta report
Generated Markdown report: /home/you/.jta/projects/example-a1b2/reports/report_123.md
```

The file contains a summary, outcome tables, recurring patterns, tokens and timing
with coverage, agent and project breakdowns, uncertainty, and proposed edits with
supporting turns. Statistics use the full filtered corpus, while recommendations
use the selected sample.

Individual turn details are available through each pattern's supporting evidence. Filter or export the report for a closer look:

```sh
# Filter to a repository or a recent time window.
jta report --repo /work/payments --since 30d

# Inspect verification problems or likely wasted effort.
jta report --opportunity missed_verification
jta report --usefulness wasted --min-confidence 0.8

# Group opportunities by their likely remediation surface.
jta report --group-by remediation_surface

# Generate the report and get paths for scripts.
jta report --format json
# Output data contains report_path (Markdown) and data_path (exact JSON).
# Open report_path for sharing; read data_path for statistics and proposals.
```

Reports cover session outcomes, verification, user intervention, turn usefulness, functional roles, downstream use, opportunities, and remediation surfaces. You can group findings by repository, source agent, tool, workflow stage, or opportunity.

Counts, tokens, and time are shown with their coverage. Missing token or timing data is not treated as zero. Parallel tool durations are not added together and presented as elapsed session time. Resource use attributed to a `wasted` turn is observed cost, not a promise that all of it could have been saved.

Every aggregate can be traced to contributing sessions and turns. Reports show their sample sizes and uncertain or excluded judgments. There is no single session quality score and no assumption that fewer turns means better work.

## Review scope and reports

`jta report` selects five eligible **project × opportunity × remediation surface**
groups by default. Recurrence is checked within each project. Categories associated
with failed or partial tasks rank first, then verification gaps, then other
opportunities; distinct supporting sessions break ties. Categories are starting
points for investigation, not semantic clusters or causal explanations.

Use `--top N` or `--all` to expand selection, subject to the configured
`review.max_investigations` cap (default 20). The report discloses selected,
eligible, and omitted groups. A recurring proposal needs three distinct sessions
by default. If no group qualifies, review falls back to preliminary findings with
a one-session minimum. `--recurring-only` disables that fallback; `--session ID`
explicitly investigates a single session, even without a detected opportunity.

```sh
jta report --top 10
jta report --all
jta report --session s_014
jta report --recurring-only
jta report --refresh
jta report --dry-run --format json
```

One CLI investigation runs per selected group, followed by a synthesis invocation.
Initial samples contain up to `max(3, min_pattern_sessions)` supporting sessions
plus a successful comparison when available. Each investigation gets excerpts,
then can retrieve complete normalized sessions from the filtered project cohort
and inspect current project snapshots. Turn filters choose investigation leads;
other turns remain available as context and counterexamples. Statistics continue
to follow the report filters.

The agent investigates specific behaviors, seeks counterexamples, and returns
findings, uncertainty, and grounded proposals. Synthesis merges overlapping
findings, ranks recommendations, and reconciles proposed edits. Agents cannot
apply edits: Codex uses its read-only sandbox, and Claude gets only Read/Glob/Grep
tools. Customizations and persistent agent sessions are disabled for report runs.

A successful run saves Markdown and JSON companions under `reports/`. Stdout
contains the Markdown path, or `report_path` and `data_path` with `--format json`.
Progress goes to stderr. CLI failures and invalid results fail the command;
a successful empty recommendation list still retains findings and its explanation.
No eligible investigations produces a local statistics report without a CLI call.

## Evidence, validation, and caching

Each temporary investigation directory has `initial.json`, an indexed
`manifest.json`, session snapshots, and project-file snapshots. Original paths
are citation identities; agents are instructed to read the staged copies. JTA
applies its redaction rules to snapshots and saved outputs. Full normalized
session snapshots let the agent inspect events omitted by initial excerpts.

Automatic project discovery includes instructions, skills, rules, README files,
package manifests, Makefiles, and common source files (Rust, Python, JavaScript,
TypeScript, shell, Go, Ruby, Java, Kotlin, C/C++, C#, Swift, Markdown, and TOML).
Applicable ancestor instructions and explicitly referenced installed skills are
also included. Repeatable `--context FILE` arguments prioritize additional files;
explicit files are associated with each selected project.

Snapshots include at most 256 files and 8,388,608 characters per project. Files over
256 KiB are skipped. Discovery stops at depth 12 and skips dependencies, build
outputs, `.git`, `.jta`, and symlinks. Omitted files and characters are disclosed.
These are current snapshots, not historical versions; existing fixes may postdate
the sessions. The directory is staged evidence, not a complete filesystem read
sandbox. Project files are not modified.

```sh
jta report --context docs/harness.md --context src/server/orchestrator.ts --dry-run --format json
```

Findings require project-scoped session/turn citations and can stand without an
edit. Recommendations additionally require exact absolute file targets, unique
before passages and literal replacements (or complete new root instruction files),
exact project-file quotes, sufficient distinct-session support, scope, risk, and
an evaluation plan. All citations must occur in the agent's inspection record.
Overlapping edits fail validation. Inspection is self-reported; valid references
alone do not prove an interpretation is correct.

Each stage may receive one correction invocation. The rejected answer and local
validation error are provided alongside the same staged evidence. A second invalid
answer fails the report. Useful observations should remain findings when an edit
cannot be justified. No invalid recommendations are saved.

Validated stages are cached under `review_cache/` using staged evidence, project
content, CLI version, configured model, instructions/schema, and budgets. Cached
results are revalidated and incur zero new model invocations. `--refresh` bypasses
caches, including after an unpinned model alias changes. `purge` removes cache
entries and reports dependent on expired session revisions.

Default budgets are 600 seconds per invocation, 20 investigations per report, and
8 MiB of captured CLI output. Investigations run sequentially. These are not hard
token or dollar limits; every CLI invocation can include multiple model/tool calls.
The JSON companion records reported usage, elapsed times, cache status, and
coverage per stage, keeping cached historical usage separate from new execution.
Corpus token tables describe the original sessions, not report processing.

```sh
jta init --review-timeout-secs 300 --review-max-investigations 10
jta recommendations
jta show <recommendation-id>
```

## Check whether a change helped

Save a baseline, change the harness, then compare a later cohort:

```sh
jta snapshot before --since 30d --repo /work/payments

# After applying a proposal and collecting new sessions:
jta analyze ./new-sessions/
jta snapshot after --since 2026-10-01 --repo /work/payments
jta compare before after
```

Comparisons show completion, verification, user intervention, recurring opportunities, and resource use, with sample sizes and uncertainty. Task success and verification lead the comparison; efficiency improvements do not offset regressions in those measures.

Snapshots freeze the selected session revisions and analysis results. Comparisons flag overlapping cohorts, different scoring configurations, and changes in agent or task mix. An observed improvement is evidence to investigate, not proof that the harness change caused it.

For calibration, export a labeling set, add human judgments, and compare them with Jev:

```sh
jta labels export --sample 20 --output labels.jsonl
# Fill in the human-label fields in labels.jsonl.
jta labels import labels.jsonl
jta calibration
```

Human labels are stored separately from model judgments. Calibration reports agreement, disagreements, and probability calibration by question, including cases where useful exploration was marked as waste. Exported labeling sets include the context needed to judge outcomes and delayed contributions.

## Local data and remote processing

New project data lives in `~/.jta/projects/<project-name>-<path-hash>/`.
`JTA_HOME` overrides `~/.jta`. The canonical project path keeps worktrees and
same-named projects separate. Existing project-local `.jta` folders are not
searched. `--workspace DIR` explicitly uses `DIR/.jta` and its `.env`.
Without that override, credentials load from the target project's `.env`.
Both modes fall back to keys saved globally by `jta init`.

`analyze` and `import` initialize storage automatically. `init` is optional setup:
it asks which report CLI to use in an interactive terminal, shows Jev credential
readiness without printing keys, offers hidden key entry or saving existing
environment keys globally, and updates only supplied settings. JSON output,
redirected input, or `--no-input` disables questions. Jev defaults to
`https://api.typesafe.ai/v1/systemone`, model `jev-latest`, and `JEV_API_KEY`.
Override these with `--jev-endpoint`, `--jev-model`, and `--jev-api-key-env`.

Import, normalization, evidence extraction, dependency linking, and aggregation run locally using deterministic code. No generative LLM summarizes or enriches turns during that pipeline. Jev performs the first semantic pass, returning categories and their probabilities. Only `report` launches generative investigations through the selected CLI.

When configured services are remote, `analyze` sends scoring context to Jev; the report CLI sends the staged context it reads to its model service. Both commands support `--dry-run` to inspect scoring payloads or the report plan without launching remote work. There is no background analysis.

Redaction runs before evidence is persisted or sent to a service. Built-in secret detection and user-defined patterns are supported; no redactor can guarantee removal of every sensitive value. Stable placeholders preserve relationships between repeated values. Original source logs are not modified or copied into the workspace; retained evidence can still contain source code and personal data.

```sh
# Preview data that has passed through redaction.
jta analyze ./sessions/ --dry-run
jta show s_014 --evidence

# Preview and then delete stored data older than a retention window.
jta purge --older-than 30d --dry-run
jta purge --older-than 30d
```

Retention defaults to 90 days and can be set with `jta init --retention-days`. `purge` removes matching workspace evidence and derived results, including snapshot data that retains those records, and reports affected objects. It never deletes original logs. Local retention controls do not control a remote provider's retention policy.

## Judgment reference

Jev retains the full probability distribution for each categorical question. The CLI normally displays the selected answer and its probability; JSON output includes all alternatives. Probabilities are model estimates whose reliability can be checked against human labels.

If Jev selects an option below the highest reported probability, analysis preserves
the original answer and records a warning with the question and conflicting
probabilities. Affected turns are also marked inconsistent for review.

Session questions:

| Question | Answers |
| --- | --- |
| Task outcome | `complete`, `partially_complete`, `failed`, `abandoned`, `unclear` |
| Outcome verification | `verified`, `claimed_but_unverified`, `known_incomplete`, `unclear` |
| User intervention | `none`, `clarification_only`, `corrected_agent`, `redirected_approach`, `unclear` |

Turn questions:

| Question | Answers |
| --- | --- |
| Functional role | `orient`, `gather_context`, `plan`, `execute`, `verify`, `recover`, `clarify`, `communicate`, `coordinate_or_wait`, `other` |
| Immediate effect | `advanced`, `enabled_later_work`, `no_observable_progress`, `regressed`, `unclear` |
| Downstream use | `consumed_immediately`, `consumed_later`, `not_consumed`, `superseded`, `reverted`, `unclear` |
| Counterfactual necessity | `outcome_worse_or_impossible`, `loop_less_informed`, `no_material_difference`, `outcome_improves`, `unknowable` |
| Overall usefulness | `essential`, `useful`, `neutral`, `wasted`, `harmful` |
| Outcome contribution | `helped_success`, `reduced_risk`, `hindered_success`, `increased_risk`, `no_material_effect`, `unclear` |
| Improvement opportunity | `redundant_work`, `missing_or_poorly_selected_context`, `wrong_or_ineffective_tool_use`, `repeated_failed_approach`, `planning_or_sequencing_problem`, `missed_verification`, `instruction_conflict_or_ambiguity`, `missing_capability`, `effective_behavior_to_reinforce`, `environment_or_tool_limitation`, `other`, `none` |
| Remediation surface | `AGENTS.md`, `skill`, `prompt`, `tool_description`, `tool_implementation`, `orchestration_or_runtime`, `context_packaging`, `missing_capability`, `other`, `none` |

Opportunity judgments include a primary category and optional secondary categories. Downstream judgments can select a supporting turn from mechanically generated candidates, or `none` when no candidate is supported.

Disagreement between usefulness, immediate effect, downstream use, and counterfactual necessity is surfaced as uncertainty. Such turns can be selected for more context or deeper review and excluded from confident aggregates. A confidence filter applies to the questions used in that report and reports how many records it excludes.

## Command reference

| Command | Purpose |
| --- | --- |
| `jta init` | Create a workspace and configure destinations, redaction, and retention. |
| `jta import <path>` | Normalize logs and extract evidence locally. |
| `jta discover` | Preview matching native project sessions offline. |
| `jta analyze` / `jta analyze <path>` / `jta analyze --all` | Discover project sessions or import supplied logs, score with Jev, and aggregate patterns. |
| `jta report` | Save full corpus statistics and LLM recommendations to Markdown; print the path and review status. |
| `jta show <id>` | Inspect a session, pattern, recommendation, or analysis run. |
| `jta recommendations` | List evidence-backed harness proposals. |
| `jta snapshot <name>` | Freeze a cohort and its analysis for comparison. |
| `jta compare <before> <after>` | Compare two saved cohorts. |
| `jta labels export` / `jta labels import` | Exchange human judgments for calibration. |
| `jta calibration` | Evaluate Jev judgments against human labels. |
| `jta purge` | Remove retained workspace data. |

All commands support `--help`. Data-producing commands support `--format json` with a versioned schema; human-readable output is the default. Progress and diagnostics go to stderr so stdout can be piped safely. Corpus reports use the latest successful analysis of each current session revision and flag mixed analysis configurations; `--run <id>` selects a specific run.

Long runs checkpoint completed work. Rerunning a command resumes unfinished work and reuses valid cached results. Individual import or service failures are reported by session, with partial results available for inspection.

Exit codes describe command execution: `0` for success, `1` for an operational failure or incomplete batch, and `2` for invalid arguments or configuration. A successfully analyzed session with a failed task outcome still returns `0`.

## Implementation

For development with the project-local toolchain installed in this checkout:

```sh
export CARGO_HOME="$PWD/.tools/cargo"
export RUSTUP_HOME="$PWD/.tools/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
cargo build --release --locked
./target/release/jta --help
```

Run these from the source checkout. To make the resulting binary available in
other codebases, follow [the installation steps above](#install-jta-once).

The default Jev endpoint is `https://api.typesafe.ai/v1/systemone`, model
`jev-latest`, with bearer authentication from `JEV_API_KEY`. Set
`--jev-api-key-env TYPESAFE_API_KEY` at initialization if using that environment
variable name instead. Credentials come from the shell environment, then the
analysis workspace's `.env`, then `~/.jta/credentials.json`; they are never stored
in workspace configuration.

`jta init` writes `config.json` in the project data directory and updates supplied settings.
The configuration is editable JSON with these defaults:

```json
{
  "schema_version": 1,
  "jev": {
    "endpoint": "https://api.typesafe.ai/v1/systemone",
    "api_key_env": "JEV_API_KEY",
    "model": "jev-latest",
    "max_context_chars": 48000,
    "max_questions": 64,
    "timeout_secs": 120
  },
  "review": {
    "backend": "codex",
    "model": null,
    "timeout_secs": 600,
    "max_investigations": 20
  },
  "redaction": {"enabled": true, "patterns": []},
  "retention_days": 90,
  "min_pattern_sessions": 3
}
```

Configure reports with `jta init --review-backend codex` or `claude`.
`--review-model` optionally overrides the CLI model; an empty value clears it.
Use `--review-timeout-secs` and `--review-max-investigations` to change budgets.
Only Jev has endpoint and API-key-variable settings. Older HTTP review settings
are ignored when loading configuration and replaced with Codex defaults;
`jta init` persists the migrated configuration. Retired review credentials are
never resolved or requested. Existing credential files are not deleted.

Custom redaction patterns are regular expressions. Matching values use stable hashed
placeholders. Review the dry-run payload before sharing confidential transcripts;
automatic redaction cannot identify every secret or sensitive passage.

### Supported inputs and evidence limits

| Input | Supported records | Validation |
| --- | --- | --- |
| Claude Code native JSONL | User/assistant text, thinking, tool use/results, usage, timestamps, metadata | Four public Trace Commons sessions; assistant message counts and usage checked |
| Codex native JSONL | `session_meta`, `response_item` messages/calls/results/reasoning, `event_msg`, `turn_context` | Synthetic regression cases and a public native transcript |
| share-codex JSON session envelope | `id`, `messages`, roles, tool calls/results | Three complete public exported sessions |

Native discovery scans metadata under the configured source roots only when
`discover` or no-path `analyze` is invoked. Explicit directory import
skips `.git`, `.jta`, `.tools`, `target`, and fixture provenance metadata. Unknown
records, damaged lines, missing results, and orphan results are reported. A result
without a call ID cannot be reliably associated with concurrent work and remains
unlinked. Exports identify source message ordinals when original line numbers are
unavailable. Content hashes deduplicate identical input; native session identifiers
preserve source revisions. Parser/redaction changes create distinct evidence revisions.
Parser version 2 namespaces native session IDs by source agent to prevent cross-agent
collisions. Reimporting evidence originally stored by parser version 1 can therefore
create a new session identity. When the source path and agent match an older-parser
current session, the new import retires that old current pointer to avoid duplicate
corpus counts. Archived revisions, analyses, runs, and snapshots remain available.
Moved source paths are not automatically reconciled with old identities.

Assistant message IDs join Claude streaming blocks. Tool-result call IDs preserve
associations across interleaving. File evidence records explicit path arguments and
patch paths; arbitrary shell commands are not a complete filesystem change log.
Downstream links are exact shared path, URL, error identifier, or command candidates
with visible reasons. References are extracted from shell commands and recorded output
as well as explicit tool path arguments; references are capped at 128 per turn and
downstream links at 64. These candidates are
not proof of consumption. Edit calls alone do not establish artifact survival,
reversion, or successful writes. Missing timing/token measurements remain unknown.
Redaction happens before evidence persistence and request construction.

The fixtures are public data, not human-labeled usefulness benchmarks. No measured
classification accuracy or causal improvement claim is made. Dataset source revisions,
licenses, and SHA-256 checksums are recorded in
[`tests/fixtures/public/PROVENANCE.json`](../tests/fixtures/public/PROVENANCE.json).
Reproduce or verify the pinned samples with:

```sh
python3 scripts/download_examples.py
python3 scripts/download_examples.py --verify-only
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Fixtures retain their original dataset/source licenses; see provenance before
redistributing embedded source code. Tests use local mock HTTP servers and do not
require credentials or spend API credits.

Detailed service contracts and validation rules: [docs/services.md](services.md).
