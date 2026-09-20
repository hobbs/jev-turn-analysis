# Review prioritization and scale

The current review is useful for a small, grounded investigation. It does not
reliably find the most common underlying issues across a large corpus. At 3,000
sessions with 100 turns each, repeated scoring context, unbudgeted recommendation
prompts, sampling coverage, and memory use are substantial limits. Increasing
`--top` or using `--all` does not fix them.

This assessment covers the selection and evidence logic used by `jta report`.
The command combines statistics and
recommendations in one Markdown artifact.
Report filters now also constrain the pattern candidates used for recommendations.

## What the current logic does well

- Counts distinct supporting sessions, so 100 matching turns from one session
  cannot masquerade as 100 independent cases.
- Prioritizes failed or partially complete tasks and verification gaps.
- Includes a successful-outcome comparison when available.
- Requires each accepted proposal to cite enough distinct sessions and belong to
  one project. File targets, exact replacement text, and quoted project context
  are checked before saving.
- Discloses omitted sessions and turns. The Markdown report separates full-corpus
  statistics from the much smaller recommendation sample.

These checks establish traceability and, for recurring reviews, recurrence in the supplied sample. They
do not validate the model's causal explanation or prove that its advice will help.

## Does it concentrate effort on the most common themes?

Only within a broad severity tier. `analytics::patterns` groups turns by
`(opportunity, remediation_surface)` and sorts by:

1. Any supporting session failed or was partially complete.
2. Otherwise, the opportunity is missed verification.
3. Otherwise, another opportunity.

Within each tier, categories with more distinct sessions come first. Pattern ID
breaks ties. A category appearing in three sessions, including one failed task,
can outrank a category appearing in 1,000 successful sessions. The failed outcome
is associated with the session; it does not establish that the categorized turn
caused the failure. `abandoned` is not included in the highest tier.

The rubric has 11 non-`none` primary opportunities and 10 remediation surfaces.
Consequently, valid judgments can produce at most 110 broad categories, however
many underlying problems exist. Missing an export check and missing a database
migration check can share the same category. The grouping ignores semantic
similarity, named tools, workflow stage, and secondary opportunities. Project and
agent counts are recorded, but project identity is not part of the grouping key.
A global recurrence threshold can therefore be met while no single project has
enough supporting sessions for a valid proposal.

When no category meets the recurrence threshold, the default falls back to a
preliminary review of the available categories. Proposals need at least one
supporting session and are labeled tentative in the prompt, report, and saved
recommendations. `--recurring-only` disables this fallback.

The default selects five categories, then up to `max(3, min_pattern_sessions)` sessions
per category. Sampling favors failed/partial outcomes, then unverified claims,
then other outcomes, with session ID as the tie-breaker. It can replace the last
choice with a successful session. This is deterministic, not a representative
sample by issue, date, tool, project, or source agent. There is no rotation across
repeated reports and no adaptive increase when a category contains more variety.

Evidence selection takes the first ten supporting turn IDs across all selected
categories in a session, then adds neighboring turns, downstream candidates,
the final turn, and up to three verification turns. Later issues can receive no
direct excerpt even when their category was selected. A low recommendation count
can mean insufficient coverage or grounding, rather than an absence of problems.

## LLM token usage at 3,000 sessions

There are two separate remote workloads: initial Jev scoring in `analyze` and
recommendation generation in `report`. Full-corpus statistics and Markdown
formatting are local and consume no model tokens. The report's token tables
measure the original coding-agent sessions, not JTA's own processing costs.

### Initial scoring can be the larger expense

At current defaults, each 100-turn session contributes three session questions
plus 20 questions per turn, or 2,003 questions. Across 3,000 sessions, that is
6,009,000 requested judgments on a fresh analysis.

`prepare_analysis` uses two paths:

| Session context | Requests per 100-turn session | Requests for 3,000 uncached sessions |
| --- | ---: | ---: |
| Full serialized state fits within 48,000 characters | ceil(2,003 / 64) = 32 | 96,000 |
| Full state exceeds 48,000 characters | 1 outcome packet + 100 turn packets = 101 | 303,000 |

These are default-configuration request counts before retries. The first path
repeats the whole session state across batches. The second builds bounded packets
for each turn, repeating shared goals, outcomes, and neighboring context. Each
question also repeats rubric instructions. A corpus with both session shapes
falls between these request counts. Cached, unchanged analyses avoid scoring
requests; changed revisions or a refresh can trigger them again.

The [`llm_usage` example](../examples/llm_usage.rs) calls the real request builders
on synthetic 100-turn sessions, without making network requests:

```sh
cargo run --locked --example llm_usage
```

Observed serialized payload sizes on September 20, 2026:

| Synthetic session | Context characters across requests | Question characters across requests | Combined characters for 3,000 sessions of that shape |
| --- | ---: | ---: | ---: |
| Minimal event text | 1,322,208 | 1,970,112 | 9,876,960,000 |
| Events with about 2,000 characters each | 1,584,235 | 1,970,181 | 10,663,248,000 |

