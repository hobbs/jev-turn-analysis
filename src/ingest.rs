use crate::{
    config::{hash, Config},
    model::*,
    redact::Redactor,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn content(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|b| {
                string(b, "text")
                    .or_else(|| string(b, "thinking"))
                    .unwrap_or_default()
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}
fn arguments(v: &Value) -> Value {
    if let Some(s) = v.as_str() {
        serde_json::from_str(s).unwrap_or_else(|_| v.clone())
    } else {
        v.clone()
    }
}
fn usage(v: &Value) -> Option<Usage> {
    let u = v.get("usage")?;
    Some(Usage {
        input_tokens: u["input_tokens"].as_u64().map(|n| {
            n + u["cache_creation_input_tokens"].as_u64().unwrap_or(0)
                + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
        }),
        output_tokens: u["output_tokens"].as_u64(),
        cached_input_tokens: u["cache_read_input_tokens"]
            .as_u64()
            .or(u["cached_input_tokens"].as_u64()),
    })
}
fn base(v: &Value, line: usize) -> Event {
    Event {
        line,
        timestamp: string(v, "timestamp"),
        message_id: string(v, "uuid").or_else(|| string(v, "id")),
        ..Default::default()
    }
}
fn blocks(out: &mut Vec<Event>, v: &Value, m: &Value, line: usize) {
    let mut e = base(v, line);
    e.message_id = string(m, "id").or(e.message_id);
    e.usage = usage(m);
    let role = m["role"].as_str().unwrap_or("unknown");
    if let Some(a) = m["content"].as_array() {
        for b in a {
            let mut e = e.clone();
            match b["type"].as_str().unwrap_or("") {
                "tool_use" => {
                    e.kind = "tool_call".into();
                    e.tool_name = string(b, "name");
                    e.call_id = string(b, "id");
                    e.input = Some(b["input"].clone());
                }
                "tool_result" => {
                    e.kind = "tool_result".into();
                    e.call_id = string(b, "tool_use_id");
                    e.text = content(&b["content"]);
                    e.is_error = b["is_error"].as_bool();
                }
                "text" | "input_text" | "output_text" => {
                    e.kind = role.into();
                    e.text = content(&b["text"]);
                }
                "thinking" => {
                    e.kind = "reasoning".into();
                    e.text = content(&b["thinking"]);
                }
                _ => {
                    e.kind = "context".into();
                    e.text = b.to_string();
                }
            }
            out.push(e);
        }
    } else {
        e.kind = role.into();
        e.text = content(&m["content"]);
        out.push(e);
    }
}
pub fn parse_file(path: &Path, config: &Config) -> Result<Session> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let redactor = Redactor::new(&config.redaction)?;
    let mut session = Session {
        source_path: redactor.text(&path.to_string_lossy()),
        imported_at: chrono::Utc::now().to_rfc3339(),
        parser_version: PARSER_VERSION.into(),
        redaction_fingerprint: hash(&serde_json::to_vec(&config.redaction)?),
        ..Default::default()
    };
    let mut native_id = None;
    let mut records = vec![];
    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
        if let Some(messages) = v["messages"].as_array() {
            session.agent = if v["metadata"].to_string().to_lowercase().contains("claude") {
                "claude_code"
            } else {
                "codex"
            }
            .into();
            native_id = string(&v, "id");
            session.repository = string(&v["metadata"], "cwd");
            for (i, m) in messages.iter().enumerate() {
                records.push((
                    i + 1,
                    serde_json::json!({"type":"export_message","message":m}),
                ));
            }
            session.warnings.push("export envelope: source locations identify message ordinals; original JSONL lines unavailable".into());
        } else {
            records.push((1, v));
        }
    } else {
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(v) => records.push((i + 1, v)),
                Err(_) => session.warnings.push(format!(
                    "line {}: malformed JSON record; evidence missing",
                    i + 1
                )),
            }
        }
    }
    let response_messages: BTreeSet<(String, String)> = records
        .iter()
        .filter(|(_, v)| v["type"] == "response_item" && v["payload"]["type"] == "message")
        .map(|(_, v)| {
            (
                string(&v["payload"], "role").unwrap_or_default(),
                redactor.text(&content(&v["payload"]["content"])),
            )
        })
        .collect();
    for (line, mut v) in records {
        if session.repository.is_none() {
            if let Some(cwd) = string(&v, "cwd").or_else(|| string(&v["payload"], "cwd")) {
                session.project_root = crate::discovery::recorded_project_root(Path::new(&cwd))
                    .ok()
                    .map(|p| redactor.text(&p.to_string_lossy()));
            }
        }
        redactor.value(&mut v);
        let kind = v["type"].as_str().unwrap_or("");
        if native_id.is_none() {
            native_id = string(&v, "sessionId").or_else(|| string(&v, "session_id"));
        }
        if session.repository.is_none() {
            session.repository = string(&v, "cwd").or_else(|| string(&v["payload"], "cwd"));
        }
        match kind {
            "session_meta" => {
                session.agent = "codex".into();
                native_id = native_id.or_else(|| string(&v["payload"], "id"));
            }
            "user" | "assistant" if v.get("message").is_some() => {
                session.agent = "claude_code".into();
                blocks(&mut session.events, &v, &v["message"], line);
            }
            "export_message" => {
                let m = &v["message"];
                if m["role"] == "tool" {
                    let mut e = base(m, line);
                    e.kind = "tool_result".into();
                    e.text = content(&m["content"]);
                    e.call_id = string(m, "tool_call_id");
                    e.tool_name = string(m, "name");
                    session.events.push(e);
                } else {
                    blocks(&mut session.events, &v, m, line);
                    if let Some(calls) = m["tool_calls"].as_array() {
                        for c in calls {
                            let mut e = base(m, line);
                            e.kind = "tool_call".into();
                            e.call_id = string(c, "id");
                            e.tool_name = string(&c["function"], "name");
                            e.input = Some(arguments(&c["function"]["arguments"]));
                            session.events.push(e);
                        }
                    }
                }
            }
            "response_item" => {
                session.agent = "codex".into();
                let p = &v["payload"];
                let mut e = base(&v, line);
                match p["type"].as_str().unwrap_or("") {
                    "message" => {
                        blocks(&mut session.events, &v, p, line);
                        continue;
                    }
                    "function_call" | "custom_tool_call" => {
                        e.kind = "tool_call".into();
                        e.call_id = string(p, "call_id").or_else(|| string(p, "id"));
                        e.tool_name = string(p, "name");
                        e.input = Some(arguments(
                            p.get("arguments")
                                .or_else(|| p.get("input"))
                                .unwrap_or(&Value::Null),
                        ));
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        e.kind = "tool_result".into();
                        e.call_id = string(p, "call_id");
                        e.text = content(&p["output"]);
                        e.is_error = p["exit_code"].as_i64().map(|n| n != 0);
                    }
                    "reasoning" => {
                        e.kind = "reasoning".into();
                        e.text = content(&p["summary"]);
                    }
                    _ => {
                        session
                            .warnings
                            .push(format!("line {line}: unknown Codex response item"));
                        continue;
                    }
                }
                session.events.push(e);
            }
            "event_msg" => {
                session.agent = "codex".into();
                let p = &v["payload"];
                let mut e = base(&v, line);
                match p["type"].as_str().unwrap_or("") {
                    "user_message" | "agent_message" => {
                        e.kind = if p["type"] == "user_message" {
                            "user"
                        } else {
                            "assistant"
                        }
                        .into();
                        e.text = content(&p["message"]);
                        if response_messages.contains(&(e.kind.clone(), e.text.clone())) {
                            continue;
                        }
                    }
                    "token_count" => {
                        e.kind = "usage".into();
                        e.usage =
                            usage(&serde_json::json!({"usage":p["info"]["last_token_usage"]}));
                    }
                    _ => {
                        e.kind = "context".into();
                        e.text = p.to_string();
                    }
                }
                session.events.push(e);
            }
            "mode"
            | "permission-mode"
            | "ai-title"
            | "turn_context"
            | "queue-operation"
            | "file-history-snapshot"
            | "progress"
            | "system"
            | "summary"
            | "attachment"
            | "last-prompt" => {
                let mut e = base(&v, line);
                e.kind = "context".into();
                e.text = v.to_string();
                session.events.push(e);
            }
            _ => session
                .warnings
                .push(format!("line {line}: unsupported record type {kind:?}")),
        }
    }
    if session.agent.is_empty()
        || !session
            .events
            .iter()
            .any(|e| matches!(e.kind.as_str(), "assistant" | "user" | "tool_call"))
    {
        bail!(
            "{}: unsupported session format or no conversation records",
            path.display()
        )
    }
    session.project_root = session.project_root.or_else(|| {
        session.repository.as_deref().and_then(|cwd| {
            let path = Path::new(cwd);
            if !path.is_absolute() {
                return None;
            }
            crate::discovery::recorded_project_root(path)
                .ok()
                .map(|root| redactor.text(&root.to_string_lossy()))
        })
    });
    session.started_at = session
        .events
        .iter()
        .filter_map(|e| e.timestamp.clone())
        .min();
    session.ended_at = session
        .events
        .iter()
        .filter_map(|e| e.timestamp.clone())
        .max();
    for event in &mut session.events {
        if let Some(input) = &mut event.input {
            redactor.value(input);
        }
        event.text = redactor.text(&event.text);
    }
    session.turns = turns(&session.events, &mut session.warnings);
    if session.turns.is_empty() {
        session.warnings.push("no assistant turns recorded".into());
    }
    if session
        .events
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind.as_str(),
                "assistant" | "tool_call" | "tool_result" | "user"
            )
        })
        .is_some_and(|e| e.kind != "assistant")
    {
        session
            .warnings
            .push("no final assistant response; session may be truncated or unfinished".into());
    }
    session.revision = hash(&serde_json::to_vec(&(
        hash(raw.as_bytes()),
        PARSER_VERSION,
        &session.redaction_fingerprint,
        &session.project_root,
    ))?);
    session.id = format!(
        "s_{}",
        &hash(
            format!(
                "{}:{}",
                session.agent,
                native_id.as_deref().unwrap_or(&session.revision)
            )
            .as_bytes()
        )[..20]
    );
    Ok(session)
}

