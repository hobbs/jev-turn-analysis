# Jev Turn Analysis

**Analyze and improve your coding agent harness.**

`jta` reads your Claude Code and Codex sessions, finds where work went well or got
stuck, and suggests specific changes to your agent instructions, skills, and tools.
Start with whether the task succeeded and was checked. Then look at wasted effort.

## Get started: two commands

Install from this checkout with a stable Rust toolchain:

```sh
cargo install --path . --locked
```

With `~/.cargo/bin` on your `PATH`, open a project where you use Codex or Claude
Code and run:

```sh
jta init
jta analyze
```

**`jta init` gets you ready.** It walks you through the Jev API key and a report CLI
(`codex` or `claude`). Key entry is hidden, and saved keys work across all your projects.

**`jta analyze` does the work.** It finds your project's sessions, scores them
with Jev, and shows what succeeded, what was verified, and where to investigate.
Each session gets a live progress bar; completed sessions leave a short result
line. No transcript export or separate import step is needed.

Run `jta analyze` again whenever you have new sessions. Unchanged results are
reused automatically, with cached sessions clearly marked.

## Generate the full report

After analysis, turn recurring patterns into suggested edits to `AGENTS.md`,
skills, and tools:

```sh
jta report
```

Choose a report CLI during `jta init`, or set it explicitly:

```sh
jta init --review-backend codex            # Default
jta init --review-backend claude
```

Install and authenticate the selected CLI separately. **JTA manages only the Jev
API key.** Report generation uses the CLI's own authentication, with no review
API key or HTTP endpoint in JTA configuration.

`jta report` saves a **Markdown report** and a JSON companion with full-corpus
statistics, findings, proposed edits, and investigation coverage. It prints the
file path to stdout and investigation progress to stderr. Suggested edits are
never applied automatically.

Report generation has three stages:

1. JTA groups opportunities by **project × category × remediation surface**,
   computes exact statistics, and chooses the five highest-priority eligible groups.
2. The selected CLI investigates each group separately. It starts with samples,
   then can read additional staged sessions and project files, test hypotheses,
   and look for counterexamples. Findings remain useful even without a proposed edit.
3. A final CLI invocation reconciles findings, merges overlapping proposals, and
   ranks recommendations. JTA checks citations, exact file quotations, edit
   locations, distinct-session support, and conflicting edits before saving.

A recurring proposal needs three distinct supporting sessions by default, within
one project. If no group qualifies, the default performs a preliminary review
with a one-session minimum. `--recurring-only` disables that fallback.

```sh
jta report --top 10                        # Select more project/category groups
jta report --all                           # All eligible groups, within the budget
jta report --session <session-id>           # Investigate one session
jta report --recurring-only                 # Require recurring support
jta report --refresh                        # Bypass investigation and synthesis caches
jta report --context src/orchestrator.ts    # Include an explicit project file
jta report --dry-run --format json          # Inspect selection and contract; no CLI launched
```

Initial samples contain up to `max(3, min_pattern_sessions)` supporting sessions,
plus an additional successful session when available. Agents can retrieve other
sessions from the filtered project cohort. The report distinguishes initial
sampling, available evidence, and agent-reported inspection. Broad rubric groups
are investigation starting points, not proven semantic clusters.

## LLM usage and budgets

Full-corpus statistics are computed locally. A fresh report with five selected
groups normally runs **five CLI investigations plus one synthesis invocation**.
Each CLI invocation can contain several model/tool turns. A validation failure
allows one correction invocation per stage; transport failures are not retried by
JTA. Invalid results fail the command instead of producing a misleading report.

Defaults limit each invocation to **600 seconds**, cap selection at **20
investigations**, and cap captured CLI output at 8 MiB. These are wall-clock,
selection, and output-size limits, **not hard model-token or dollar budgets**.
Configure them with:

```sh
jta init --review-timeout-secs 300 --review-max-investigations 10
jta init --review-model <cli-model-name>     # Optional; otherwise the CLI default
```

Validated stages are cached against evidence, project snapshots, CLI version,
backend/model settings, output schema, and instructions. `--refresh` forces a
new investigation, including after an unpinned model alias changes. Successful
stages remain reusable if a later stage fails. Cached results are revalidated.

