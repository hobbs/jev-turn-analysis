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
        .env("PATH", fake_path(root))
        .arg("--format")
        .arg("json")
        .output()
        .unwrap()
}
fn fake_path(root: &std::path::Path) -> std::ffi::OsString {
    let bin = root.join(".jta/test-bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["codex", "claude"] {
        let path = bin.join(name);
        if !path.exists() {
            std::fs::write(&path, include_str!("support/fake_review_cli.py")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}
fn data(o: Output) -> Value {
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    serde_json::from_slice::<Value>(&o.stdout).unwrap()["data"].clone()
}
fn report_data(output: Output) -> Value {
    let receipt = data(output);
    assert_eq!(receipt.as_object().unwrap().len(), 2);
    assert!(receipt["report_path"].as_str().unwrap().ends_with(".md"));
    serde_json::from_slice(&std::fs::read(receipt["data_path"].as_str().unwrap()).unwrap()).unwrap()
}
fn generated_markdown(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let path = stdout
        .trim()
        .strip_prefix("Generated Markdown report: ")
        .unwrap();
    std::fs::read_to_string(path).unwrap()
}
#[test]
fn saved_recommendations_expose_action_and_evaluation_in_readable_formats() {
    let temp = tempfile::tempdir().unwrap();
    let w = Workspace::init(temp.path(), &Config::default()).unwrap();
    // Grounded proposals must expose literal edits as well as their rationale.
    let proposal = json!({
        "id":"r_existing", "title":"Verify exported artifacts",
        "project_root":"/project",
        "targets":[{"path":"/project/AGENTS.md","action":"edit","before":"Run npm test.",
            "after":"Run npm test and inspect the exported PPTX.","rationale":"The project delivers PPTX files.",
            "context_refs":[{"path":"/project/README.md","quote":"Export a PPTX"}]}],
        "remediation_surface":"AGENTS.md",
        "proposed_change":"Before claiming export success, open the generated artifact.",
        "scope":"Export changes", "observed_pattern":"Only a build was checked",
        "outcome_effect":"Artifact usability unknown",
        "evaluation_plan":"Replay the export task; pass when the artifact opens.",
        "risk":"Additional latency", "uncertainty":"Limited evidence",
        "counterexamples":["Documentation-only edits need no export."],
        "supporting_refs":[{"session_id":"s_example","turn_id":7}],
        "source_evidence":{"transcript":"RAW_EVIDENCE_SENTINEL"}
    });
    w.save_json("recommendations", "r_existing", &proposal)
        .unwrap();
    for format in ["text", "markdown"] {
        let output = Command::new(env!("CARGO_BIN_EXE_jta"))
            .arg("--workspace")
            .arg(temp.path())
            .env("PATH", fake_path(temp.path()))
            .args(["recommendations", "--format", format])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        for key in [
            "title",
            "remediation_surface",
            "proposed_change",
            "scope",
            "observed_pattern",
            "outcome_effect",
            "evaluation_plan",
            "risk",
            "uncertainty",
        ] {
            assert!(
                text.contains(proposal[key].as_str().unwrap()),
                "missing {key}: {text}"
            );
        }
        assert!(text.contains("Documentation-only edits need no export."));
        assert!(text.contains("jta show s_example --turn 7"));
        assert!(!text.contains("RAW_EVIDENCE_SENTINEL"));
        assert!(text.contains("Edit: /project/AGENTS.md"));
        assert!(text.contains("Run npm test and inspect the exported PPTX."));
    }
    let saved = data(invoke(temp.path(), &["recommendations"]));
    assert_eq!(saved, json!([proposal]));
}
#[test]
fn legacy_generic_recommendations_are_retained_but_not_presented_as_guidance() {
    let temp = tempfile::tempdir().unwrap();
    let w = Workspace::init(temp.path(), &Config::default()).unwrap();
    let legacy =
        json!({"id":"r_legacy","title":"Generic advice","proposed_change":"Verify everything"});
    w.save_json("recommendations", "r_legacy", &legacy).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(temp.path())
        .env("PATH", fake_path(temp.path()))
        .arg("recommendations")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("no project file targets"));
    assert!(!text.contains("Verify everything"));
    assert_eq!(data(invoke(temp.path(), &["show", "r_legacy"])), legacy);
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
    let r = report_data(invoke(temp.path(), &["report"]))["statistics"].clone();
    assert_eq!(r["sessions"], 1);
    assert_eq!(r["counts"]["task_outcome"]["complete"], 1);
    let run = data(invoke(temp.path(), &["analyze", "--all"]));
    assert_eq!(run["analysis_ids"], json!(["a_test"]));
    assert_eq!(run["selection"]["api_calls"], 0);
    assert_eq!(run["selection"]["planned_requests"], 0);
    assert_eq!(run["selection"]["cached_sessions"], 1);
    assert_eq!(run["report"]["sessions"], 1);
    assert_eq!(w.analyses().unwrap().len(), 1);
    data(invoke(temp.path(), &["snapshot", "before"]));
    let mut s = w.session("s_test", None).unwrap();
    s.revision = "rev2".into();
    w.save_session(&s).unwrap();
    assert_eq!(
        report_data(invoke(temp.path(), &["report"]))["statistics"]["sessions"],
        0
    );
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
fn init_claude_and_invalid_args() {
    let temp = tempfile::tempdir().unwrap();
    data(invoke(temp.path(), &["init", "--review-backend", "claude"]));
    let w = Workspace::discover(Some(temp.path())).unwrap();
    let c = w.config().unwrap();
    assert_eq!(c.review.backend, "claude");
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
    let preview = data(invoke(temp.path(), &["report", "--dry-run"]));
    assert_eq!(preview["backend"], "claude");
}
#[test]
fn small_corpus_defaults_to_preliminary_review_with_strict_opt_out() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    let preview = data(invoke(temp.path(), &["report", "--dry-run"]));
    let evidence = preview["investigations"][0]["initial"].clone();
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["preliminary"], true);
    assert_eq!(preview["selection"]["minimum_sessions"], 1);
    let strict = data(invoke(
        temp.path(),
        &["report", "--recurring-only", "--dry-run"],
    ));
    assert_eq!(strict["selection"]["sampled_sessions"], 0);
    assert_eq!(strict["selection"]["preliminary"], false);
    let skipped = report_data(invoke(temp.path(), &["report", "--recurring-only"]));
    assert_eq!(skipped["cli_invocations"], 0);
    assert!(skipped["message"]
        .as_str()
        .unwrap()
        .contains("without --recurring-only"));
    let preview = data(invoke(
        temp.path(),
        &["report", "--session", "s_test", "--dry-run"],
    ));
    let evidence = preview["investigations"][0]["initial"].clone();
    assert_eq!(evidence["isolated"], true);
    assert_eq!(evidence["preliminary"], false);
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 1);
}
#[test]
fn review_preview_includes_fresh_project_files_and_explicit_context() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    std::fs::write(temp.path().join("CLAUDE.md"), "Use the export skill.").unwrap();
    let extra = temp.path().join("tool.ts");
    std::fs::write(
        &extra,
        "export function deck() {}\n// password=private-value",
    )
    .unwrap();
    for instruction in [
        "Use the export skill.",
        "Read .claude/skills/export/SKILL.md before export.",
    ] {
        std::fs::write(temp.path().join("CLAUDE.md"), instruction).unwrap();
        let preview = data(invoke(
            temp.path(),
            &[
                "report",
                "--session",
                "s_test",
                "--context",
                extra.to_str().unwrap(),
                "--dry-run",
            ],
        ));
        let context = &preview["investigations"][0]["project_context"];
        assert!(context.to_string().contains(instruction));
        assert!(context.to_string().contains("export function deck()"));
        assert!(!context.to_string().contains("private-value"));
        assert_eq!(
            context["projects"][0]["project_root"],
            temp.path().canonicalize().unwrap().to_str().unwrap()
        );
    }
}
#[test]
fn review_without_project_files_retains_findings_without_edits() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut session = w.session("s_test", None).unwrap();
    session.project_root = Some(temp.path().join("missing").to_string_lossy().into_owned());
    w.save_session(&session).unwrap();
    let result = report_data(invoke(temp.path(), &["report", "--session", "s_test"]));
    assert!(result["recommendations"].as_array().unwrap().is_empty());
    assert_eq!(result["findings"].as_array().unwrap().len(), 1);
    assert_eq!(result["cli_invocations"], 2);
    let md = std::fs::read_to_string(result["report_path"].as_str().unwrap()).unwrap();
    assert!(md.contains("Verification needs clearer evidence"));
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
    let preview = data(invoke(temp.path(), &["report", "--dry-run"]));
    let evidence = preview["investigations"][0]["initial"].clone();
    assert_eq!(evidence["sessions"].as_array().unwrap().len(), 4);
    assert_eq!(evidence["omitted_supporting_sessions"], 27);
    assert_eq!(evidence["patterns"][0]["corpus_sessions"], 31);
    assert_eq!(evidence["patterns"][0]["selected_sessions"], 4);
    let report = report_data(invoke(temp.path(), &["report"]));
    assert_eq!(report["statistics"]["sessions"], 31);
    assert_eq!(report["statistics"]["turns"], 31);
    assert_eq!(report["selection"]["sampled_sessions"], 4);
    assert_eq!(report["sources"].as_array().unwrap().len(), 31);
    let markdown = std::fs::read_to_string(report["report_path"].as_str().unwrap()).unwrap();
    assert!(markdown.contains("| 31 | 31 | 4 |"));
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
fn combined_report_rejects_review_and_handles_filters_and_empty_corpus() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let removed = invoke(temp.path(), &["review"]);
    assert_eq!(removed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed.stderr).contains("unrecognized subcommand"));
    let preview = data(invoke(
        temp.path(),
        &["report", "--dry-run", "--opportunity", "none"],
    ));
    assert_eq!(preview["selection"]["eligible_patterns"], 0);
    assert!(preview["contract"]["prompt"]
        .as_str()
        .unwrap()
        .contains("Investigate before drafting"));
    assert!(w.list_json::<Value>("reports").unwrap().is_empty());
    let empty = report_data(invoke(temp.path(), &["report", "--agent", "claude"]));
    assert_eq!(empty["statistics"]["sessions"], 0);
    assert_eq!(empty["cli_invocations"], 0);
    assert!(empty["recommendations"].as_array().unwrap().is_empty());
    let markdown = std::fs::read_to_string(empty["report_path"].as_str().unwrap()).unwrap();
    assert!(markdown.contains("Model review was skipped"));
    assert!(!markdown.contains("No actionable recommendations in this review"));
    for format in ["text", "markdown"] {
        let output = Command::new(env!("CARGO_BIN_EXE_jta"))
            .arg("--workspace")
            .arg(temp.path())
            .env("PATH", fake_path(temp.path()))
            .args(["report", "--format", format])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stderr).contains("Review"));
        let markdown = generated_markdown(output);
        assert!(markdown.starts_with("# Coding agent report"));
        assert!(markdown.contains("## Corpus statistics"));
    }
    std::fs::write(temp.path().join("README.md"), "Run cargo test.").unwrap();
    let mut session = w.session("s_test", None).unwrap();
    session.project_root = Some(temp.path().to_string_lossy().into_owned());
    w.save_session(&session).unwrap();
    let result = report_data(invoke(temp.path(), &["report", "--session", "s_test"]));
    assert_eq!(result["statistics"]["sessions"], 1);
    assert_eq!(result["cli_invocations"], 2);
    assert_eq!(result["review_execution"]["backend"], "codex");
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
        .env("JTA_HOME", root.join("jta-home"))
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
    assert!(!project.join(".jta").exists());
    assert!(!nested.join(".jta").exists());
    let root = project.canonicalize().unwrap();
    let hash = jta::config::hash(root.to_string_lossy().as_bytes());
    let storage = temp
        .path()
        .join("jta-home/projects")
        .join(format!("project-{}", &hash[..16]));
    assert!(storage.join("config.json").exists());
    let stored: Vec<_> = std::fs::read_dir(storage.join("sessions"))
        .unwrap()
        .map(|e| {
            serde_json::from_slice::<Session>(&std::fs::read(e.unwrap().path()).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(stored.len(), 2);
    assert!(stored
        .iter()
        .all(|s| s.project_root.as_deref() == root.to_str()));
    let w = Workspace::init(&temp.path().join("explicit"), &Config::default()).unwrap();
    for session in stored {
        w.save_session(&session).unwrap();
    }
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
    let from_all = data(
        Command::new(env!("CARGO_BIN_EXE_jta"))
            .current_dir(&nested)
            .env("JTA_HOME", temp.path().join("jta-home"))
            .args([
                "analyze",
                "--all",
                "--agent",
                "claude",
                "--dry-run",
                "--format",
                "json",
            ])
            .output()
            .unwrap(),
    );
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
        .env("JTA_HOME", temp.path().join("jta-home"))
        .current_dir(&nested)
        .args(["init", "--format", "json"])
        .output()
        .unwrap();
    let initialized = data(init);
    assert!(
        std::path::Path::new(initialized["data_dir"].as_str().unwrap())
            .join("config.json")
            .exists()
    );
    assert!(!project.join(".jta").exists());
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
        .env("PATH", fake_path(temp.path()))
        .arg("report")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = generated_markdown(output);
    assert!(text.contains("| codex | 1 | 1 | 1/1 (100.0%) | 1/1 (100.0%) | unknown (0/1 turns) |"));
    assert!(text.contains("| Input tokens | unknown | 0/1 turns; 0/1 sessions |"));
    let review = data(invoke(
        temp.path(),
        &["report", "--agent", "claude_code", "--dry-run"],
    ));
    assert!(review["contract"].is_object());
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

#[test]
fn init_updates_only_supplied_settings_and_switches_cli_defaults() {
    let temp = tempfile::tempdir().unwrap();
    data(invoke(
        temp.path(),
        &[
            "init",
            "--retention-days",
            "42",
            "--jev-model",
            "custom-model",
            "--review-backend",
            "claude",
        ],
    ));
    data(invoke(
        temp.path(),
        &["init", "--review-backend", "codex", "--no-input"],
    ));
    let w = Workspace::discover(Some(temp.path())).unwrap();
    let c = w.config().unwrap();
    assert_eq!(c.retention_days, 42);
    assert_eq!(c.jev.model, "custom-model");
    assert_eq!(c.review.backend, "codex");
    assert!(c.review.model.is_none());
    data(invoke(temp.path(), &["init", "--no-input"]));
    assert_eq!(w.config().unwrap().review.backend, "codex");
}

#[test]
fn review_explains_default_limit_and_all_expands_selection() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut c = w.config().unwrap();
    c.min_pattern_sessions = 1;
    w.save_config(&c).unwrap();
    let mut a: Analysis = w.load_json("analyses", "a_test").unwrap();
    let base = a.turns[&1].clone();
    let mut s = w.session("s_test", None).unwrap();
    let turn = s.turns[0].clone();
    for (i, category) in [
        "redundant_work",
        "repeated_failed_approach",
        "missing_capability",
        "environment_or_tool_limitation",
        "planning_or_sequencing_problem",
    ]
    .iter()
    .enumerate()
    {
        let id = i as u32 + 2;
        let mut t = turn.clone();
        t.id = id;
        s.turns.push(t);
        let mut judgment = base.clone();
        judgment.answers.get_mut("opportunity").unwrap().selected = category.to_string();
        a.turns.insert(id, judgment);
    }
    w.save_session(&s).unwrap();
    w.save_analysis(&a).unwrap();
    let default = data(invoke(temp.path(), &["report", "--dry-run"]));
    assert_eq!(default["selection"]["eligible_patterns"], 6);
    assert_eq!(default["selection"]["selected_patterns"], 5);
    let all = data(invoke(temp.path(), &["report", "--all", "--dry-run"]));
    assert_eq!(all["selection"]["selected_patterns"], 6);
    let top = data(invoke(temp.path(), &["report", "--top", "2", "--dry-run"]));
    assert_eq!(top["selection"]["selected_patterns"], 2);
    assert_eq!(
        invoke(temp.path(), &["report", "--top", "0"]).status.code(),
        Some(2)
    );
    assert_eq!(
        invoke(temp.path(), &["report", "--all", "--top", "2"])
            .status
            .code(),
        Some(2)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(temp.path())
        .env("PATH", fake_path(temp.path()))
        .args(["report", "--dry-run"])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("5 of 6 eligible patterns"));
    assert!(text.contains("--all"));
    assert!(!text.contains("\x1b["));
}

#[test]
fn purge_removes_markdown_reports_with_expired_sources() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    let reviewed = data(invoke(temp.path(), &["report", "--session", "s_test"]));
    let report = std::path::Path::new(reviewed["report_path"].as_str().unwrap());
    assert!(std::fs::read_to_string(report)
        .unwrap()
        .contains("## Summary"));
    let preview = data(invoke(
        temp.path(),
        &["purge", "--older-than", "30d", "--dry-run"],
    ));
    assert!(preview["affected_objects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == report.to_str()));
    assert!(report.exists());
    data(invoke(temp.path(), &["purge", "--older-than", "30d"]));
    assert!(!report.exists());
}

#[test]
fn storage_uses_canonical_project_identity_and_ignores_local_jta() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("data");
    let init = |cwd: &std::path::Path| {
        data(
            Command::new(env!("CARGO_BIN_EXE_jta"))
                .current_dir(cwd)
                .env("JTA_HOME", &home)
                .args(["init", "--no-input", "--format", "json"])
                .output()
                .unwrap(),
        )
    };
    let first = temp.path().join("a/app");
    let second = temp.path().join("b/app");
    for p in [&first, &second] {
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir(p.join(".git")).unwrap();
    }
    let local = Workspace::init(&first, &Config::default()).unwrap();
    let mut c = local.config().unwrap();
    c.retention_days = 17;
    local.save_config(&c).unwrap();
    let a = init(&first);
    let nested = init(&first.join("src"));
    let b = init(&second);
    assert_eq!(a["data_dir"], nested["data_dir"]);
    assert_ne!(a["data_dir"], b["data_dir"]);
    assert_eq!(a["configuration"]["retention_days"], 90);
    assert!(!second.join(".jta").exists());
    #[cfg(unix)]
    {
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        assert_eq!(init(&alias)["data_dir"], a["data_dir"]);
    }
}

#[test]
fn readable_numbers_and_durations_preserve_exact_json() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut session = w.session("s_test", None).unwrap();
    session.turns[0].usage = Some(jta::model::Usage {
        input_tokens: Some(3_908_439_023),
        ..Default::default()
    });
    w.save_session(&session).unwrap();
    let exact = report_data(invoke(temp.path(), &["report"]))["statistics"].clone();
    assert_eq!(
        exact["resources"]["input_tokens"]["total"],
        3_908_439_023u64
    );
    let output = Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(temp.path())
        .env("PATH", fake_path(temp.path()))
        .arg("report")
        .output()
        .unwrap();
    let text = generated_markdown(output);
    assert!(text.contains("3.9B"));
    assert!(!text.contains("3908439023"));
    assert!(text.contains("1m 0s"));
    assert!(!text.contains("\x1b["));
}

#[test]
fn global_jev_credentials_work_across_projects_with_env_and_dotenv_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("user-data");
    std::fs::create_dir(&home).unwrap();
    let key_name = "JTA_TEST_SHARED_JEV_KEY";
    std::fs::write(
        home.join("credentials.json"),
        json!({key_name:"global-test-secret"}).to_string(),
    )
    .unwrap();
    for i in 0..3 {
        let project = temp.path().join(format!("project{i}"));
        let w = seed(&project);
        let mut c = w.config().unwrap();
        c.jev.api_key_env = key_name.into();
        w.save_config(&c).unwrap();
        if i > 0 {
            std::fs::write(
                project.join(".env"),
                format!("{key_name}=project-test-secret\n"),
            )
            .unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_jta"));
        command
            .arg("--workspace")
            .arg(&project)
            .args(["init", "--no-input", "--format", "json"])
            .env("JTA_HOME", &home)
            .env_remove(key_name);
        if i == 2 {
            command.env(key_name, "shell-test-secret");
        }
        let output = command.output().unwrap();
        for secret in [
            "global-test-secret",
            "project-test-secret",
            "shell-test-secret",
        ] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
        }
        let result = data(output);
        assert_eq!(result["jev_key_ready"], true);
        assert!(result.get("review_key_ready").is_none());
        assert!(result["configuration"]["review"]
            .get("api_key_env")
            .is_none());
    }
}