pub fn import_path(path: &Path, config: &Config) -> Result<Vec<Session>> {
    let (sessions, errors) = import_batch(path, config)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"))
    }
    Ok(sessions)
}
pub fn import_batch(path: &Path, config: &Config) -> Result<(Vec<Session>, Vec<String>)> {
    if path.is_file() {
        return match parse_file(path, config) {
            Ok(s) => Ok((vec![s], vec![])),
            Err(e) => Ok((vec![], vec![format!("{}: {e}", path.display())])),
        };
    }
    if !path.is_dir() {
        bail!("import path does not exist")
    }
    let mut paths = vec![];
    let mut errors = vec![];
    for e in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(".jta" | ".git" | ".tools" | "target")
            )
        })
    {
        match e {
            Ok(e) => {
                if e.file_type().is_file()
                    && e.path()
                        .extension()
                        .is_some_and(|x| x == "jsonl" || x == "json")
                    && e.file_name() != "PROVENANCE.json"
                {
                    paths.push(e.into_path());
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    paths.sort();
    let mut sessions = vec![];
    let mut revisions = BTreeSet::new();
    for p in paths {
        match parse_file(&p, config) {
            Ok(s) => {
                if revisions.insert(s.revision.clone()) {
                    sessions.push(s)
                }
            }
            Err(e) => errors.push(format!("{}: {e}", p.display())),
        }
    }
    Ok((sessions, errors))
}

fn turns(events: &[Event], warnings: &mut Vec<String>) -> Vec<Turn> {
    let mut turns: Vec<Turn> = vec![];
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    let mut current = None;
    let mut last_line = 0;
    let mut last_message = None;
    for (index, e) in events.iter().enumerate() {
        if e.kind == "user" {
            current = None;
            continue;
        }
        let mut target = current;
        if e.kind == "assistant" || e.kind == "reasoning" || e.kind == "tool_call" {
            let same_message = e.message_id.is_some() && e.message_id == last_message;
            if current.is_none()
                || (e.line != last_line
                    && !same_message
                    && (e.kind == "assistant" || e.message_id.is_some()))
            {
                turns.push(Turn {
                    id: turns.len() as u32 + 1,
                    ..Default::default()
                });
                current = Some(turns.len() - 1);
            }
            target = current;
            last_line = e.line;
            last_message = e.message_id.clone();
            if let (Some(id), Some(t)) = (&e.call_id, target) {
                calls.insert(id.clone(), t);
            }
        } else if e.kind == "tool_result" {
            target = e.call_id.as_ref().and_then(|id| calls.remove(id));
            if target.is_none() {
                warnings.push(format!(
                    "line {}: orphan tool result (missing call ID or matching call)",
                    e.line
                ));
            }
        }
        if let Some(t) = target {
            let t = &mut turns[t];
            t.event_indices.push(index);
            if e.kind == "assistant" || e.kind == "reasoning" {
                if !t.intent.is_empty() {
                    t.intent.push('\n')
                }
                t.intent.push_str(&e.text);
            }
            if let Some(name) = &e.tool_name {
                if e.kind == "tool_call" {
                    t.tools.push(name.clone());
                }
            }
            if let Some(input) = &e.input {
                if let Some(cmd) = input.get("cmd").or_else(|| input.get("command")) {
                    let cmd = if let Some(a) = cmd.as_array() {
                        a.iter().map(content).collect::<Vec<_>>().join(" ")
                    } else {
                        content(cmd)
                    };
                    if [
                        "cargo test",
                        "pytest",
                        "npm test",
                        "go test",
                        "npx vitest",
                        "cargo check",
                    ]
                    .iter()
                    .any(|v| cmd.contains(v))
                    {
                        t.verification.push(format!("recorded command: {cmd}"));
                    }
                    t.commands.push(cmd);
                }
                if let Some(file) = input
                    .get("file_path")
                    .or_else(|| input.get("path"))
                    .and_then(Value::as_str)
                {
                    if e.tool_name.as_deref().is_some_and(|n| {
                        ["Write", "Edit", "MultiEdit", "write_file", "edit_file"].contains(&n)
                    }) {
                        t.files_changed.push(file.into());
                    } else {
                        t.files_read.push(file.into());
                    }
                }
                let patch = if input.is_string() {
                    input.as_str().unwrap_or("")
                } else {
                    input.get("patch").and_then(Value::as_str).unwrap_or("")
                };
                for line in patch.lines() {
                    for prefix in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                        if let Some(path) = line.strip_prefix(prefix) {
                            t.files_changed.push(path.to_owned());
                        }
                    }
                }
            }
            if e.is_error == Some(true)
                || (e.kind == "tool_result"
                    && serde_json::from_str::<Value>(&e.text)
                        .ok()
                        .and_then(|v| v["metadata"]["exit_code"].as_i64())
                        .is_some_and(|n| n != 0))
            {
                t.errors.push(e.text.clone());
            }
            if e.usage.is_some() {
                t.usage = e.usage.clone();
            }
        }
    }
    for (id, _) in calls {
        warnings.push(format!(
            "tool call {id}: missing result; evidence incomplete"
        ));
    }
    let reference_pattern=regex::Regex::new(r#"https?://[^\s\"'<>]+|(?:[A-Za-z0-9_.-]+/)+[A-Za-z0-9_.-]+|\b[A-Z][A-Z0-9_]*(?:Error|Exception)\b|\bE[0-9]{3,5}\b"#).expect("reference regex");
    for turn in &mut turns {
        let mut references = BTreeSet::new();
        for index in &turn.event_indices {
            let event = &events[*index];
            let text = format!(
                "{} {}",
                event.text,
                event
                    .input
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default()
            );
            for item in reference_pattern.find_iter(&text).take(256) {
                let value = item
                    .as_str()
                    .trim_end_matches(['.', ',', ':', ';', ')', ']']);
                if value.len() > 4 && value.len() < 512 {
                    references.insert(value.to_owned());
                }
            }
        }
        turn.references = references.into_iter().take(128).collect();
    }
    for i in 0..turns.len() {
        let stamps: Vec<_> = turns[i]
            .event_indices
            .iter()
            .filter_map(|j| events[*j].timestamp.as_ref())
            .filter_map(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .collect();
        if let (Some(a), Some(b)) = (stamps.iter().min(), stamps.iter().max()) {
            turns[i].duration_ms = Some((*b - *a).num_milliseconds().max(0) as u64);
        }
        for j in i + 1..turns.len() {
            let shared_references: Vec<String> = turns[i]
                .references
                .iter()
                .filter(|r| turns[j].references.contains(r))
                .take(16)
                .cloned()
                .collect();
            let shared = !shared_references.is_empty()
                || turns[i]
                    .files_read
                    .iter()
                    .chain(&turns[i].files_changed)
                    .any(|p| {
                        turns[j].files_read.contains(p)
                            || turns[j].files_changed.contains(p)
                            || turns[j].intent.contains(p)
                    })
                || turns[i]
                    .commands
                    .iter()
                    .any(|c| turns[j].commands.contains(c));
            if shared && turns[i].candidate_downstream.len() < 64 {
                let id = turns[j].id;
                turns[i].candidate_downstream.push(id);
                let link = DownstreamLink {
                    turn_id: id,
                    reason: "shared_reference: exact path, URL, error identifier, or command (candidate, not proven consumption)".into(),
                    evidence: if shared_references.is_empty() {turns[i].commands.iter().filter(|c|turns[j].commands.contains(c)).take(16).cloned().collect()} else {shared_references},
                };
                turns[i].downstream_links.push(link);
            }
            if !turns[i].errors.is_empty()
                && !turns[i].commands.is_empty()
                && turns[i].commands == turns[j].commands
            {
                let id = turns[i].id;
                turns[j].retry_of.push(id);
            }
        }
    }
    turns
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(raw: &str) -> Session {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("test.jsonl");
        fs::write(&p, raw).unwrap();
        parse_file(&p, &Config::default()).unwrap()
    }
    #[test]
    fn concurrent_results_retries_and_redaction() {
        let s = parse(
            r#"{"type":"session_meta","payload":{"id":"s"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"try"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"a","arguments":"{\"cmd\":\"cargo test\",\"api_key\":\"secretvalue\"}"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"parallel"}]}}
{"type":"response_item","payload":{"type":"function_call_output","call_id":"a","output":"{\"metadata\":{\"exit_code\":1},\"output\":\"failed\"}"}}
{"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"b","arguments":{"cmd":"cargo test"}}}
{"type":"response_item","payload":{"type":"function_call_output","call_id":"b","output":"ok"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
{"bad":"#,
        );
        assert_eq!(s.turns.len(), 3);
        assert!(!s.turns[0].errors.is_empty());
        assert_eq!(s.turns[1].retry_of, vec![1]);
        assert_eq!(s.turns[0].candidate_downstream, vec![2]);
        assert!(s.warnings.iter().any(|w| w.contains("malformed")));
        assert!(!serde_json::to_string(&s).unwrap().contains("secretvalue"));
    }
    #[test]
    fn claude_streaming_message_and_usage() {
        let s = parse(
            r#"{"type":"user","sessionId":"x","message":{"role":"user","content":"do work"}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"hmm"}],"usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4,"output_tokens":5}}}
{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"tool_use","id":"t","name":"Read","input":{"file_path":"src/a.rs"}}],"usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4,"output_tokens":5}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"hi"}]}}
{"type":"assistant","message":{"id":"m2","role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"src/a.rs"}}]}}
"#,
        );
        assert_eq!(s.turns.len(), 2);
        assert_eq!(s.turns[0].usage.as_ref().unwrap().input_tokens, Some(9));
        assert_eq!(s.turns[0].candidate_downstream, vec![2]);
        assert!(s.warnings.iter().any(|w| w.contains("missing result")));
    }
    #[test]
    fn native_ids_are_agent_namespaced() {
        let claude = parse(
            r#"{"type":"user","sessionId":"same-uuid","message":{"role":"user","content":"hello"}}"#,
        );
        let codex = parse(
            r#"{"type":"session_meta","payload":{"id":"same-uuid"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#,
        );
        assert_ne!(claude.id, codex.id);
    }
    #[test]
    fn derived_project_attribution_changes_revision_not_identity() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let cwd = project.join("nested");
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        let source = temp.path().join("session.jsonl");
        fs::write(&source,serde_json::json!({"type":"user","sessionId":"stable-id","cwd":cwd,"message":{"role":"user","content":"hello"}}).to_string()).unwrap();
        let before = parse_file(&source, &Config::default()).unwrap();
        assert_eq!(
            before.project_root,
            Some(
                project
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
        fs::write(cwd.join(".git"), "gitdir: ../worktree-state").unwrap();
        let after = parse_file(&source, &Config::default()).unwrap();
        assert_eq!(before.id, after.id);
        assert_ne!(before.project_root, after.project_root);
        assert_ne!(before.revision, after.revision);
        assert_eq!(
            after.revision,
            parse_file(&source, &Config::default()).unwrap().revision
        );
    }
    #[test]
    fn stable_identity_revisions_and_redaction() {
        let raw = r#"{"type":"user","sessionId":"x","message":{"role":"user","content":"hi"}}"#;
        let a = parse(raw);
        let b = parse(raw);
        assert_eq!(a.id, b.id);
        assert_eq!(a.revision, b.revision);
        let c = parse(&format!("{raw}\n"));
        assert_eq!(a.id, c.id);
        assert_ne!(a.revision, c.revision);
    }
    #[test]
    fn delayed_discovery_reference() {
        let mut events = vec![];
        for id in 0..7 {
            events.push(Event {
                line: id * 2 + 1,
                kind: "assistant".into(),
                text: format!("step {id}"),
                ..Default::default()
            });
            events.push(Event{line:id*2+2,kind:"tool_call".into(),tool_name:Some("shell".into()),input:Some(serde_json::json!({"cmd":if id==0{"rg --files src/retry.rs"}else if id==6{"cat src/retry.rs"}else{"pwd"}})),..Default::default()});
        }
        let turns = turns(&events, &mut vec![]);
        assert!(turns[0].candidate_downstream.contains(&7));
        assert!(turns[0]
            .downstream_links
            .iter()
            .any(|l| l.turn_id == 7 && l.evidence.contains(&"src/retry.rs".into())));
        assert!(turns[0].files_read.is_empty());
    }
    #[test]
    fn partial_batch_and_redaction_revision() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("valid.jsonl");
        fs::write(
            &path,
            r#"{"type":"user","sessionId":"x","message":{"role":"user","content":"hello"}}"#,
        )
        .unwrap();
        fs::write(d.path().join("bad.jsonl"), "not json").unwrap();
        let (sessions, errors) = import_batch(d.path(), &Config::default()).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(errors.len(), 1);
        let mut config = Config::default();
        config.redaction.patterns.push("hello".into());
        let changed = parse_file(&path, &config).unwrap();
        assert_eq!(sessions[0].id, changed.id);
        assert_ne!(sessions[0].revision, changed.revision);
    }
    #[test]
    fn public_fixtures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/public");
        let sessions = import_path(&root, &Config::default()).unwrap();
        assert!(sessions.len() >= 7);
        for s in sessions {
            assert!(!s.turns.is_empty(), "{}", s.source_path);
            if s.source_path.contains("claude-") {
                let n = if s.source_path.contains("4c09") {
                    1
                } else if s.source_path.contains("674e") {
                    10
                } else {
                    11
                };
                assert_eq!(s.turns.len(), n, "{}", s.source_path);
            }
        }
    }
}