These count transmitted JSON characters, not actual model tokens or billed usage.
They exclude outputs, retries, and service-side prompt construction. Jev's internal
processing and usage accounting cannot be inferred from the serialized payload
alone. The measurement establishes substantial repetition in the client requests;
it does not establish a dollar bill. The memory probe below contains no transcript
events and should not be used to estimate this workload's model usage.

### Recommendation input grows with the selected sample

The recommendation model does not receive all 300,000 turns. With the default
recurrence threshold, `report` selects three sessions per chosen category. Shared
sessions are deduplicated. Default selection is five categories, so at most 15
distinct sessions are sent. A corpus ten times larger can still send the same
sample size, at the cost of examining a smaller share of the issues.

For planning, let `T` be tokens of excerpted evidence and judgments per sampled
session, and `F` be fixed instructions, schema, and the selected project context.
First-request input is approximately `F + sampled_sessions × T`, plus category
metadata. This is a planning model; use the configured model's tokenizer to size
a concrete payload.

The following is an illustration assuming `T = 10,000`, not a measured token count:

| Selection, with disjoint supporting sessions | Sampled sessions | Session-evidence input tokens, before F |
| --- | ---: | ---: |
| One category | 3 | 30,000 |
| Default five categories | 15 | 150,000 |
| `--all` with 30 eligible categories | 90 | 900,000 |
| `--all` with all 110 valid category combinations | 330 | 3,300,000 |

`--all` sends one larger request. It does not batch, summarize, or dynamically
resize the evidence to the selected model's context window. Actual size depends
on excerpt length, tool inputs, judgments, overlap between sampled sessions,
project files, and the recurrence setting. Ten initial supporting turns can
expand to as many as 64 turns per session. With five events per turn and separate
1,000-character limits for event text and tool input, 15 sessions can contribute
9.6 million characters from those fields alone. That is a permitted worst-case
shape, not typical usage and not an enforced total request limit.

### Fixed prompt cost, output, and repeated calls

The actual system prompt contains **32,945 characters**, including **28,696
characters of Humanizer guidance**. The serialized response format/schema adds
1,874 characters. These are measured by the example with the default recurrence
threshold; they are not tokenizer counts. Project context has a separate
96,000-character file-text budget, plus metadata.

The request allows **6,000 completion tokens for the whole response**. All
recommendations, literal edits, citations, summaries, and evaluation plans share
that allowance. Selecting more categories does not increase it. An interrupted
or truncated response fails rather than producing a partial report. The Humanizer
guidance runs inside this same request; it adds input, not a second rewriting call.

On a grounding/schema validation failure, the correction request repeats the
original input and adds the first answer and an error message. If the initial
input is `I` tokens and the rejected answer is `R`, the two inputs total roughly
`2I + R + correction instructions`, before any transport retries. Each of these
requests permits up to three HTTP attempts. A timed-out attempt might already
have been processed; attempt count is not a reliable count of billable completions.

Reports have no recommendation cache. Repeating an unchanged report repeats this
work. The current service code discards the recommendation provider's `usage`
object when parsing its answer and retains only HTTP attempt counts. Dollar
cost requires actual provider usage, the selected model's rates, and any cache
accounting; the existing artifacts cannot establish it.

Inspect a concrete request without spending provider tokens:

```sh
jta report --dry-run --format json > report-preview.json
```

Use the selected model's tokenizer on the message content and account for its
message framing and structured-output schema. A character-to-token ratio is only
an approximation, especially for JSON, code, identifiers, or multilingual text.

### Token-efficiency changes to prioritize

1. Add model-aware input sizing and a hard total input/output budget before sending
   any recommendation request. Reserve output space; expose predicted usage in
   dry runs and retain actual provider usage in the saved report.
2. Reuse scored judgments and cached compact issue summaries. Retrieve the raw
   excerpts needed for each proposal instead of repeating all selected sessions
   in every batch. Cache recommendations against evidence, project-file hashes,
   model, and prompt version.
3. Batch independent issue clusters within the token budget and deduplicate their
   proposals using compact summaries and references. Give each cluster direct
   evidence and an explicit share of the budget.
4. Reduce repeated rubric/context transmission during initial scoring where the
   Jev API contract permits it. Validate the effect on classification quality;
   simply shortening context can remove the evidence needed to score correctly.
5. Replace the full Humanizer examples in production prompts with a concise,
   reviewed writing guide derived from that skill. Compare clarity and grounding
   before adopting the shorter prompt. Provider prompt caching, when available,
   may reduce billed input but does not make an oversized request fit.

These changes are recommendations, not implemented token controls. The current
pipeline is economical only when the chosen sample and its excerpts stay small;
it does not yet provide predictable token cost for comprehensive corpus review.