fn report_with_mode(root: &std::path::Path, mode: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jta"))
        .arg("--workspace")
        .arg(root)
        .args(["report", "--format", "json"])
        .args(extra)
        .env("PATH", fake_path(root))
        .env("JTA_FAKE_MODE", mode)
        .env("JEV_API_KEY", "should-not-reach-report-cli")
        .env("JTA_FAKE_LOG", root.join("cli-calls.log"))
        .output()
        .unwrap()
}

#[test]
fn cli_backends_retrieve_validate_cache_and_refresh_without_review_keys() {
    for backend in ["codex", "claude"] {
        let temp = tempfile::tempdir().unwrap();
        let w = seed(temp.path());
        std::fs::write(
            temp.path().join("README.md"),
            "Run cargo test.\npassword=secret-sentinel",
        )
        .unwrap();
        let mut c = w.config().unwrap();
        c.review.backend = backend.into();
        c.review.model = Some("fixture-model".into());
        w.save_config(&c).unwrap();
        let base = w.session("s_test", None).unwrap();
        let analysis: Analysis = w.load_json("analyses", "a_test").unwrap();
        for i in 0..6 {
            let mut s = base.clone();
            s.id = format!("s_extra{i}");
            s.events.push(jta::model::Event {
                text: "password=secret-sentinel".into(),
                ..Default::default()
            });
            w.save_session(&s).unwrap();
            let mut a = analysis.clone();
            a.id = format!("a_extra{i}");
            a.session_id = s.id;
            w.save_analysis(&a).unwrap();
        }
        let result = report_data(report_with_mode(temp.path(), "", &[]));
        assert_eq!(result["cli_invocations"], 2);
        assert_eq!(result["review_execution"]["backend"], backend);
        assert_eq!(result["statistics"]["sessions"], 7);
        assert_eq!(result["selection"]["sampled_sessions"], 4);
        assert_eq!(result["selection"]["available_sessions"], 7);
        assert_eq!(result["recommendations"][0]["supporting_sessions"], 7);
        assert_eq!(result["inspected_refs"].as_array().unwrap().len(), 7);
        assert!(!result.to_string().contains("secret-sentinel"));
        let calls = std::fs::read_to_string(temp.path().join("cli-calls.log")).unwrap();
        assert_eq!(calls.lines().count(), 2);
        for line in calls.lines() {
            let call: Value = serde_json::from_str(line).unwrap();
            assert_eq!(call["sessions"], 7);
            assert!(
                !std::path::Path::new(call["cwd"].as_str().unwrap()).exists(),
                "temporary evidence must be removed"
            );
            assert!(call["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "fixture-model"));
        }
        let cached = report_data(report_with_mode(temp.path(), "", &[]));
        assert_eq!(cached["cli_invocations"], 0);
        assert_eq!(cached["review_performed"], true);
        let md = std::fs::read_to_string(cached["report_path"].as_str().unwrap()).unwrap();
        assert!(!md.contains("Model review was skipped"));
        assert_eq!(
            std::fs::read_to_string(temp.path().join("cli-calls.log"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        let refreshed = report_data(report_with_mode(temp.path(), "", &["--refresh"]));
        assert_eq!(refreshed["cli_invocations"], 2);
        std::fs::write(temp.path().join("README.md"), "Run cargo test --locked.").unwrap();
        let changed = report_data(report_with_mode(temp.path(), "", &[]));
        assert_eq!(changed["cli_invocations"], 2);
        // Suggested changes were never applied.
        assert_eq!(
            std::fs::read_to_string(temp.path().join("README.md")).unwrap(),
            "Run cargo test --locked."
        );
    }
}

#[test]
fn cli_validation_correction_is_bounded_and_bad_results_never_become_reports() {
    for backend in ["codex", "claude"] {
        for mode in [
            "repair",
            "bad_quote",
            "unknown_ref",
            "uninspected",
            "fail",
            "malformed",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let w = seed(temp.path());
            std::fs::write(temp.path().join("README.md"), "Run cargo test.").unwrap();
            let mut c = w.config().unwrap();
            c.review.backend = backend.into();
            w.save_config(&c).unwrap();
            let output = report_with_mode(temp.path(), mode, &[]);
            if mode == "repair" {
                let result = report_data(output);
                assert_eq!(result["cli_invocations"], 4);
                assert_eq!(result["recommendations"].as_array().unwrap().len(), 1);
            } else {
                assert!(
                    !output.status.success(),
                    "{backend} {mode} unexpectedly succeeded"
                );
                assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-sentinel"));
                assert!(w.list_json::<Value>("reports").unwrap().is_empty());
                assert!(w.list_json::<Value>("recommendations").unwrap().is_empty());
                if let Ok(log) = std::fs::read_to_string(temp.path().join("cli-calls.log")) {
                    assert!(log.lines().count() <= 2);
                }
            }
        }
    }
}

#[test]
fn report_splits_projects_before_recurrence_and_enforces_investigation_budget() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let base = w.session("s_test", None).unwrap();
    let analysis: Analysis = w.load_json("analyses", "a_test").unwrap();
    for i in 0..3 {
        let root = temp.path().join(format!("project{i}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("README.md"), "Run the project check.").unwrap();
        let mut s = base.clone();
        s.id = if i == 0 {
            "s_test".into()
        } else {
            format!("s_project{i}")
        };
        s.project_root = Some(root.canonicalize().unwrap().to_string_lossy().into_owned());
        w.save_session(&s).unwrap();
        let mut a = analysis.clone();
        a.id = if i == 0 {
            "a_test".into()
        } else {
            format!("a_project{i}")
        };
        a.session_id = s.id;
        w.save_analysis(&a).unwrap();
    }
    let strict = data(invoke(
        temp.path(),
        &["report", "--recurring-only", "--dry-run"],
    ));
    assert_eq!(strict["selection"]["selected_patterns"], 0);
    let preview = data(invoke(temp.path(), &["report", "--all", "--dry-run"]));
    assert_eq!(preview["investigations"].as_array().unwrap().len(), 3);
    assert_eq!(preview["selection"]["preliminary"], true);
    for job in preview["investigations"].as_array().unwrap() {
        assert_eq!(job["available_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            job["project_context"]["projects"].as_array().unwrap().len(),
            1
        );
    }
    let mut c = w.config().unwrap();
    c.review.max_investigations = 2;
    w.save_config(&c).unwrap();
    let result = report_data(report_with_mode(temp.path(), "", &["--all"]));
    assert_eq!(result["cli_invocations"], 3);
    assert_eq!(result["selection"]["omitted_patterns"], 1);
    assert_eq!(
        result["review_execution"]["stages"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn insufficient_distinct_session_support_is_rejected_after_correction() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    std::fs::write(temp.path().join("README.md"), "Run cargo test.").unwrap();
    let base = w.session("s_test", None).unwrap();
    let analysis: Analysis = w.load_json("analyses", "a_test").unwrap();
    for i in 0..2 {
        let mut s = base.clone();
        s.id = format!("s_{i}");
        w.save_session(&s).unwrap();
        let mut a = analysis.clone();
        a.id = format!("a_{i}");
        a.session_id = s.id;
        w.save_analysis(&a).unwrap();
    }
    let output = report_with_mode(temp.path(), "insufficient", &[]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("insufficient distinct-session support")
    );
    assert!(w.list_json::<Value>("reports").unwrap().is_empty());
}

#[test]
fn legacy_review_settings_migrate_without_resolving_retired_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    let mut config = serde_json::to_value(w.config().unwrap()).unwrap();
    config["review"] = json!({"provider":"openrouter","model":"old/api-model", "endpoint":"https://example.invalid", "api_key_env":"OLD_KEY"});
    std::fs::write(w.data_dir().join("config.json"), config.to_string()).unwrap();
    let setup = data(invoke(temp.path(), &["init", "--no-input"]));
    assert_eq!(setup["configuration"]["review"]["backend"], "codex");
    assert!(setup["configuration"]["review"]["model"].is_null());
    for removed in ["provider", "endpoint", "api_key_env"] {
        assert!(setup["configuration"]["review"].get(removed).is_none());
    }
    for removed in [
        "--review-provider",
        "--review-endpoint",
        "--review-api-key-env",
    ] {
        assert_eq!(
            invoke(temp.path(), &["init", removed, "value"])
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn dry_run_requires_no_cli_and_reports_missing_runtime_and_invalid_budgets() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    for (arg, value) in [
        ("--review-timeout-secs", "0"),
        ("--review-max-investigations", "0"),
        ("--review-backend", "openai"),
    ] {
        assert_eq!(
            invoke(temp.path(), &["init", "--no-input", arg, value])
                .status
                .code(),
            Some(2)
        );
    }
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_jta"))
            .arg("--workspace")
            .arg(temp.path())
            .args(args)
            .env("PATH", "")
            .output()
            .unwrap()
    };
    assert!(run(&["report", "--dry-run", "--format", "json"])
        .status
        .success());
    let failure = run(&["report"]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("Cannot start report CLI"));
    let preview = data(invoke(temp.path(), &["report", "--dry-run"]));
    let id = preview["investigations"][0]["id"].as_str().unwrap();
    let shown = data(invoke(temp.path(), &["show", id]));
    assert_eq!(
        shown["project_root"],
        temp.path().canonicalize().unwrap().to_str().unwrap()
    );
}

#[test]
fn cached_results_are_revalidated_and_purged_with_source_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let w = seed(temp.path());
    report_data(report_with_mode(temp.path(), "", &[]));
    let cached = w.list_json::<Value>("review_cache").unwrap();
    assert_eq!(cached.len(), 2);
    let mut paths = std::fs::read_dir(w.data_dir().join("review_cache"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    // Corrupt both cached inspection records; neither can bypass current validation.
    for path in &paths {
        let mut result: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        result["response"]["inspected_refs"] = json!([{"session_id":"unknown","turn_id":99}]);
        std::fs::write(path, result.to_string()).unwrap();
    }
    assert!(!report_with_mode(temp.path(), "", &[]).status.success());
    assert_eq!(w.list_json::<Value>("reports").unwrap().len(), 1);
    data(invoke(temp.path(), &["purge", "--older-than", "30d"]));
    assert!(w.list_json::<Value>("review_cache").unwrap().is_empty());
}
