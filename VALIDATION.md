# Validation

Validated on 2026-09-20 with Rust 1.98.1 on macOS arm64.

## Public data only

No personal Claude Code or Codex session directories were inspected or analyzed.
Seven pinned public fixtures are bundled: four native Claude Code sessions from
Trace Commons and three complete Codex export records from nmuendler/share-codex.
Together they normalize into 69 assistant turns. Their provenance, licenses, and
checksums are in `tests/fixtures/public/PROVENANCE.json`.

Independent source-record counts and token calculations matched the normalized
Claude sessions exactly:

| Source session prefix | Assistant turns | Input tokens, including cache | Output tokens |
| --- | ---: | ---: | ---: |
| `4c09dfa9` | 1 | 33,132 | 943 |
| `674e0f5c` | 10 | 263,636 | 3,343 |
| `11ef2190` | 11 | 315,428 | 3,239 |
| `da6566ff` | 11 | 317,993 | 2,530 |

An additional public Codex transcript was downloaded for local compatibility
testing. Its export removes tool-call IDs, so the parser reports unlinked tool
results rather than inventing associations. That file remains in ignored local
validation data because the source dataset did not declare a license.

## Automated and command-level checks

Initial implementation checks passed: **30 tests** (22 library and 8 CLI integration tests),
`cargo fmt --check`, and `cargo clippy --all-targets -- -D warnings`.
An optimized executable was built at `target/release/jta`.

The Rust tests cover normalization, streaming block deduplication, concurrent tool
results, delayed reference links, redaction, partial imports, immutable revisions,
HTTP contracts/retries/error sanitization, probability validation, bounded context,
review schemas/references, confidence filters, resource coverage, and model-version
provenance. Command tests cover cache reuse, reports, frozen snapshots, revision
inspection, recurrence thresholds, labels/calibration, and retention of newer
revisions when older revisions are purged.
Comparison tests verify that a rise from two to three completed tasks still shows
a 25-percentage-point regression when cohort sizes change from two to four.
Rates include denominators and Wilson 95% intervals; empty denominators stay null.

Additional black-box checks exercised every offline command family with the public
fixtures, duplicate imports, exact prepared scoring payload bounds, valid JSON
stdout, invalid-argument exit status, and persistence of successful imports from a
partially invalid batch. Fixture SHA-256 verification passed for all seven files;
a cold download also reproduced every fixture's pinned bytes.

Reproduce the automated checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
python3 scripts/download_examples.py --verify-only
```

## Bounded live API smoke tests

Credentials came from the existing `.env`; they were not printed or stored in
analysis configuration or results. Live tests used only the public one-turn
Claude session `4c09dfa9`:

- Jev successfully returned and validated all 23 categorical distributions
  (three session questions and 20 turn questions), including a downstream choice
  whose only supported option was `none`.
- OpenAI `gpt-4.1` returned one valid isolated proposal with source references.
- OpenRouter `openai/gpt-4.1` returned one valid isolated proposal with source
  references.
- Repeating analysis without supplying credentials reused the stored Jev result
  and left the analysis count unchanged.

The first OpenAI attempt exposed an unconstrained remediation-surface schema
field. The field now uses the rubric's enum; the subsequent request passed.
The live results are isolated findings, not recurring harness recommendations.
Local test workspaces and paid-call outputs are ignored under `.validation/`.
A scan found no configured credential values in implementation, fixtures,
documentation, or validation artifacts.

These checks establish implementation behavior and provider compatibility. The
public fixtures have no human usefulness labels, so scoring accuracy, probability
calibration, and causal improvements to agent outcomes remain unmeasured.

## Project discovery extension

Final extension checks passed: **47 tests** (32 library and 15 CLI integration
tests), formatting, strict Clippy, and the optimized release build.

Discovery tests use explicit temporary Codex and Claude storage roots. The tests
never invoke discovery against the developer's actual agent directories.

Independent black-box checks transformed only metadata in downloaded public logs
and placed them in simulated native storage layouts. From a nested project
directory, discovery selected three Codex sessions (including an archive and a
session whose original working subdirectory no longer exists) and one Claude
session. It excluded a sibling project with a shared path prefix, a nested Git
repository, and a helper transcript. A shared Claude storage-directory name did
not override the recorded working-directory match.

The CLI automatically initialized `.jta/` at the checkout root, preserved source
agent and recorded cwd, supported source filtering, and deduplicated repeat imports.
An end-to-end run used a local HTTP mock for 61 scoring requests, then verified
combined project reports, independent Codex/Claude reports, cache reuse without
additional requests, and agent-scoped review previews. Mock judgments exercise
plumbing only and are not semantic assessments of the public sessions.

Regression tests also cover symlink boundaries, separate worktrees, metadata scan
limits, absent and unreadable sources, namespace migration with historical result
preservation, avoiding unrelated parent analysis workspaces, source-only discovery
with no `HOME`, and immutable revisions when non-Git project attribution changes.
