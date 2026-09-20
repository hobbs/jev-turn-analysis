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

Set `JEV_API_KEY` in your shell environment or in a `.env` file at the target
project root before scoring. Then run:

```sh
cd /path/to/your/codebase

# List matching sessions locally; no API key or network request required.
jta discover

# Import and score this project's Codex and Claude Code sessions.
jta analyze

# See outcomes, verification gaps, and recurring patterns.
jta report
```

Both agents are included by default. Bare `jta analyze` discovers sessions for the
current project and creates `.jta/` at its root if needed; no initial `jta init` or
manual transcript export is required. Use `jta analyze --dry-run --format json`
to inspect the redacted scoring payload before sending it. Restrict a source with
`jta analyze --agent codex` or `jta analyze --agent claude`; the same filters work
with `discover` and `report`.

The installed binary does **not** carry this source repository's `.env` with it.
Scoring and review load `.env` from the analysis workspace root, normally the target
codebase's root. With `--workspace`, that explicit workspace's `.env` is used.
Existing shell environment values take precedence. Use the source checkout's
`.env.example` as a template and add the needed keys to your project's existing
`.env`, or create one if absent. Keep `.env` and `.jta/` out of version control. Offline discovery, import, reports, and previews
need no key; analysis sends evidence to Jev.

Optional generative LLM review uses a separate credential: `OPENAI_API_KEY` for
OpenAI or `OPENROUTER_API_KEY` for OpenRouter. To
enable it, run `jta init --review-provider openai` (or `openrouter`) in your target
project **before its first analysis**. `init` does not overwrite existing
configuration: if analysis already created `.jta/config.json`, edit its `review`
settings instead. See [configuration defaults](#implementation).

```sh
# Follow a finding back to its turns and source events.
jta show s_014 --turn 18

# Preview selected evidence, then review with the configured LLM.
jta review --dry-run
jta review --top 5
jta recommendations
```

You get useful diagnostics from `analyze` and `report` without a generative LLM.
Jev supplies bounded semantic judgments; optional review explains causes and
proposes improvements. See [validation results](VALIDATION.md) for checks performed.

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
jta review --agent claude --dry-run
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
them into your current workspace (run `jta init` first if it has none):

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

The default report shows whether tasks succeeded, where tokens and wall-clock time went, and which patterns deserve attention:

```console
$ jta report

20 sessions · 486 turns · Sep 14–20, 2026
12 Claude Code · 8 Codex · 2 repositories

OUTCOMES
  Complete             12   ████████████
  Partially complete    4   ████
  Failed                2   ██
  Abandoned             1   █
  Unclear               1   █

  10 of 12 complete tasks had recorded verification.
   4 sessions included completion claims without verification.
   6 sessions required user correction or redirection.

TURN USEFULNESS
  Essential            91   19%
  Useful              248   51%
  Neutral              70   14%
  Wasted               62   13%
  Harmful              15    3%

  42 turns have uncertain or conflicting judgments.
  Counts above include their most likely labels.

TOKENS & WALL-CLOCK TIME
  Input tokens        2.10M   includes 1.40M cached tokens
  Output tokens       0.30M
  Session time        5h 42m  summed elapsed time across sessions
  Median session        14m

  Turns labeled wasted consumed 180K tokens.
  Observed cost; potential savings require further review.
  Coverage: tokens 18/20 sessions · timing 20/20 sessions

PATTERNS TO INVESTIGATE
  ID     Pattern                          Sessions  Likely surface
  p_01   Completion without verification      4      AGENTS.md
  p_02   Repeating the same failed approach   3      tool_description
  p_04   Re-reading unchanged context        6      context_packaging

BEHAVIORS TO REINFORCE
  p_03   Early targeted tests guided fixes    5      skill
  p_05   Checking constraints before edits    4      AGENTS.md

NEXT
  jta show p_01          Inspect supporting turns and outcomes
  jta review --dry-run   Preview evidence for deeper LLM review

No LLM review yet. Patterns are candidates, not recommendations.
```

Individual turn details are available through each pattern's supporting evidence. Filter or export the report for a closer look:

```sh
# Filter to a repository or a recent time window.
jta report --repo /work/payments --since 30d

# Inspect verification problems or likely wasted effort.
jta report --opportunity missed_verification
jta report --usefulness wasted --min-confidence 0.8

# Group opportunities by their likely remediation surface.
jta report --group-by remediation_surface

# Export a report for scripts or sharing.
jta report --format json > report.json
jta report --format markdown > report.md
```

Reports cover session outcomes, verification, user intervention, turn usefulness, functional roles, downstream use, opportunities, and remediation surfaces. You can group findings by repository, source agent, tool, workflow stage, or opportunity.

Counts, tokens, and time are shown with their coverage. Missing token or timing data is not treated as zero. Parallel tool durations are not added together and presented as elapsed session time. Resource use attributed to a `wasted` turn is observed cost, not a promise that all of it could have been saved.

Every aggregate can be traced to contributing sessions and turns. Reports show their sample sizes and uncertain or excluded judgments. There is no single session quality score and no assumption that fewer turns means better work.

## Turn repeated evidence into proposals

```console
$ jta review --dry-run
Selected for review
  p_01  Missing verification       4 sessions · 11 turns
  p_02  Repeated failed retries    3 sessions · 16 turns
  p_03  Effective targeted tests   5 sessions · 14 turns

Priority: failed or partial tasks, then correctness and verification
gaps, then avoidable effort in successful tasks.

LLM destination: configured review provider
Payload: redacted excerpts, linked context, aggregate evidence, and project files
No request sent.
```

Initial pattern candidates come from Jev categories and deterministic grouping. LLM review can connect related candidates, inspect causes, and propose a narrowly scoped change. It receives the relevant source context as well as aggregate evidence.

Review produces only project-grounded proposals for the coding agent's instructions,
skills, tools, and orchestration. It reads current `AGENTS.md`, `AGENT.md`,
`CLAUDE.md`, instruction overrides, Markdown rules, and `SKILL.md` files within the
selected projects, plus ancestor instructions. It includes installed `SKILL.md`
files referenced by absolute path in selected sessions, limited to the configured
Codex/Claude skills and plugins directories and `~/.agents/skills`. Aliased skill
catalog entries alone are not resolved. Project READMEs, package manifests, and
Makefiles provide concrete commands and workflow context.

Use repeatable `--context <file>` options for additional skills, tool definitions,
or implementation files (paths are relative to the invocation directory). Explicit
files are included for each selected project; use `--repo` to limit a multi-project
workspace. Review does not load arbitrary source code by default.

```sh
jta review --context docs/harness.md --context src/server/orchestrator.ts --dry-run --format json
```

Every proposal must name an exact project and file, provide a literal before/after
edit, and cite both session turns and exact passages from inspected project files.
Paths, existing passages, and quotations are validated before saving. Missing root
`AGENTS.md` or `CLAUDE.md` files are explicit creation candidates; proposed new
contents still require project-file evidence. A category-only suggestion such as
"add a verification gate" is not accepted. If the evidence cannot support a
project-specific edit, review returns no recommendations.
If a response fails grounding validation, review makes at most one correction
request with the validation error and checks the corrected response again. An
invalid result is never saved; correction can incur a second provider request.

Snapshots contain at most 64 files, 16,000 characters per file, and 96,000 characters
in total across selected projects. Discovery stops at depth 12, skips dependencies,
build outputs and `.jta`, and does not follow project symlinks. Unreadable files,
files above 256 KiB, and truncation are disclosed in the preview. Files are freshly
redacted before sending. These are current files, not reconstructed historical
versions; the prompt instructs review to account for fixes already present.

```console
$ jta recommendations
r_01  Add the deck renderer to the export skill's completion check

Project: /work/deck-studio

Edit: /work/deck-studio/.claude/skills/export/SKILL.md

Replace this passage:

    Run npm test before handing off the deck.

With:

    Run npm test before handing off the deck. For export changes, run
    npx tsx scripts/render-deck.ts .generated/<presentation-id>.pptx
    and inspect the slide PNGs in .artifacts/qa/ for required visible labels.
    Report the deck ID and any missing labels; do not report those as verified.

Why here: This skill owns deck handoff, but currently requires only unit tests.

File evidence (/work/deck-studio/README.md): npx tsx scripts/render-deck.ts .generated/<presentation-id>.pptx

Surface: skill

Proposed change: Extend the export skill's existing npm test instruction with the
project's render-deck command and a visible-label check.

Applies to: Changes affecting exported artifacts.

Observed behavior: Completion claims cited builds without an artifact check.

Effect: Users had to discover unusable exports after handoff.

How to evaluate: Replay representative export tasks. Every export-success claim
must link an artifact that opens; track task completion and verification time.

Risk: Extra export latency.

Uncertainty: Excerpts may omit checks performed elsewhere.

Evidence:
  - jta show s_014 --turn 18
  - jta show s_015 --turn 7
  - jta show s_016 --turn 12

$ jta show r_01
```

Each recommendation includes:

- The observed pattern and its effect on task outcomes.
- Supporting session and turn references, uncertainty, and counterexamples.
- The proposed remediation surface and concrete suggested wording or behavior.
- Exact target paths, literal edits or new-file contents, and project-file citations.
- The scope where it should apply and the risk of applying it too broadly.
- A way to evaluate whether the change improves future sessions.

`jta review` and `jta recommendations` display these details in text and Markdown.
Older saved reviews without concrete file targets are flagged and omitted from
the guidance; rerun `jta review` to generate grounded replacements. The old records
remain available through `jta show <recommendation-id>` and `--format json`, which
includes the full stored objects and source evidence. No proposed edits are applied.

By default, harness recommendations require support from at least three distinct sessions. A serious isolated failure still appears in diagnostics and can be reviewed directly with `jta review --session s_014`; it is marked as an isolated finding. The recurrence threshold is configurable.

The review prompt includes this threshold for each proposal. Proposals citing too
few distinct sessions are skipped with a warning; qualifying recommendations are
still saved. If none qualify, use an isolated session review or gather more evidence.

Recommendations remain proposals. `jta` never automatically edits `AGENTS.md`, installs skills, or changes runtime policy. You decide which changes to apply.

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

`jta init` creates `.jta/` at the current project root for configuration, evidence, cached judgments, and reports, or at the directory supplied with `--workspace`. Commands use the nearest enclosing workspace within the current Git project boundary, or the explicitly supplied workspace. No-path `analyze` creates a workspace at the discovered project root when needed; `discover` itself does not initialize storage. Analysis data should be excluded from version control.

Initialization is noninteractive. It defaults to the Jev endpoint `https://api.typesafe.ai/v1/systemone`, model `jev-latest`, and credential variable `JEV_API_KEY`; override these with `--jev-endpoint`, `--jev-model`, and `--jev-api-key-env`. Optional generative review is configured separately with `--review-provider` and related flags. Credentials are read from the environment or local `.env` and are not stored in reports or workspace configuration.

Import, normalization, evidence extraction, dependency linking, and aggregation run locally using deterministic code. No generative LLM summarizes or enriches turns during that pipeline. Jev performs the first semantic pass, returning categories and their probabilities. Only `review` invokes a generative LLM.

When configured services are remote, `analyze` sends scoring context to Jev and `review` sends selected context to the LLM provider. Both commands support `--dry-run` to inspect the destination and prepared payload before sending it. There is no background upload or automatic LLM review.

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
| `jta report` | Summarize and filter the analyzed corpus. |
| `jta show <id>` | Inspect a session, pattern, recommendation, or analysis run. |
| `jta review` | Use an LLM to investigate selected evidence and propose improvements. |
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
variable name instead. Credentials come from the shell environment or the analysis
workspace's `.env`; they are never stored in workspace configuration.

`jta init` writes `.jta/config.json` without replacing existing configuration.
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
    "provider": "none",
    "endpoint": "https://api.openai.com/v1/chat/completions",
    "api_key_env": "OPENAI_API_KEY",
    "model": "gpt-5.6-terra",
    "timeout_secs": 120
  },
  "redaction": {"enabled": true, "patterns": []},
  "retention_days": 90,
  "min_pattern_sessions": 3
}
```

For optional review, initialize a new workspace with `--review-provider openai`,
or `--review-provider openrouter` (defaults to `OPENROUTER_API_KEY` and model
`openai/gpt-4.1`). For an existing workspace, edit the `review` object in
`.jta/config.json`; rerunning `init` does not change it. Endpoint, model, and
key-variable flags can override defaults when initializing.
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
[`tests/fixtures/public/PROVENANCE.json`](tests/fixtures/public/PROVENANCE.json).
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

Detailed service contracts and validation rules: [docs/services.md](docs/services.md).