## The 300,000-turn memory and clustering check

An offline synthetic probe is provided in
[`examples/review_scale.rs`](../examples/review_scale.rs). It constructs 3,000
sessions with 100 turns each and 30 different issue descriptions assigned to the
same primary category. It uses only two turn-judgment fields, two session fields,
and no transcript events. It does not call a provider or read personal sessions.

Run it on macOS with:

```sh
cargo build --locked --release --example review_scale
/usr/bin/time -l target/release/examples/review_scale
```

Observed in the development environment on September 20, 2026:

| Measurement | Result |
| --- | ---: |
| Sessions / turns | 3,000 / 300,000 |
| Fixture construction | 0.11 seconds |
| Pattern aggregation | 0.19 seconds |
| Full corpus aggregation | 3.41 seconds |
| Process peak resident memory | 5,663,440,896 bytes (5.66 GB) |
| Described issue clusters / resulting categories | 30 / 1 |
| Default sessions sampled from that category | 3 (0.1%) |

These are measurements of one synthetic workload, not a production throughput
guarantee. Memory includes the fixture and aggregation in the same process. The
probe excludes store reads, request construction, Markdown rendering, JSON
serialization, and model latency. Real session events and full distributions
add data. It does not measure the quality of generated recommendations.

The 30 descriptions intentionally exercise the category-collision problem; they
are not a benchmark of semantic clustering quality. The code never reads those
descriptions while grouping, so all 30 necessarily collapse into one category.

## Where scale breaks down

| Area | Current behavior | Consequence |
| --- | --- | --- |
| Corpus loading | Reads all current sessions and retained analyses; cohort matching scans analyses for each session | O(sessions × analyses) matching; historical analyses increase work |
| Aggregation memory | Clones sessions and analyses for filtered, agent, project, and pattern aggregates; materializes per-turn references | Several large copies coexist, as the probe demonstrates |
| Turn lookups | Scans the session's turn list for each judgment, repeated across breakdowns | O(turns per session²) lookup work |
| Pattern sampling | Default at most 15 distinct sessions across five categories, often fewer | A growing corpus does not earn more representative coverage |
| Many categories | `--all` still samples a few sessions per category | Coverage inside each category remains weak |
| Provider context | One request; local excerpt bounds, no global request token budget | More selected categories can exceed the model's context limit |
| Provider output | Fixed 6,000 completion tokens, with no per-cluster batches | Many independent issues compete for limited response space; truncation fails validation |
| Repeated reports | No cached semantic findings or incremental review; full evidence copied into saved proposals | Repeated context costs and redundant storage grow with use |

Even the default request has no small global bound. Ten supporting turns can
expand to as many as 64 turns per sampled session after adding context, each with
up to five events and separately bounded text and input excerpts. Five categories
can select 15 sessions. Those limits permit millions of characters before project
files and judgments. `--all` is not a streaming or map/reduce implementation.

## Recommended next implementation

First, make coverage measurable and reduce aggregation memory. Use indexes for
session/revision and turn lookups, borrow data during aggregation, and retain only
compact counts and reference IDs until selected excerpts are needed. Stream or
page historical analyses. Keep exact counts separate from model-generated prose.

Then add a discovery stage before drafting edits:

1. Create compact issue records with project, behavior, tool or artifact, workflow
   stage, outcome, confidence, and source references. Cache them by session
   revision, rubric, model, and prompt version. Process only new or changed turns.
2. Cluster related issue records within their applicable project/workflow. Keep
   distinct-session and affected-turn counts for each cluster. Audit cluster
   merging and splitting with labeled examples, especially issues sharing a rubric
   category. Include secondary opportunities without double-counting sessions.
3. Allocate review effort primarily by distinct-session prevalence, with measured
   outcome association and confidence shown separately. Reserve explicit capacity
   for rare severe failures so the frequency ordering remains understandable.
   Treat observed wasted tokens as cost evidence, not guaranteed savings.
4. Sample examples across each cluster's time periods, agents, projects where
   appropriate, and outcomes. Include counterexamples. Expand sampling when the
   examples disagree; ensure each selected cluster retains direct turn excerpts.
5. Draft recommendations in batches with hard input/output token budgets and
   bounded concurrency. Merge overlapping proposals in a final pass while
   preserving cluster counts, project scope, and supporting references. Retain the
   existing grounding and minimum-support validation.

The report should state how many eligible clusters and affected sessions were
reviewed, which remain, and what the budget omitted. Validate the redesign with
a 300,000-turn fixture containing common, rare severe, and deliberately overlapping
issues. Check cluster recall, priority ordering, peak memory, prompt budgets,
incremental cache reuse, and whether common late-session issues reach the model.

The combined report and Humanizer integration are implemented in this change.
Semantic clustering, adaptive sampling, batching, and the aggregation redesign
remain future work.
