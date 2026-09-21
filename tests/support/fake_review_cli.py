#!/usr/bin/env python3
"""Offline CLI contract fixture; never contacts a model provider."""
import json
import os
from pathlib import Path
import sys

backend = Path(sys.argv[0]).name
if "--version" in sys.argv:
    print(f"{backend} fixture 1.0")
    sys.exit(0)
args = sys.argv[1:]
assert "JEV_API_KEY" not in os.environ
prompt = sys.stdin.read()
mode = os.environ.get("JTA_FAKE_MODE", "")
if mode == "fail":
    print("secret-sentinel", file=sys.stderr)
    sys.exit(1)
if backend == "codex":
    assert args[0] == "exec"
    assert args[args.index("--sandbox") + 1] == "read-only"
    assert "--ephemeral" in args and "--ignore-user-config" in args
else:
    assert "--safe-mode" in args and "--no-session-persistence" in args
    assert args[args.index("--tools") + 1] == "Read,Glob,Grep"
    assert args[args.index("--permission-mode") + 1] == "dontAsk"
    assert "--bare" not in args
initial = json.loads(Path("initial.json").read_text())
manifest = json.loads(Path("manifest.json").read_text())
# Actually retrieve turns outside the initial excerpts and file contents.
sessions = [json.loads(Path(s["path"]).read_text()) for s in manifest["sessions"]]
files = [json.loads(Path(f["snapshot_path"]).read_text()) for p in manifest["projects"] for f in p["files"]]
assert "secret-sentinel" not in json.dumps(sessions + files)
if "JTA_FAKE_LOG" in os.environ:
    with open(os.environ["JTA_FAKE_LOG"], "a") as f:
        f.write(json.dumps({"backend": backend, "args": args, "synthesis": "SYNTHESIS:" in prompt,
                           "correction": "CORRECTION:" in prompt, "sessions": len(sessions),
                           "cwd": str(Path.cwd()), "manifest": manifest}) + "\n")
refs = [{"session_id": s["session_id"], "turn_id": t["id"]} for s in sessions for t in s["turns"]]
root = manifest["projects"][0]["project_root"]
result = {"summary": "Inspect the recorded verification result before changing instructions.",
          "themes": ["The sampled behavior deserves investigation."], "findings": [], "recommendations": [],
          "inspected_refs": refs, "inspected_files": [f["path"] for f in files]}
if refs:
    result["findings"] = [{"project_root": root, "title": "Verification needs clearer evidence",
                           "observation": "The recorded result is ambiguous.", "supporting_refs": [refs[-1]],
                           "uncertainty": "A missing record does not establish a missing check.", "counterexamples": []}]
if files and refs and mode != "findings_only":
    f = next((f for f in files if f["text"].strip()), None)
    if f:
        result["recommendations"] = [{"project_root": root, "title": "Record the existing check result",
            "observed_pattern": "The verification result was not clear in the cited turns.",
            "outcome_effect": "Outcome remains uncertain.", "supporting_refs": refs,
            "uncertainty": "Limited sample.", "counterexamples": [], "remediation_surface": "AGENTS.md",
            "proposed_change": "Record the project check result.", "scope": "This project only.",
            "risk": "Extra reporting.", "evaluation_plan": "Replay the task and check the recorded result.",
            "targets": [{"path": f["path"], "action": "edit", "before": f["text"],
                         "after": f["text"] + "\nRecord the check result.\n", "rationale": "Clarify this project's existing check.",
                         "context_refs": [{"path": f["path"], "quote": f["text"]}]}]}]
if "SYNTHESIS:" in prompt:
    # Preserve validated results, consolidating duplicate proposals.
    first = initial["investigations"][0]["result"]
    result["findings"] = first["findings"]
    result["recommendations"] = first["recommendations"]
if mode in ("bad_quote", "repair") and "CORRECTION:" not in prompt and result["recommendations"]:
    result["recommendations"][0]["targets"][0]["context_refs"][0]["quote"] = "invented passage"
if mode == "bad_quote" and result["recommendations"]:
    result["recommendations"][0]["targets"][0]["context_refs"][0]["quote"] = "invented passage"
if mode == "unknown_ref":
    result["inspected_refs"].append({"session_id": "unknown", "turn_id": 999})
if mode == "uninspected":
    result["inspected_refs"] = []
if mode == "insufficient" and result["recommendations"]:
    result["recommendations"][0]["supporting_refs"] = [refs[0]]
if mode == "malformed":
    print("not JSON")
    sys.exit(0)
if backend == "codex":
    Path(args[args.index("--output-last-message") + 1]).write_text(json.dumps(result))
    print(json.dumps({"type": "item.completed", "item": {"type": "command_execution", "command": "read staged evidence", "exit_code": 0}}))
    print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 123, "output_tokens": 45}}))
else:
    print(json.dumps({"type": "result", "subtype": "success", "is_error": False,
                      "structured_output": result, "usage": {"input_tokens": 123, "output_tokens": 45},
                      "modelUsage": {}, "total_cost_usd": 0.01, "num_turns": 3}))