The JSON companion preserves CLI-reported usage, elapsed time, cache status, and
inspection references per stage. Claude cost values, when returned, are estimates.
Cached usage belongs to the original execution. **Corpus token tables describe
the original coding sessions**, not JTA's processing cost.

Jev scoring in `analyze` is unchanged and reuses cached session analyses. See the
[usage assessment](docs/review-assessment.md) for scoring scale, report budgets,
and remaining limitations.

## Know what's happening

Analysis shows the session count, cached results, and planned Jev requests before
scoring. While it runs, the terminal shows:

- A spinner and progress bar for the current session, advancing as batches finish.
- Elapsed time, the current API call, and retry status when needed.
- Overall progress through the selected sessions.
- A short result line for each session: scored, cached, or failed.

Failed sessions stay visible while the remaining sessions continue. The final
summary includes actual API calls and task outcomes. Report generation shows investigation and synthesis progress;
its saved report includes CLI invocation counts, cache status, coverage, and review notes.

Large quantities use readable units such as `3.9B`. The saved JSON companion
preserves exact values; `--format json` returns
`report_path` and `data_path`, without printing either file's contents.
Progress goes to stderr. Redirected logs use plain lines instead of animations,
so piping output stays straightforward:

```sh
jta report                                # Outcomes, patterns, tokens, and timing
jta report --agent codex                   # Focus on one agent
jta report --format json                   # Return Markdown and JSON file paths
jta recommendations                       # Read saved suggestions again
jta show <session-id> --turn 18             # Follow a finding back to the transcript
```

## Your checkout stays clean

Configuration, imported sessions, cached scores, and reports live in:

```text
~/.jta/projects/<project-name>-<path-hash>/
```

Subdirectories of a Git checkout share its project storage; separate worktrees
and projects with the same name stay separate. `JTA_HOME` changes the base data
directory. Use `jta init --no-input` to see the exact location.

Project-local `.jta` folders are not automatically used. For explicit storage,
`jta --workspace /path/to/storage analyze` uses `/path/to/storage/.jta`;
discovery still targets your current project. This override also loads credentials
from `/path/to/storage/.env`.

## Jev authentication: set up once

Run `jta init` to save the Jev key in
`~/.jta/credentials.json`. It works across all your projects. Keys are kept
separate from project configuration and never included in reports or setup JSON.
The file is unencrypted and readable/writable only by your user on macOS/Linux
(`0600`). `JTA_HOME` relocates both credentials and project data.

Credential precedence is: **shell environment → project `.env` → saved global key**.
Custom `--jev-api-key-env` names also identify entries
in the global file. Rerun `init` to replace a saved key, or remove its entry from
`credentials.json` to forget it. Setup never sends a key to a provider to test it.
`--no-input`, JSON output, and redirected input never prompt or save credentials.

## What leaves your machine?

Discovery, import, and reading saved reports run locally. `analyze` sends redacted
session context to Jev; `report` lets the selected agent CLI read staged, redacted session and project
file snapshots. The CLI sends the material it reads to its configured model service.
Both offer `--dry-run --format json` to inspect scoring payloads or the report plan before execution. `jta discover` lists matching sessions entirely offline. Redaction helps remove secrets, but inspect sensitive logs before sending.
Original session logs are never changed.

You can also analyze exported logs with `jta analyze ./sessions/`, filter by agent,
compare before-and-after cohorts, and remove retained data with `jta purge`.

## Go deeper

- [Full guide](docs/guide.md): discovery, filters, configuration, comparisons, and command reference.
- [Service contracts](docs/services.md): payloads, validation, and retry behavior.
- [Validation results](VALIDATION.md): what has been checked and current limitations.
- [Architecture](ARCHITECTURE.md): how parsing, scoring, and review fit together.

For an offline sample, run `jta analyze tests/fixtures/public --dry-run` from this
checkout. The [public fixtures](tests/fixtures/public/README.md) include provenance
and licensing details.

Development checks require Python 3 for fake CLI fixtures, and no API credentials:

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```
