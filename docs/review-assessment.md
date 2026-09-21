# Review quality, coverage, and cost

The report pipeline uses one coding-agent CLI investigation per selected
project/category, followed by synthesis. It replaces the former single HTTP
recommendation request. The implementation is in `src/review_pipeline.rs` and
`src/services/review_cli.rs`.

## Current selection and investigation

Statistics cover the full filtered corpus. Opportunities are grouped by project,
primary opportunity, and remediation surface before recurrence and ranking.
Categories associated with failed or partially completed sessions rank first,
then missed verification, then other opportunities; distinct-session prevalence
breaks ties. These associations do not establish causation.

Default selection is five eligible groups. `--all` expands selection up to
`review.max_investigations` (default 20); omitted groups are disclosed. If no group
meets `min_pattern_sessions` (default three), a preliminary review may proceed
with one supporting session. `--recurring-only` disables that fallback.

Each investigation starts with up to `max(3, min_pattern_sessions)` supporting
sessions plus a successful comparison when available. It receives excerpts of up
to ten supporting turns with neighbors, downstream candidates, final response,
and verification. Unlike the old pipeline, it can retrieve all other normalized
turns and sessions in that project's filtered cohort from staged files.

The agent is asked to distinguish specific underlying issues, investigate
counterexamples, and stop when supported, contradicted, or budget-limited. A
supported observation can be retained as a finding without inventing an edit.
Synthesis merges overlapping findings and reconciles proposed edits. Deterministic
validation checks citation identity and file grounding; it cannot establish the
truth of an interpretation or the causal benefit of an edit.

## Budgets and measurement

For N selected groups, an uncached successful report normally uses N + 1 CLI
invocations. Each stage may use one correction invocation: at most 2 × (N + 1).
There is no report invocation when nothing is selected. CLI version checks are
local and are not counted as model invocations.

Each invocation has a 600-second default wall-clock limit and an 8 MiB captured
output limit. Investigations run sequentially. These controls bound elapsed work
but do not impose a hard token or dollar ceiling. CLI loops may make multiple
model/tool calls. A report with more groups repeats some project context across
independent agent contexts; this trades cost for focused coverage.

The JSON companion stores usage returned by the CLI, stage elapsed times,
inspection references, cache status, and investigation results before synthesis.
The Markdown report summarizes coverage and execution. Inspection is explicitly
self-reported; available evidence is not a claim of complete review. Original
session token tables remain separate from report-generation usage.

Caches include staged session content and judgments, project snapshots, CLI
version, configured model, instructions/schema, budgets, and pipeline version.
Unchanged validated investigations and synthesis are reused. `--refresh` forces
execution, including when an unpinned model alias changes without a CLI change.
Changes anywhere in a project's available evidence can invalidate its stages.
Cache entries retain source revisions so `purge` can remove derived data.

## Jev scoring at 3,000 sessions

Initial scoring is unchanged. Each 100-turn session asks three session questions
and twenty questions per turn: 2,003 judgments. At 64 questions per request:

| Session context | Requests per session | Requests for 3,000 uncached sessions |
| --- | ---: | ---: |
| Full state fits the 48,000-character threshold | 32 | 96,000 |
| Full state exceeds the threshold | 101 | 303,000 |

These exclude retries. Context and question instructions repeat across batches;
unchanged analyses are cached. `cargo run --locked --example llm_usage` exercises
the actual builders on synthetic sessions without credentials or model calls.
Character counts are not tokenizer counts or billed usage.

## Remaining limits and evaluation

- Categories remain rubric buckets, not persisted semantic clusters. Agents split
  issues during investigation; category sampling can still miss rare issues.
- Initial samples are deterministic and favor outcome severity. More evidence is
  available, but retrieval quality depends on the agent's decisions.
- File snapshots are bounded and reflect current files, not historical versions.
- Corpus loading, JSON construction, and per-investigation staging still duplicate
  data. The pipeline is not a streaming solution for very large corpora.
- Read-only execution prevents applying edits; citation validation does not prove
  that recommendations improve task success.
- No live model-quality benchmark has been run for this CLI redesign. Fake CLI
  tests establish adapter, retrieval, caching, and validation behavior only.

Use a fixed labeled corpus to compare issue recall, unsupported claims, useful
recommendations, coverage, elapsed time, and CLI-reported usage. Include late
issues, unrelated problems in the same category, cross-project collisions,
successful counterexamples, and instructions that already cover the behavior.
