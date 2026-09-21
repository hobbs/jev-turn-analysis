# Remote service contracts

Jev scoring uses the [official TypeSafe evaluation API](https://docs.typesafe.ai/api):
`POST https://api.typesafe.ai/v1/systemone`, bearer authentication, and a body with
`model`, `state`, and a named `questions` map. The default model is `jev-latest`;
the CLI resolves `JEV_API_KEY` from the shell, project `.env`, or global
`~/.jta/credentials.json`, in that order (the key name is configurable).
Interactive `jta init` saves global credentials separately from project config;
`JTA_HOME` overrides the global directory. Files are atomically written with
user-only `0600` permissions on Unix. Keys never enter reports or config output.
Every question is `choice`, with explicit instructions and a criteria map. Turn
IDs appear in instructions because question-map keys do not enter inference.

Three session questions and twenty questions per turn are sent: eight rubric
questions, one downstream candidate selection, and eleven independent yes/no
opportunity questions. Secondary opportunities are supported yes answers excluding
the selected primary category. All distributions, including secondary and
downstream questions, are retained. Missing categories, extra categories, invalid
probabilities, sums outside the rounding tolerance, unknown selections, and incomplete
responses fail the analysis rather than inventing a judgment. The sum tolerance is
0.001 for higher-precision distributions. When every probability is rounded to
hundredths, as observed in live Jev responses, the tolerance is 0.005 per option
(the maximum cumulative rounding error). All-zero distributions are rejected.
Original probabilities are preserved; accepted sums differing from one by more
than 0.001 produce analysis warnings. Sum errors identify the question, total, and
tolerance. The API documents `choice` as the highest-probability option, but live
responses can disagree with their reported distributions. Such disagreements
produce analysis warnings identifying the question, selected option, and a
highest-probability alternative with both probabilities; turn answers also receive
an inconsistency marker. The original selection and probabilities are preserved
instead of rejecting the entire session or substituting a different judgment.
Ties and floating-point differences within 1e-9 do not trigger warnings.
`confidence()` is
the selected category's probability; `provider_confidence` separately preserves
[Jev's confidence measure](https://docs.typesafe.ai/confidence).

Small sessions share their complete normalized transcript across question batches.
Large sessions get deterministic packets with independently budgeted excerpts for
the goal and user corrections, target, final state, verification, nearby turns,
and candidate downstream evidence. Excerpts preserve source lines, event indices,
and turn IDs. Truncation and omitted-event counts are explicit. The character
budget applies to serialized state, not HTTP headers or the question rubric.
An omitted check does not establish that no check happened. Mechanical dependencies
are candidates for Jev, not preassigned usefulness. Cross-question contradictions
and close probability margins are marked for further review.

`prepare_analysis` returns exact Jev request bodies without reading credentials
or making network calls. Jev HTTP uses configurable request timeouts, a maximum
20-second connection timeout, no redirects, and up to three attempts for connection
failures, timeouts, HTTP 429, or server errors. Retry-After is capped at 30 seconds;
otherwise delays are one then two seconds. Responses are limited to 8 MiB.
Authentication failures are not retried. Errors omit bodies and credentials.
HTTPS is required except for loopback test servers.

## Report CLI contracts

Report generation uses `codex` or `claude` on PATH. JTA has no OpenAI/OpenRouter
review client or review credentials. `--version` checks availability and becomes
part of cache identity; authentication is exercised by the actual investigation.
No login command is launched and no CLI token is copied into JTA storage.

- Codex: `exec --sandbox read-only --ignore-user-config --ignore-rules
  --skip-git-repo-check --ephemeral --json --output-schema schema.json
  --output-last-message answer.json -`. Instructions arrive on stdin. A successful
  `turn.completed` event and a valid JSON answer file are both required.
- Claude: `--print --safe-mode --no-session-persistence --output-format stream-json
  --verbose --permission-mode dontAsk --tools Read,Glob,Grep
  --allowedTools Read,Glob,Grep --strict-mcp-config --mcp-config '{"mcpServers":{}}'
  --json-schema <schema>`. A successful `result` event with `structured_output` is
  required. Safe mode preserves CLI authentication; bare mode is intentionally
  not used because it excludes subscription login.

These interfaces were checked against the installed CLIs and their official docs:
[Codex noninteractive mode](https://learn.chatgpt.com/docs/non-interactive-mode),
[Claude programmatic usage](https://code.claude.com/docs/en/headless), and
[Claude CLI reference](https://code.claude.com/docs/en/cli-reference).
Use current CLI versions supporting the flags above. JTA isolates report tasks
from user/project customizations; an optional `review.model` is passed explicitly.

Processes start directly, with no shell interpolation, in temporary evidence
folders. Original project paths identify citations; agents are instructed to read
only staged files. Codex enforces read-only tool execution, while Claude receives
only Read/Glob/Grep tools. This is not a claim that staged files form a complete
filesystem read sandbox. Transcript instructions remain untrusted data.
Ephemeral sessions avoid adding JTA investigations to normal session discovery.
On Unix, process groups are killed on timeout, cancellation, and exit so tool
subprocesses do not survive their investigation. Raw CLI errors are not echoed.

`manifest.json` maps normalized sessions and original project paths to redacted
snapshot files. `initial.json` contains category samples or, for synthesis,
validated investigation results. Session snapshots include the full normalized
session and stored Jev judgments, allowing retrieval beyond the initial excerpts.
Project files have separate count/size limits documented in the guide.

The common response contains `summary`, `themes`, `findings`, `recommendations`,
`inspected_refs`, and `inspected_files`. Findings require project-scoped source
references but need not suggest an edit. Recommendations also require exact
file targets, unique before passages, literal replacements, exact context quotes,
and the configured number of distinct supporting sessions. Quotes and references
must appear in the inspection record. Overlapping edits are rejected. Inspection
is self-reported and cannot prove that the model actually read a particular file.

One correction invocation is allowed after local validation failure. It receives
the same staged evidence, the rejected answer, and the validation error. Malformed
CLI events, failed execution, timeouts, and incomplete output fail immediately.
No invalid report or recommendations are saved. Successful earlier stages can be
reused on a later run.

CLI event traces and reported usage are redacted before saving in `review_cache/`.
Reports record invocation counts, elapsed time, per-stage usage, cache status, and
coverage. CLI invocations are not equivalent to model requests. JTA does not apply
a hard model-token or dollar limit; its budgets bound investigations, wall time,
and captured process output. Cached stages report zero new invocations and keep
historical usage separate. Purging expired source revisions also removes dependent
cache entries and reports.
