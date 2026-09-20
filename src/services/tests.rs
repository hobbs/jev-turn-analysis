use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
fn fixture() -> Session {
    Session {
        id: "s_test".into(),
        revision: "rev".into(),
        events: vec![
            Event {
                kind: "user".into(),
                text: "Fix behavior and verify with tests".into(),
                line: 1,
                ..Default::default()
            },
            Event {
                kind: "assistant".into(),
                text: "Investigating".into(),
                line: 2,
                ..Default::default()
            },
            Event {
                kind: "tool_result".into(),
                text: "Tests passed".into(),
                line: 3,
                ..Default::default()
            },
        ],
        turns: vec![
            Turn {
                id: 1,
                event_indices: vec![1],
                candidate_downstream: vec![2],
                ..Default::default()
            },
            Turn {
                id: 2,
                event_indices: vec![2],
                verification: vec!["tests passed".into()],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
fn answer(request: &Value) -> Value {
    let answers:Map<String,Value>=request["questions"].as_object().unwrap().iter().map(|(id,q)|{
        let criteria=q["criteria"].as_object().unwrap();let chosen=criteria.keys().next().unwrap();
        let probabilities:Map<String,Value>=criteria.keys().map(|label|(label.clone(),json!(if label==chosen {1.0}else{0.0}))).collect();
        (id.clone(),json!({"type":"choice","choice":chosen,"confidence":1.0,"probabilities":probabilities}))
    }).collect();
    json!({"model":"jev-test","answers":answers,"usage":{"input_tokens":10,"output_tokens":20}})
}
#[test]
fn all_questions_preserve_complete_distributions() {
    let mut config = Config::default();
    config.jev.max_questions = 7;
    let requests = prepare_analysis(&fixture(), &config).unwrap();
    let count: usize = requests
        .iter()
        .map(|r| {
            assert!(r["questions"].as_object().unwrap().len() <= 7);
            validate_answers(r, &answer(r)).unwrap().len()
        })
        .sum();
    assert_eq!(count, 3 + 2 * 20);
    assert!(requests.iter().any(|r| r["questions"]
        .get("turn.1.secondary.missed_verification")
        .is_some()));
}
#[test]
fn rejects_missing_and_invalid_probabilities() {
    let request = prepare_analysis(&fixture(), &Config::default())
        .unwrap()
        .remove(0);
    let response = answer(&request);
    let key = response["answers"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    for replacement in [json!({}), json!({"unexpected":1.0})] {
        let mut bad = response.clone();
        bad["answers"][&key]["probabilities"] = replacement;
        assert!(validate_answers(&request, &bad).is_err());
    }
    let mut bad = response.clone();
    bad["answers"][&key]["confidence"] = json!(2);
    assert!(validate_answers(&request, &bad).is_err());
    let mut bad = response;
    bad["answers"].as_object_mut().unwrap().remove(&key);
    assert!(validate_answers(&request, &bad).is_err());
}
#[test]
fn non_maximum_choices_preserve_provider_values() {
    let request = json!({"questions":{"q":{"criteria":{"a":null,"b":null}}}});
    for (a, b, mismatch) in [
        (0.2, 0.8, true),
        (0.0, 1.0, true),
        (0.5, 0.5, false),
        (0.4999999999, 0.5000000001, false),
        (0.8, 0.2, false),
    ] {
        let response = json!({"model":"jev-test","answers":{"q":{
            "type":"choice","choice":"a","confidence":0.75,
            "probabilities":{"a":a,"b":b}
        }}});
        let validated = validate_answers(&request, &response).unwrap();
        let distribution = &validated["q"];
        assert_eq!(distribution.selected, "a");
        assert_eq!(distribution.confidence(), a);
        assert_eq!(distribution.provider_confidence, Some(0.75));
        assert_eq!(
            serde_json::to_value(&distribution.probabilities).unwrap(),
            response["answers"]["q"]["probabilities"]
        );
        assert_eq!(
            distribution.higher_probability_alternative().is_some(),
            mismatch
        );
        assert_eq!(
            rubric::inconsistencies(&validated)
                .iter()
                .any(|s| s.contains("not a maximum")),
            mismatch
        );

        let mut unknown = response.clone();
        unknown["answers"]["q"]["choice"] = json!("unknown");
        assert!(validate_answers(&request, &unknown)
            .unwrap_err()
            .to_string()
            .contains("unknown alternative"));
    }
}
#[tokio::test]
async fn scoring_preserves_non_maximum_choices_and_records_warnings() {
    let mut config = Config::default();
    config.jev.api_key_env = "JTA_CHOICE_MISMATCH_TEST_KEY".into();
    std::env::set_var(&config.jev.api_key_env, "test-key");
    let session = fixture();
    let request = prepare_analysis(&session, &config).unwrap().remove(0);
    let mut response = answer(&request);
    let keys = ["session.outcome_verification", "turn.1.usefulness"];
    for key in keys {
        let answer = &mut response["answers"][key];
        // Keep the complete distribution, but select an option with probability zero.
        answer["choice"] = json!(
            answer["probabilities"]
                .as_object()
                .unwrap()
                .iter()
                .find(|(_, p)| p.as_f64() == Some(0.0))
                .unwrap()
                .0
        );
    }
    let expected = validate_answers(&request, &response).unwrap();
    let (url, handle) = server(vec![(200, response)]);
    config.jev.endpoint = url;
    let analysis = score_session(&session, &config).await.unwrap();
    assert_eq!(analysis.warnings.len(), 2);
    for (key, distribution) in [
        (keys[0], &analysis.session["outcome_verification"]),
        (keys[1], &analysis.turns[&1].answers["usefulness"]),
    ] {
        assert_eq!(
            serde_json::to_value(distribution).unwrap(),
            serde_json::to_value(&expected[key]).unwrap()
        );
        let warning = analysis.warnings.iter().find(|w| w.contains(key)).unwrap();
        assert!(warning.contains(&distribution.selected));
        assert!(warning.contains("probability=0.00000000"));
        assert!(warning.contains("probability=1.00000000"));
        assert!(warning.contains("original choice and probabilities preserved"));
    }
    assert!(analysis.turns[&1]
        .inconsistencies
        .iter()
        .any(|s| s == "Selected choice is not a maximum probability alternative: usefulness"));
    assert_eq!(handle.join().unwrap().len(), 1);
    std::env::remove_var(&config.jev.api_key_env);
}
#[test]
fn rounded_probability_sums_preserve_provider_values() {
    let request = json!({"questions":{"session.outcome_verification":{"criteria":{
        "claimed_but_unverified":null,"known_incomplete":null,"unclear":null,"verified":null
    }}}});
    // Regression: a live response that previously rejected the entire session.
    let probabilities = json!({"claimed_but_unverified":0.93,"known_incomplete":0.02,"unclear":0.04,"verified":0.0});
    let response = json!({"model":"jev-test","answers":{"session.outcome_verification":{
        "type":"choice","choice":"claimed_but_unverified","confidence":0.8,"probabilities":probabilities
    }}});
    let validated = validate_answers(&request, &response).unwrap();
    let distribution = &validated["session.outcome_verification"];
    assert_eq!(
        serde_json::to_value(&distribution.probabilities).unwrap(),
        probabilities
    );
    assert_eq!(distribution.provider_confidence, Some(0.8));

    let request = json!({"questions":{"q":{"criteria":{"a":null,"b":null,"c":null}}}});
    for (values, valid) in [
        ([0.34, 0.33, 0.32], true),     // 0.99
        ([0.34, 0.34, 0.33], true),     // 1.01
        ([0.333, 0.333, 0.333], true),  // Existing high-precision tolerance.
        ([0.334, 0.334, 0.334], false), // Higher precision cannot use rounding budget.
        ([0.34, 0.32, 0.32], false),    // 0.98 exceeds three-option rounding budget.
        ([0.34, 0.34, 0.34], false),    // 1.02
        ([0.0, 0.0, 0.0], false),
        ([1.01, 0.0, 0.0], false),
        ([1.0, 0.01, -0.01], false),
    ] {
        let response = json!({"model":"jev-test","answers":{"q":{
            "type":"choice","choice":"a","confidence":0.5,
            "probabilities":{"a":values[0],"b":values[1],"c":values[2]}
        }}});
        let result = validate_answers(&request, &response);
        assert_eq!(result.is_ok(), valid, "{values:?}: {result:?}");
        if values == [0.34, 0.34, 0.34] {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("for q: sum=1.02000000, tolerance=0.01500000"));
        }
    }
}
#[tokio::test]
async fn scoring_records_rounding_warning() {
    let mut config = Config::default();
    config.jev.api_key_env = "JTA_ROUNDING_TEST_KEY".into();
    std::env::set_var(&config.jev.api_key_env, "test-key");
    let session = fixture();
    let request = prepare_analysis(&session, &config).unwrap().remove(0);
    let mut response = answer(&request);
    let answer = &mut response["answers"]["session.outcome_verification"];
    answer["choice"] = json!("claimed_but_unverified");
    answer["probabilities"] = json!({"claimed_but_unverified":0.93,"known_incomplete":0.02,"unclear":0.04,"verified":0.0});
    let (url, handle) = server(vec![(200, response)]);
    config.jev.endpoint = url;
    let analysis = score_session(&session, &config).await.unwrap();
    assert!(analysis
        .warnings
        .iter()
        .any(|warning| warning.contains("session.outcome_verification sum to 0.99000000")));
    assert_eq!(
        analysis.session["outcome_verification"].probabilities["claimed_but_unverified"],
        0.93
    );
    assert_eq!(handle.join().unwrap().len(), 1);
    std::env::remove_var(&config.jev.api_key_env);
}
#[test]
fn bounded_packet_preserves_outcome_and_references() {
    let mut session = fixture();
    session.events[1].text = "x".repeat(100_000);
    let packet = evidence_packet(&session, Some(1), 4096).unwrap();
    assert!(packet.to_string().chars().count() <= 4096);
    assert_eq!(packet["bounded"], true);
    assert!(packet["sections"]["final_state"]
        .to_string()
        .contains("Tests passed"));
    assert!(packet["sections"]["candidate_downstream"]
        .to_string()
        .contains("turn_ids"));
    assert!(packet.to_string().contains("truncated"));
}
#[test]
fn review_schema_and_reference_validation() {
    let evidence = json!({"sessions":[{"session_id":"s_test","turns":[{"turn_id":2}]}],
        "project_context":{"projects":[{"project_root":"/project","session_ids":["s_test"],
            "files":[{"path":"/project/AGENTS.md","text":"Run npm test before delivery."}],
            "creation_targets":["/project/CLAUDE.md"]}]}});
    let mut result = json!({"recommendations":[{"title":"Verify","observed_pattern":"Missing check","outcome_effect":"Uncertain success","supporting_refs":[{"session_id":"s_test","turn_id":2}],"uncertainty":"One case","counterexamples":[],"remediation_surface":"AGENTS.md","proposed_change":"Require relevant checks","scope":"Coding tasks","risk":"Slow checks","evaluation_plan":"Compare verified task completion"}]});
    result["recommendations"][0]["project_root"] = json!("/project");
    result["recommendations"][0]["targets"] = json!([{"path":"/project/AGENTS.md","action":"edit",
        "before":"Run npm test before delivery.","after":"Run npm test before delivery; record its result.",
        "rationale":"The project already requires npm test; make its result visible.",
        "context_refs":[{"path":"/project/AGENTS.md","quote":"Run npm test"}]}]);
    validate_review(&result, &evidence).unwrap();
    for (pointer, value) in [
        ("/recommendations/0/targets", json!([])),
        ("/recommendations/0/project_root", json!("/invented")),
        (
            "/recommendations/0/targets/0/path",
            json!("/project/missing/SKILL.md"),
        ),
        (
            "/recommendations/0/targets/0/before",
            json!("An invented instruction"),
        ),
        (
            "/recommendations/0/targets/0/after",
            json!("Run npm test before delivery."),
        ),
        ("/recommendations/0/targets/0/context_refs", json!([])),
        (
            "/recommendations/0/targets/0/context_refs/0/quote",
            json!("invented project command"),
        ),
        (
            "/recommendations/0/targets/0/context_refs/0/path",
            json!("/another-project/AGENTS.md"),
        ),
    ] {
        let mut invalid = result.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            validate_review(&invalid, &evidence).is_err(),
            "accepted {pointer}"
        );
    }
    let mut generic = result.clone();
    generic["recommendations"][0]
        .as_object_mut()
        .unwrap()
        .remove("targets");
    assert!(validate_review(&generic, &evidence).is_err());
    let mut creation = result.clone();
    creation["recommendations"][0]["targets"][0]["path"] = json!("/project/CLAUDE.md");
    creation["recommendations"][0]["targets"][0]["action"] = json!("create");
    creation["recommendations"][0]["targets"][0]["before"] = json!("");
    validate_review(&creation, &evidence).unwrap();
    creation["recommendations"][0]["targets"][0]["path"] = json!("/project/AGENTS.md");
    assert!(validate_review(&creation, &evidence).is_err());
    let mut wrong_project = evidence.clone();
    wrong_project["project_context"]["projects"][0]["session_ids"] = json!(["s_other"]);
    assert!(validate_review(&result, &wrong_project).is_err());
    assert!(validate_review(&json!({"recommendations":[]}), &json!({})).is_ok());
    result["recommendations"][0]["supporting_refs"][0]["turn_id"] = json!(99);
    assert!(validate_review(&result, &evidence).is_err());
    let mut config = Config::default();
    config.review.provider = "openrouter".into();
    let payload = prepare_review(&evidence, &config).unwrap();
    assert_eq!(payload["provider"]["require_parameters"], true);
    assert_eq!(payload["response_format"]["json_schema"]["strict"], true);
}
fn server(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut requests = vec![];
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = vec![];
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            assert!(headers
                .to_ascii_lowercase()
                .contains("authorization: bearer test-key"));
            let size: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + size {
                let n = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..n]);
            }
            requests.push(serde_json::from_slice(&bytes[header_end..header_end + size]).unwrap());
            let body = body.to_string();
            write!(stream,"HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    (format!("http://{address}/v1/systemone"), handle)
}
#[tokio::test]
async fn review_repairs_once_and_revalidates_project_grounding() {
    let evidence = json!({"isolated":true,"sessions":[{"session_id":"s_test","turns":[{"turn_id":2}]}],
        "project_context":{"projects":[{"project_root":"/project","session_ids":["s_test"],
        "files":[{"path":"/project/AGENTS.md","text":"Run npm test."}],"creation_targets":[]}]}});
    let valid = json!({"recommendations":[{
        "project_root":"/project","title":"Record the npm test result","observed_pattern":"Test outcome missing",
        "outcome_effect":"Unverified result","supporting_refs":[{"session_id":"s_test","turn_id":2}],
        "uncertainty":"Limited evidence","counterexamples":[],"remediation_surface":"AGENTS.md",
        "proposed_change":"Record npm test result","scope":"This project","risk":"Extra reporting",
        "evaluation_plan":"Check npm test outcome in the replay",
        "targets":[{"path":"/project/AGENTS.md","action":"edit","before":"Run npm test.",
            "after":"Run npm test and record its result.","rationale":"Make this project's existing check visible.",
            "context_refs":[{"path":"/project/AGENTS.md","quote":"Run npm test."}]}]
    }]});
    let mut invalid = valid.clone();
    invalid["recommendations"][0]["targets"][0]["context_refs"][0]["quote"] =
        json!("Made up quote");
    std::env::set_var("JTA_TEST_GROUNDED_REVIEW_KEY", "test-key");
    for repaired in [true, false] {
        let response = |value: &Value| json!({"choices":[{"finish_reason":"stop","message":{"content":value.to_string()}}]});
        let (url, handle) = server(vec![
            (200, response(&invalid)),
            (200, response(if repaired { &valid } else { &invalid })),
        ]);
        let mut config = Config::default();
        config.review.provider = "openai".into();
        config.review.endpoint = url;
        config.review.api_key_env = "JTA_TEST_GROUNDED_REVIEW_KEY".into();
        let result = review(&evidence, &config).await;
        if repaired {
            assert_eq!(result.unwrap(), valid);
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("after one correction"));
        }
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 4);
        assert_eq!(requests[0]["messages"][1], requests[1]["messages"][1]);
        assert!(requests[1]["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("context_refs[0]"));
    }
}
#[tokio::test]
async fn http_contract_retries_rate_limit_and_preserves_payload() {
    let request = prepare_analysis(&fixture(), &Config::default())
        .unwrap()
        .remove(0);
    let expected = answer(&request);
    let (url, handle) = server(vec![
        (429, json!({"secret":"must not leak"})),
        (200, expected.clone()),
    ]);
    let received = transport::post(&transport::client(10).unwrap(), &url, "test-key", &request)
        .await
        .unwrap();
    assert_eq!(received, expected);
    assert_eq!(handle.join().unwrap(), vec![request.clone(), request]);
}
#[tokio::test]
async fn http_errors_are_sanitized_and_not_retried_for_auth() {
    let (url, handle) = server(vec![(401, json!({"secret":"sensitive-response"}))]);
    let error = transport::post(
        &transport::client(10).unwrap(),
        &url,
        "test-key",
        &json!({}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("401"));
    assert!(!error.contains("sensitive-response"));
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn malformed_turn_identity_is_rejected_before_remote_work() {
    let mut session = fixture();
    session.turns[1].id = 1;
    assert!(prepare_analysis(&session, &Config::default()).is_err());
    session.turns[1].id = 2;
    session.turns[1].event_indices = vec![usize::MAX];
    assert!(prepare_analysis(&session, &Config::default()).is_err());
}
