# Remote service contracts

Jev scoring uses the [official TypeSafe evaluation API](https://docs.typesafe.ai/api):
`POST https://api.typesafe.ai/v1/systemone`, bearer authentication, and a body with
`model`, `state`, and a named `questions` map. The default model is `jev-latest`;
the CLI reads `JEV_API_KEY` (the environment variable name is configurable).
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

`prepare_analysis` and `prepare_review` return exact request bodies without reading
credentials or making network requests. Review uses Chat Completions with strict
JSON Schema for [OpenAI](https://developers.openai.com/api/docs/guides/structured-outputs)
and [OpenRouter](https://openrouter.ai/docs/guides/features/structured-outputs).
OpenRouter requests require supporting provider parameters. Review proposals must
include observed pattern, outcome effect, supporting session/turn pairs,
uncertainty, counterexamples, remediation surface, concrete change, scope, risk,
and an evaluation plan. Local validation rejects malformed or unknown references.
No generated review output is executed or used as a replacement for Jev scoring.

HTTP uses configurable total request timeouts, a maximum twenty-second connection
timeout, no redirects, and at most three attempts for connection failures,
timeouts, HTTP 429, or server errors. Retry-After seconds are capped at thirty;
otherwise delays are one then two seconds. Authentication and validation failures
are not retried. Response bodies are limited to 8 MiB. Errors omit remote bodies,
credential values, and endpoint details. HTTPS is required except for loopback
mock servers. Retrying a timed-out POST may cause a provider to bill both attempts.

Offline contract tests use local HTTP fixtures and never require paid credentials.

Every successful response must identify the actual Jev model. Analysis provenance
stores both requested alias and resolved models, and reports flag mixed resolved
versions even when local configuration fingerprints match. An unchanged alias can
resolve differently later; cached analyses cannot discover that change without a
new request. Pin a provider model version for controlled comparisons or refresh
analyses and inspect the resolved identities. Source and bounded-evidence warnings
are exposed in report `evidence_warnings`; original distributions are never
rewritten into synthetic certainty or synthetic uncertainty.
