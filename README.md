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

**`jta init` gets you ready.** It walks you through API keys and an optional review
provider. Key entry is hidden, and saved keys work across all your projects.

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

Choose OpenAI or OpenRouter during `jta init` to enable review. If you skipped
that choice, rerun `jta init` when you're ready.

`jta report` saves one **Markdown report** with outcome tables, recurring patterns,
token and timing coverage, agent/project breakdowns, and recommendations with
proposed wording, tradeoffs, and ways to check the change. It prints only the
path of the generated file to stdout. Status messages on stderr explain preliminary
reviews or why recommendations were unavailable.
Suggested edits are never applied automatically.

Statistics cover the full filtered corpus; recommendations use a smaller sample.
If no provider is configured or no project-specific edits are supported,
the report still includes the statistics and explains what is missing. When the
model returns no suggestions, its explanation and observations are saved in the
report instead of being replaced by a generic status message. Provider
errors fail the command rather than claiming that recommendations were generated.
Recommendation prose uses the bundled [Humanizer skill](third_party/humanizer/SOURCE.md).

By default, review selects **five priority patterns**, each supported by at least
three sessions. If none meet that threshold, it reviews the available patterns
and labels suggestions as **preliminary**, with limited session support. Use
`--recurring-only` to skip this fallback. The report states how many patterns were
selected and how many are available.
The limit restricts the sample; it is not a token budget or a limit of five API
calls or five recommendations.

```sh
jta report --top 10                        # Expand the pattern selection
jta report --all                           # Include every eligible pattern
jta report --session <session-id>          # Investigate a single session
jta report --recurring-only               # Require recurring patterns
jta report --dry-run                       # Preview without calling the provider
```

`--all` still samples sessions and bounds excerpts. These patterns are broad rubric
categories, not semantic issue clusters. See the [review assessment](docs/review-assessment.md)
for prioritization, LLM token usage, and a 300,000-turn scaling check. Review needs readable project
files to propose a concrete edit. If it cannot support a suggestion, the report
explains that instead of filling space with generic advice.

## LLM usage on large corpora

For **3,000 sessions with 100 turns each**, the two stages have different costs:

| Stage | What is sent | Scaling behavior |
| --- | --- | --- |
| `analyze` | Context and questions for every uncached session and turn | Work grows with the corpus; unchanged analyses are reused |
| `report` statistics | Nothing; aggregation and Markdown formatting are local | No model tokens |
| `report` recommendations | Sampled excerpts, judgments, project files, and the writing prompt | Defaults select up to five categories and three sessions per category, at most 15 distinct sessions |

A fresh analysis asks **2,003 questions per 100-turn session**. At the default
64-question batch limit, that means **96,000 Jev requests** across 3,000 sessions
if each full session fits the 48,000-character context threshold. Larger sessions
use separate outcome and per-turn packets: **303,000 requests**. Both counts
exclude retries. Each batch repeats context and question instructions, so token
usage can substantially exceed the size of the original transcripts.

Report generation does **not** send all 300,000 turns to the recommendation
model. But it also has **no total input-token budget**. As an illustration, if
each sampled session contributes 10,000 tokens, 15 sessions contribute 150,000
input tokens before the system prompt, response schema, and project files.
This is an assumption for planning, not a measured token count. `--all` can select
more sessions and still sends one request. The 6,000-token completion limit applies
to the entire response, not to each issue.

The bundled Humanizer adds 28,696 characters to each request. A validation
correction resends the original prompt plus the rejected answer; transport retries
can also repeat requests. Report recommendations are not cached, so generating
the same report again repeats the model work. `--dry-run --format json` exposes
the payload without calling a provider, for inspection with that model's tokenizer.

**The report's token tables describe the original agent sessions.** JTA currently
records recommendation HTTP attempts, but does not retain the provider's token
usage or calculate its bill. Dollar cost depends on the configured model, actual
tokenization, output length, and any provider caching. See the
[full usage assessment](docs/review-assessment.md#llm-token-usage-at-3000-sessions)
for request arithmetic and the proposed token budgets and caching changes.

## Know what's happening

Analysis shows the session count, cached results, and planned Jev requests before
scoring. While it runs, the terminal shows:

- A spinner and progress bar for the current session, advancing as batches finish.
- Elapsed time, the current API call, and retry status when needed.
- Overall progress through the selected sessions.
- A short result line for each session: scored, cached, or failed.

Failed sessions stay visible while the remaining sessions continue. The final
summary includes actual API calls and task outcomes. Report generation is quiet;
its saved report includes provider attempt counts and review notes.

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

## API keys: set up once

Run `jta init` to save Jev and your selected review provider's keys in
`~/.jta/credentials.json`. They work across all your projects. Keys are kept
separate from project configuration and never included in reports or setup JSON.
The file is unencrypted and readable/writable only by your user on macOS/Linux
(`0600`). `JTA_HOME` relocates both credentials and project data.

Credential precedence is: **shell environment → project `.env` → saved global key**.
Custom `--jev-api-key-env` and `--review-api-key-env` names also identify entries
in the global file. Rerun `init` to replace a saved key, or remove its entry from
`credentials.json` to forget it. Setup never sends a key to a provider to test it.
`--no-input`, JSON output, and redirected input never prompt or save credentials.

## What leaves your machine?

Discovery, import, and reading saved reports run locally. `analyze` sends redacted
session context to Jev; `report` sends selected excerpts and
relevant project files to your chosen LLM provider when recommendations are enabled.
Both offer `--dry-run --format json` so you can inspect the payload before sending it. `jta discover` lists matching sessions entirely offline. Redaction helps remove secrets, but inspect sensitive logs before sending.
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

Development checks require no API credentials:

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```
