//! Inspect wire payload sizes without credentials, personal logs, or provider calls.
//! Character counts are not tokenizer counts or billed usage.
use jta::{
    config::Config,
    model::{Event, Session, Turn},
    services,
};
use serde_json::json;

fn main() {
    let config = Config::default();
    let review = services::prepare_review(&json!({"isolated":false}), &config).unwrap();
    println!(
        "Recommendation system prompt: {} characters (includes Humanizer)",
        review["messages"][0]["content"]
            .as_str()
            .unwrap()
            .chars()
            .count()
    );
    println!(
        "Recommendation response format/schema: {} serialized characters",
        review["response_format"].to_string().chars().count()
    );
    println!(
        "Recommendation completion limit: {} tokens for the whole response",
        review["max_completion_tokens"]
    );
    for (name, text, expected_requests) in [
        ("minimal", String::new(), 32),
        (
            "with event text",
            "Inspect the exported artifact and check the recorded result. ".repeat(35),
            101,
        ),
    ] {
        let session = Session {
            id: "s_synthetic".into(),
            revision: "v1".into(),
            events: (1..=100)
                .map(|line| Event {
                    line,
                    kind: "assistant".into(),
                    text: text.clone(),
                    ..Default::default()
                })
                .collect(),
            turns: (1..=100)
                .map(|id| Turn {
                    id,
                    event_indices: vec![id as usize - 1],
                    intent: "Check the artifact".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let payloads = services::prepare_analysis(&session, &config).unwrap();
        assert_eq!(payloads.len(), expected_requests);
        let questions: usize = payloads
            .iter()
            .map(|p| p["questions"].as_object().unwrap().len())
            .sum();
        assert_eq!(questions, 2003);
        let state_chars: usize = payloads
            .iter()
            .map(|p| p["state"].to_string().chars().count())
            .sum();
        let question_chars: usize = payloads
            .iter()
            .map(|p| p["questions"].to_string().chars().count())
            .sum();
        println!("\n100-turn session ({name}):");
        println!(
            "  {} questions; {} requests per session; {} requests for 3,000 uncached sessions",
            questions,
            payloads.len(),
            payloads.len() * 3000
        );
        println!(
            "  {} context characters + {} question characters across that session's requests",
            state_chars, question_chars
        );
        println!(
            "  {} total context + question characters if all 3,000 sessions had this shape",
            (state_chars + question_chars) as u64 * 3000
        );
    }
    println!("\nCounts exclude retries, provider framing, and outputs. They are synthetic payload measurements, not token or price estimates.");
}
