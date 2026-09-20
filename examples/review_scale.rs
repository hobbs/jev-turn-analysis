//! Offline, synthetic aggregation probe; no real transcripts or provider calls.
use jta::{
    analytics,
    model::{Analysis, Distribution, Session, Turn, TurnJudgment},
};
use std::{collections::BTreeMap, time::Instant};

fn answer(selected: &str) -> Distribution {
    Distribution {
        selected: selected.into(),
        probabilities: BTreeMap::from([(selected.into(), 1.0)]),
        ..Default::default()
    }
}

fn main() {
    let start = Instant::now();
    let pairs = (0..3000)
        .map(|i| {
            let session = Session {
                id: format!("s_{i:04}"),
                revision: "rev1".into(),
                agent: "codex".into(),
                project_root: Some("/synthetic/project".into()),
                // Thirty distinct issue descriptions share the same rubric category.
                turns: (1..=100)
                    .map(|id| Turn {
                        id,
                        intent: format!("Issue cluster {} at step {id}", i % 30),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            let analysis = Analysis {
                id: format!("a_{i:04}"),
                session_id: session.id.clone(),
                revision: session.revision.clone(),
                session: BTreeMap::from([
                    ("task_outcome".into(), answer("complete")),
                    (
                        "outcome_verification".into(),
                        answer("claimed_but_unverified"),
                    ),
                ]),
                turns: (1..=100)
                    .map(|id| {
                        (
                            id,
                            TurnJudgment {
                                answers: BTreeMap::from([
                                    ("opportunity".into(), answer("missed_verification")),
                                    ("remediation_surface".into(), answer("AGENTS.md")),
                                ]),
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
                ..Default::default()
            };
            (session, analysis)
        })
        .collect::<Vec<_>>();
    println!(
        "Fixture construction: {:.2}s",
        start.elapsed().as_secs_f64()
    );
    let start = Instant::now();
    let patterns = analytics::patterns(&pairs);
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].sessions, 3000);
    assert_eq!(patterns[0].supporting.len(), 300000);
    println!(
        "Pattern aggregation: {:.2}s; 30 described issue clusters collapse into {} category",
        start.elapsed().as_secs_f64(),
        patterns.len()
    );
    drop(patterns);
    let start = Instant::now();
    let report = analytics::report(&pairs, &analytics::Filters::default()).unwrap();
    assert_eq!(report["sessions"], 3000);
    assert_eq!(report["turns"], 300000);
    println!(
        "Corpus report: {:.2}s; {} sessions; {} turns",
        start.elapsed().as_secs_f64(),
        report["sessions"],
        report["turns"]
    );
    println!("Default review of this category samples 3 sessions (0.1%); it cannot inspect all 30 clusters.");
}
