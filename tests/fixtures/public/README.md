# Public test sessions

These are downloaded public sessions, not sessions from the developer's machine.
`PROVENANCE.json` records pinned dataset revisions, original URLs, SHA-256 checksums,
and any transformations. No recorded commands are executed during testing.

- `claude-*.jsonl`: four complete native Claude Code logs from
  [Trace Commons](https://huggingface.co/datasets/trace-commons/agent-traces),
  published under CC BY 4.0. Attribution: Trace Commons and its contributors.
- `share-codex-*.json`: three complete session objects selected from
  [nmuendler/share-codex](https://huggingface.co/datasets/nmuendler/share-codex),
  published under CC BY 4.0. Attribution: nmuendler and dataset contributors.
  The dataset wraps messages in an export envelope. Original source-code licensing
  information remains in each object's metadata and is not superseded by the
  dataset license.

These examples exercise format compatibility. They are not human-labeled ground
truth and cannot establish scoring accuracy or probability calibration.

An additional native Codex trace from `Mike0021/codex-sessions` was downloaded to
the ignored `.validation/public/` directory for local compatibility testing. It is
not included here because that dataset did not declare a license at download time.
