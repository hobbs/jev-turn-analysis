use jta::{
    config::{analysis_fingerprint, Config},
    model::{
        Analysis, Distribution, Session, Turn, TurnJudgment, SESSION_QUESTIONS, TURN_QUESTIONS,
    },
    store::Workspace,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    process::{Command, Output},
};
fn invoke(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(root)
        .args(args)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap()
}
fn data(o: Output) -> Value {
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice::<Value>(&o.stdout).unwrap()["data"].clone()
}
fn dist(answers: &[&str], selected: &str) -> Distribution {
    Distribution {
        selected: selected.into(),
        probabilities: answers
            .iter()
            .map(|a| (a.to_string(), if *a == selected { 1.0 } else { 0.0 }))
            .collect(),
        ..Default::default()
    }
}
fn seed(root: &std::path::Path) -> Workspace {
    let c = Config::default();
    let w = Workspace::init(root, &c).unwrap();
    let s = Session {
        id: "s_test".into(),
        revision: "rev1".into(),
        source_path: "/original/never-delete.jsonl".into(),
        agent: "codex".into(),
        repository: Some("/work/test".into()),
        imported_at: "2020-01-01T00:00:00Z".into(),
        started_at: Some("2020-01-01T00:00:00Z".into()),
        ended_at: Some("2020-01-01T00:01:00Z".into()),
        turns: vec![Turn {
            id: 1,
            intent: "Verify implementation".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    w.save_session(&s).unwrap();
    let mut answers = BTreeMap::new();
    for (q, options) in TURN_QUESTIONS {
        answers.insert(
            q.to_string(),
            dist(
                options,
                if *q == "opportunity" {
                    "missed_verification"
                } else {
                    options[0]
                },
            ),
        );
    }
    let a = Analysis {
        id: "a_test".into(),
        session_id: s.id,
        revision: s.revision,
        created_at: "2020-01-01T00:02:00Z".into(),
        config_fingerprint: analysis_fingerprint(&c),
        rubric_version: jta::model::RUBRIC_VERSION.into(),
        session: SESSION_QUESTIONS
            .iter()
            .map(|(q, opts)| (q.to_string(), dist(opts, opts[0])))
            .collect(),
        turns: BTreeMap::from([(
            1,
            TurnJudgment {
                answers,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    w.save_analysis(&a).unwrap();
    w
}
#[test]
fn offline_lifecycle_cache_snapshot_labels_purge() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let r = data(invoke(temp.path(), &["report"]));
    assert_eq!(r["sessions"], 1);
    assert_eq!(r["counts"]["task_outcome"]["complete"], 1);
    let run = data(invoke(temp.path(), &["analyze", "--all"]));
    assert_eq!(run["analysis_ids"], json!(["a_test"]));
    assert_eq!(w.analyses().unwrap().len(), 1);
    data(invoke(temp.path(), &["snapshot", "before"]));
    let mut s = w.session("s_test", None).unwrap();
    s.revision = "rev2".into();
    w.save_session(&s).unwrap();
    assert_eq!(data(invoke(temp.path(), &["report"]))["sessions"], 0);
    let compared = data(invoke(temp.path(), &["compare", "before", "before"]));
    assert_eq!(compared["before"]["sessions"], 1);
    assert!(compared["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("Overlapping")));
    let shown = data(invoke(
        temp.path(),
        &[
            "show",
            "s_test",
            "--turn",
            "1",
            "--run",
            run["id"].as_str().unwrap(),
        ],
    ));
    assert_eq!(
        shown["judgment"]["answers"]["usefulness"]["selected"],
        "essential"
    );
    s.revision = "rev1".into();
    w.save_session(&s).unwrap();
    let path = temp.path().join("labels.jsonl");
    data(invoke(
        temp.path(),
        &[
            "labels",
            "export",
            "--sample",
            "20",
            "--output",
            path.to_str().unwrap(),
        ],
    ));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut edited = String::new();
    for line in text.lines() {
        let mut v: Value = serde_json::from_str(line).unwrap();
        if v["turn_id"].is_null() {
            v["human_labels"] = json!({"task_outcome":"complete"});
        } else {
            v["human_labels"] = json!({"usefulness":"useful"});
        }
        edited.push_str(&v.to_string());
        edited.push('\n');
    }
    std::fs::write(&path, edited).unwrap();
    data(invoke(
        temp.path(),
        &["labels", "import", path.to_str().unwrap()],
    ));
    let cal = data(invoke(temp.path(), &["calibration"]));
    assert_eq!(cal["questions"]["task_outcome"]["agreement"], 1.0);
    assert_eq!(cal["disagreements"].as_array().unwrap().len(), 1);
    let preview = data(invoke(
        temp.path(),
        &["purge", "--older-than", "30d", "--dry-run"],
    ));
    assert!(preview["affected_objects"].as_array().unwrap().len() >= 7);
    assert_eq!(w.sessions().unwrap().len(), 1);
    data(invoke(temp.path(), &["purge", "--older-than", "30d"]));
    assert!(w.sessions().unwrap().is_empty());
    assert!(w.list_json::<Value>("snapshots").unwrap().is_empty());
    assert!(w.list_json::<Value>("runs").unwrap().is_empty());
    assert!(w.list_json::<Value>("labels").unwrap().is_empty());
}
#[test]
fn init_openrouter_and_invalid_args() {
    let temp = tempfile::tempdir().unwrap();
    data(invoke(
        temp.path(),
        &["init", "--review-provider", "openrouter"],
    ));
    let w = Workspace::discover(Some(temp.path())).unwrap();
    let c = w.config().unwrap();
    assert_eq!(c.review.api_key_env, "OPENROUTER_API_KEY");
    assert_eq!(
        c.review.endpoint,
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(
        invoke(temp.path(), &["analyze", "--all", "--project", "."])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        invoke(temp.path(), &["report", "--min-confidence", "1.5"])
            .status
            .code(),
        Some(2)
    );
    let preview = data(invoke(temp.path(), &["review", "--dry-run"]));
    assert_eq!(preview["provider"], "openrouter");
}
#[test]
fn isolated_review_is_offline_and_recurring_default_excludes_single_session() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    let preview = data(invoke(temp.path(), &["review", "--dry-run"]));
    let evidence: Value = serde_json::from_str(
        preview["request"]["messages"][1]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 0);
    let preview = data(invoke(
        temp.path(),
        &["review", "--session", "s_test", "--dry-run"],
    ));
    let evidence: Value = serde_json::from_str(
        preview["request"]["messages"][1]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["isolated"], true);
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 1);
}
#[test]
fn purge_preserves_new_revision_and_its_analysis() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut s = w.session("s_test", None).unwrap();
    s.revision = "fresh".into();
    s.imported_at = chrono::Utc::now().to_rfc3339();
    w.save_session(&s).unwrap();
    let mut a: Analysis = w.load_json("analyses", "a_test").unwrap();
    a.id = "a_fresh".into();
    a.revision = s.revision.clone();
    w.save_analysis(&a).unwrap();
    data(invoke(temp.path(), &["purge", "--older-than", "30d"]));
    assert_eq!(w.session("s_test", None).unwrap().revision, "fresh");
    assert!(w.session("s_test", Some("rev1")).is_err());
    assert_eq!(w.analyses().unwrap().len(), 1);
    assert_eq!(w.analyses().unwrap()[0].id, "a_fresh");
}
#[test]
fn label_import_drops_unknown_fields_and_rejects_bad_turn() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let path = temp.path().join("labels.jsonl");
    let mut label = json!({"session_id":"s_test","revision":"rev1","analysis_id":"a_test","turn_id":null,"human_labels":{"task_outcome":"complete"},"untrusted_extra":"unredacted private data"});
    std::fs::write(&path, label.to_string()).unwrap();
    data(invoke(
        temp.path(),
        &["labels", "import", path.to_str().unwrap()],
    ));
    let stored = w.list_json::<Value>("labels").unwrap();
    assert!(stored[0].get("untrusted_extra").is_none());
    label["turn_id"] = json!(-1);
    std::fs::write(&path, label.to_string()).unwrap();
    assert_eq!(
        invoke(temp.path(), &["labels", "import", path.to_str().unwrap()])
            .status
            .code(),
        Some(1)
    );
}
#[test]
fn partial_import_keeps_valid_sessions_and_emits_json() {
    let temp = tempfile::tempdir().unwrap();
    Workspace::init(temp.path(), &Config::default()).unwrap();
    let inputs = temp.path().join("inputs");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("valid.jsonl"),"{\"type\":\"session_meta\",\"payload\":{\"id\":\"public-test\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}}\n").unwrap();
    std::fs::write(
        inputs.join("unsupported.jsonl"),
        "{\"not_a_transcript\":true}\n",
    )
    .unwrap();
    let output = invoke(temp.path(), &["import", inputs.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["data"]["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["failures"].as_array().unwrap().len(), 1);
}
#[test]
fn recurring_review_bounds_corpus_sample_and_support_refs() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let base = w.session("s_test", None).unwrap();
    let a: Analysis = w.load_json("analyses", "a_test").unwrap();
    for i in 0..30 {
        let mut s = base.clone();
        s.id = format!("s_corpus{i:02}");
        w.save_session(&s).unwrap();
        let mut analysis = a.clone();
        analysis.id = format!("a_corpus{i:02}");
        analysis.session_id = s.id;
        w.save_analysis(&analysis).unwrap();
    }
    let preview = data(invoke(temp.path(), &["review", "--dry-run"]));
    let evidence: Value = serde_json::from_str(
        preview["request"]["messages"][1]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 3);
    assert_eq!(evidence["omitted_supporting_sessions"], 28);
    assert_eq!(evidence["patterns"][0]["corpus_sessions"], 31);
    assert_eq!(evidence["patterns"][0]["selected_sessions"], 3);
    for r in evidence["patterns"][0]["supporting"].as_array().unwrap() {
        assert!(evidence["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["session_id"] == r["session_id"]
                && s["turns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["turn_id"] == r["turn_id"])));
    }
}
#[test]
fn review_skips_insufficient_support_without_discarding_eligible_proposals() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    for case in ["mixed", "all_skipped", "isolated", "unknown_reference"] {
        let temp = tempfile::tempdir().unwrap();
        let w = seed(temp.path());
        let base = w.session("s_test", None).unwrap();
        let base_analysis: Analysis = w.load_json("analyses", "a_test").unwrap();
        for id in ["s_second", "s_third"] {
            let mut s = base.clone();
            s.id = id.into();
            w.save_session(&s).unwrap();
            let mut a = base_analysis.clone();
            a.id = format!("a_{id}");
            a.session_id = s.id;
            w.save_analysis(&a).unwrap();
        }
        let proposal = |ids: &[&str]| {
            json!({
                "title":"Verify changes", "observed_pattern":"Checks omitted",
                "outcome_effect":"Completion unverified", "uncertainty":"Limited evidence",
                "counterexamples":[], "remediation_surface":"AGENTS.md",
                "proposed_change":"Run relevant checks", "scope":"Coding tasks",
                "risk":"Additional latency", "evaluation_plan":"Measure verified outcomes",
                "supporting_refs":ids.iter().map(|id|json!({"session_id":id,"turn_id":1})).collect::<Vec<_>>()
            })
        };
        // Repeating a session reference must not count as distinct support.
        let insufficient = proposal(&["s_test", "s_test", "s_test"]);
        let eligible = proposal(&["s_test", "s_second", "s_third"]);
        let proposals = match case {
            "mixed" => vec![insufficient, eligible],
            "all_skipped" | "isolated" => vec![insufficient],
            _ => vec![eligible, proposal(&["s_test", "s_second", "s_unknown"])],
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = w.config().unwrap();
        c.review.provider = "openai".into();
        c.review.endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        c.review.api_key_env = "JTA_TEST_REVIEW_KEY".into();
        std::fs::write(
            w.data_dir().join("config.json"),
            serde_json::to_vec(&c).unwrap(),
        )
        .unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(pos) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let length: usize = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < end + length {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
            let minimum = if case == "isolated" { 1 } else { 3 };
            assert!(request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains(&format!("at least {minimum} distinct session IDs")));
            let result = json!({"recommendations":proposals});
            let response = json!({"choices":[{"finish_reason":"stop","message":{"content":result.to_string()}}]}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_jta"));
        command
            .args([
                "--workspace",
                temp.path().to_str().unwrap(),
                "review",
                "--format",
                "json",
            ])
            .env("JTA_TEST_REVIEW_KEY", "test-key");
        if case == "isolated" {
            command.args(["--session", "s_test"]);
        }
        let output = command.output().unwrap();
        handle.join().unwrap();
        if case == "unknown_reference" {
            assert!(!output.status.success());
            assert!(w.list_json::<Value>("recommendations").unwrap().is_empty());
            continue;
        }
        let result = data(output);
        let expected_saved = usize::from(case != "all_skipped");
        assert_eq!(
            result["recommendations"].as_array().unwrap().len(),
            expected_saved
        );
        assert_eq!(
            w.list_json::<Value>("recommendations").unwrap().len(),
            expected_saved
        );
        if case == "isolated" {
            assert_eq!(result["recommendations"][0]["isolated"], true);
            assert!(result["skipped_recommendations"]
                .as_array()
                .unwrap()
                .is_empty());
        } else {
            assert_eq!(
                result["skipped_recommendations"][0]["supporting_sessions"],
                1
            );
            assert_eq!(result["skipped_recommendations"][0]["required_sessions"], 3);
            assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        }
        if case == "all_skipped" {
            assert!(result["message"]
                .as_str()
                .unwrap()
                .contains("No recommendations met"));
        }
    }
}
#[test]
fn comparison_uses_proportions_and_handles_empty_denominators() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    for (name, complete, total) in [("small", 2, 2), ("large", 3, 4), ("empty", 0, 0)] {
        let snapshot = json!({"id":name,"created_at":"2026-01-01T00:00:00Z","filters":jta::analytics::Filters::default(),"pairs":[],"report":{"sessions":total,"counts":{"task_outcome":{"complete":complete,"failed":total-complete},"outcome_verification":{"verified":complete,"known_incomplete":total-complete},"user_intervention":{"none":complete,"corrected_agent":total-complete}}}});
        w.save_json("snapshots", name, &snapshot).unwrap();
    }
    let comparison = data(invoke(temp.path(), &["compare", "small", "large"]));
    let metric = &comparison["metrics"]["task_outcome"]["complete"];
    assert_eq!(metric["before"]["count"], 2);
    assert_eq!(metric["after"]["count"], 3);
    assert_eq!(metric["before"]["denominator"], 2);
    assert_eq!(metric["after"]["denominator"], 4);
    assert_eq!(metric["delta_percentage_points"], -25.0);
    assert!(metric["before"]["wilson95"]["lower"].as_f64().unwrap() < 1.0);
    assert_eq!(metric["before"]["wilson95"]["upper"], 1.0);
    let empty = data(invoke(temp.path(), &["compare", "empty", "large"]));
    assert!(empty["metrics"]["task_outcome"]["complete"]["before"]["proportion"].is_null());
    assert!(empty["metrics"]["task_outcome"]["complete"]["delta_percentage_points"].is_null());
}
fn native_fixture(root: &std::path::Path, project: &std::path::Path) {
    let codex = root.join("codex/sessions");
    let claude = root.join("claude/projects/opaque-dir");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    let lines = [
        json!({"type":"session_meta","payload":{"id":"native-codex","cwd":project}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Verified native fixture"}]}}),
    ];
    std::fs::write(
        codex.join("sample.jsonl"),
        lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let lines = [
        json!({"type":"user","sessionId":"native-claude","cwd":project,"message":{"role":"user","content":"Inspect this fixture"}}),
        json!({"type":"assistant","sessionId":"native-claude","cwd":project,"message":{"role":"assistant","content":[{"type":"text","text":"Checked fixture"}]}}),
    ];
    std::fs::write(
        claude.join("sample.jsonl"),
        lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}
fn native_command(root: &std::path::Path, cwd: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jta"))
        .current_dir(cwd)
        .args(args)
        .arg("--codex-home")
        .arg(root.join("codex"))
        .arg("--claude-config-dir")
        .arg(root.join("claude"))
        .args(["--format", "json"])
        .output()
        .unwrap()
}
#[test]
fn project_discovery_is_offline_combined_scoped_and_autoinitializes_root() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let nested = project.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir(project.join(".git")).unwrap();
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::create_dir(other.join(".git")).unwrap();
    native_fixture(temp.path(), &nested);
    // An unrelated enclosing workspace must not capture a checkout's default analysis.
    Workspace::init(temp.path(), &Config::default()).unwrap();
    let found = data(native_command(temp.path(), &nested, &["discover"]));
    assert_eq!(found["selected_sessions"], 2);
    assert_eq!(
        found["project_root"],
        project.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert!(!project.join(".jta").exists());
    let only = data(native_command(
        temp.path(),
        &nested,
        &["discover", "--agent", "claude"],
    ));
    assert_eq!(only["selected_sessions"], 1);
    assert_eq!(only["sessions"][0]["agent"], "claude_code");
    let unrelated = data(native_command(
        temp.path(),
        &nested,
        &["discover", "--project", other.to_str().unwrap()],
    ));
    assert_eq!(unrelated["selected_sessions"], 0);
    let analyzed = data(native_command(
        temp.path(),
        &nested,
        &["analyze", "--dry-run"],
    ));
    assert_eq!(analyzed["selection"]["selected_sessions"], 2);
    assert!(project.join(".jta/config.json").exists());
    assert!(!nested.join(".jta").exists());
    let w = Workspace::discover(Some(&project)).unwrap();
    assert_eq!(w.sessions().unwrap().len(), 2);
    assert!(w
        .sessions()
        .unwrap()
        .iter()
        .all(|s| s.project_root.as_deref()
            == Some(project.canonicalize().unwrap().to_str().unwrap())));
    assert_eq!(
        Workspace::discover(Some(temp.path()))
            .unwrap()
            .sessions()
            .unwrap()
            .len(),
        0
    );
    let selected = data(native_command(
        temp.path(),
        &nested,
        &["analyze", "--dry-run", "--agent", "codex"],
    ));
    assert_eq!(selected["payloads"].as_array().unwrap().len(), 1);
    let from_all = data(invoke(
        &project,
        &["analyze", "--all", "--agent", "claude", "--dry-run"],
    ));
    assert_eq!(from_all["payloads"].as_array().unwrap().len(), 1);
}
#[test]
fn explicit_workspace_does_not_change_discovery_project_and_init_uses_root() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let nested = project.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(project.join(".git"), "gitdir: ../worktree-meta").unwrap();
    let storage = temp.path().join("storage");
    native_fixture(temp.path(), &project);
    let v = data(native_command(
        temp.path(),
        &nested,
        &[
            "--workspace",
            storage.to_str().unwrap(),
            "analyze",
            "--dry-run",
        ],
    ));
    assert_eq!(v["selection"]["selected_sessions"], 2);
    assert_eq!(
        Workspace::discover(Some(&storage))
            .unwrap()
            .sessions()
            .unwrap()
            .len(),
        2
    );
    assert!(!project.join(".jta").exists());
    let init = Command::new(env!("CARGO_BIN_EXE_jta"))
        .current_dir(&nested)
        .args(["init", "--format", "json"])
        .output()
        .unwrap();
    data(init);
    assert!(project.join(".jta/config.json").exists());
    assert!(!nested.join(".jta").exists());
}
#[test]
fn parser_migration_retires_only_legacy_current_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut old = w.session("s_test", None).unwrap();
    old.parser_version = "1".into();
    w.save_session(&old).unwrap();
    let mut current = old.clone();
    current.id = "s_namespaced".into();
    current.revision = "newparser".into();
    current.parser_version = "2".into();
    w.save_session(&current).unwrap();
    assert_eq!(w.sessions().unwrap().len(), 1);
    assert_eq!(w.sessions().unwrap()[0].id, "s_namespaced");
    assert_eq!(w.session("s_test", Some("rev1")).unwrap().id, "s_test");
    assert_eq!(w.analyses().unwrap().len(), 1);
}
#[test]
fn unreadable_native_source_is_partial_not_complete() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir_all(temp.path().join("codex/sessions")).unwrap();
    std::fs::write(
        temp.path().join("codex/sessions/unreadable.jsonl"),
        [255u8, 254],
    )
    .unwrap();
    let output = native_command(temp.path(), &project, &["discover"]);
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["data"]["selected_sessions"], 0);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(v["data"]["complete"], false);
    assert!(!v["data"]["errors"].as_array().unwrap().is_empty());
    assert!(!project.join(".jta").exists());
}
#[test]
fn single_source_explicit_root_needs_no_home_and_claude_alias_works() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    native_fixture(temp.path(), &project);
    for (agent, flag, folder) in [
        ("codex", "--codex-home", "codex"),
        ("claude_code", "--claude-config-dir", "claude"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_jta"))
            .current_dir(&project)
            .env_remove("HOME")
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .args(["discover", "--agent", agent, flag])
            .arg(temp.path().join(folder))
            .args(["--format", "json"])
            .output()
            .unwrap();
        let found = data(output);
        assert_eq!(found["selected_sessions"], 1);
        assert_eq!(found["sessions"][0]["agent"], agent);
    }
}
#[test]
fn human_report_shows_per_agent_outcomes_and_token_coverage() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    let output = Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(temp.path())
        .arg("report")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("codex: 1 sessions, 1 turns · complete 1, verified 1"));
    assert!(text.contains("Observed input tokens: unknown (coverage 0/1 turns)"));
    let review = data(invoke(
        temp.path(),
        &["review", "--agent", "claude_code", "--dry-run"],
    ));
    assert!(review["request"].is_object());
}
#[test]
fn non_git_discovery_scope_changes_preserve_immutable_revision_history() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("project");
    let child = parent.join("component");
    let cwd = child.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let storage = temp.path().join("shared");
    native_fixture(temp.path(), &cwd);
    let analyze_scope = |scope: &std::path::Path| {
        data(native_command(
            temp.path(),
            &cwd,
            &[
                "--workspace",
                storage.to_str().unwrap(),
                "analyze",
                "--dry-run",
                "--agent",
                "codex",
                "--project",
                scope.to_str().unwrap(),
            ],
        ));
    };
    analyze_scope(&parent);
    let workspace = Workspace::discover(Some(&storage)).unwrap();
    let first = workspace.sessions().unwrap().remove(0);
    assert_eq!(
        first.project_root.as_deref(),
        parent.canonicalize().unwrap().to_str()
    );
    analyze_scope(&child);
    let second = workspace.sessions().unwrap().remove(0);
    assert_eq!(first.id, second.id);
    assert_ne!(first.revision, second.revision);
    assert_eq!(
        second.project_root.as_deref(),
        child.canonicalize().unwrap().to_str()
    );
    assert_eq!(
        workspace
            .session(&first.id, Some(&first.revision))
            .unwrap()
            .project_root,
        first.project_root
    );
    analyze_scope(&child);
    assert_eq!(workspace.sessions().unwrap()[0].revision, second.revision);
    assert_eq!(
        workspace.list_json::<Session>("revisions").unwrap().len(),
        2
    );
}
