//! Bounded evidence packets and verified TypeSafe/OpenAI-compatible HTTP contracts.
mod packets;
mod review;
mod rubric;
mod transport;
use crate::{
    config::{analysis_fingerprint, hash, Config},
    model::*,
};
use anyhow::{bail, ensure, Context, Result};
pub use packets::evidence_packet;
pub use review::{prepare_review, review, validate_review};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Build exact wire payloads without touching credentials or the network.
pub fn prepare_analysis(session: &Session, config: &Config) -> Result<Vec<Value>> {
    ensure!(
        config.jev.max_questions > 0,
        "max_questions must be positive"
    );
    ensure!(
        config.jev.max_context_chars >= 4096,
        "max_context_chars must be at least 4096"
    );
    let turn_ids: BTreeSet<_> = session.turns.iter().map(|t| t.id).collect();
    ensure!(
        turn_ids.len() == session.turns.len() && !turn_ids.contains(&0),
        "Session turn IDs must be unique and positive"
    );
    ensure!(
        session
            .turns
            .iter()
            .all(|t| t.event_indices.iter().all(|i| *i < session.events.len())),
        "Turn references an absent event"
    );
    let full = json!({"session_id":session.id,"revision":session.revision,"warnings":session.warnings,"event_indexing":"Turn event_indices are zero-based indices into events; turn id is the stable one-based assistant-turn identity. Source line numbers are one-based.","events":session.events,"turns":session.turns});
    let short = full.to_string().chars().count() <= config.jev.max_context_chars;
    let mut groups = vec![(
        if short {
            full.clone()
        } else {
            evidence_packet(session, None, config.jev.max_context_chars)?
        },
        rubric::session_questions(),
    )];
    for turn in &session.turns {
        let state = if short {
            full.clone()
        } else {
            evidence_packet(session, Some(turn.id), config.jev.max_context_chars)?
        };
        let questions = rubric::turn_questions(turn);
        if short {
            groups[0].1.extend(questions);
        } else {
            groups.push((state, questions));
        }
    }
    let mut payloads = Vec::new();
    for (state, questions) in groups {
        let entries: Vec<_> = questions.into_iter().collect();
        for chunk in entries.chunks(config.jev.max_questions) {
            let questions: Map<String, Value> = chunk.iter().cloned().collect();
            payloads.push(json!({"model":config.jev.model,"state":state,"questions":questions}));
        }
    }
    Ok(payloads)
}

/// Reject missing, foreign, malformed, or incomplete distributions; never invent probabilities.
pub fn validate_answers(
    request: &Value,
    response: &Value,
) -> Result<BTreeMap<String, Distribution>> {
    ensure!(
        response["model"]
            .as_str()
            .is_some_and(|m| !m.trim().is_empty()),
        "Jev response missing resolved model identity"
    );
    let expected = request["questions"]
        .as_object()
        .context("invalid local questions")?;
    let answers = response["answers"]
        .as_object()
        .context("Jev response missing answers")?;
    ensure!(
        answers.len() == expected.len(),
        "Jev response question count mismatch"
    );
    let mut out = BTreeMap::new();
    for (key, q) in expected {
        let a = answers
            .get(key)
            .context("Jev response missing requested question")?;
        ensure!(
            a["type"] == "choice",
            "Jev response has incorrect answer type"
        );
        let selected = a["choice"].as_str().context("Jev answer missing choice")?;
        let probabilities: BTreeMap<String, f64> =
            serde_json::from_value(a["probabilities"].clone())
                .map_err(|_| anyhow::anyhow!("Jev answer has malformed probabilities"))?;
        let criteria = q["criteria"]
            .as_object()
            .context("invalid local criteria")?;
        ensure!(
            probabilities.len() == criteria.len()
                && probabilities.keys().all(|k| criteria.contains_key(k)),
            "Jev probability alternatives mismatch"
        );
        ensure!(
            probabilities.contains_key(selected),
            "Jev selected unknown alternative"
        );
        ensure!(
            probabilities
                .values()
                .all(|p| p.is_finite() && (0.0..=1.0).contains(p)),
            "Jev probabilities out of range"
        );
        let sum = probabilities.values().sum::<f64>();
        // Jev can round each probability to hundredths independently. Only
        // allow that rounding budget when every value is on the hundredth grid;
        // retain the stricter tolerance for higher-precision distributions.
        let rounded = probabilities
            .values()
            .all(|p| (p * 100.0 - (p * 100.0).round()).abs() <= 1e-9);
        let tolerance = if rounded {
            0.005 * probabilities.len() as f64
        } else {
            0.001
        };
        ensure!(
            sum > 0.0 && (sum - 1.0).abs() <= tolerance + 1e-9,
            "Jev probabilities do not sum to one for {key}: sum={sum:.8}, tolerance={tolerance:.8}"
        );
        let confidence = a["confidence"]
            .as_f64()
            .context("Jev answer missing confidence")?;
        ensure!(
            (0.0..=1.0).contains(&confidence),
            "Jev confidence out of range"
        );
        out.insert(
            key.clone(),
            Distribution {
                selected: selected.into(),
                probabilities,
                provider_confidence: Some(confidence),
            },
        );
    }
    Ok(out)
}

pub async fn score_session(session: &Session, config: &Config) -> Result<Analysis> {
    let requests = prepare_analysis(session, config)?;
    let key = transport::credential(&config.jev.api_key_env)?;
    let client = transport::client(config.jev.timeout_secs)?;
    let mut answers = BTreeMap::new();
    let mut usage = Vec::new();
    let mut warnings = session.warnings.clone();
    let total = requests.len();
    for (index, request) in requests.into_iter().enumerate() {
        crate::ui::scoring_batch(index, total, &session.id);
        let response = transport::post(&client, &config.jev.endpoint, &key, &request).await?;
        let validated = validate_answers(&request, &response)?;
        crate::ui::scored_batch(index + 1);
        for (question, distribution) in &validated {
            if let Some((alternative, probability)) = distribution.higher_probability_alternative()
            {
                warnings.push(format!(
                    "Jev selected choice for {question}: {} (probability={:.8}) is not a maximum probability alternative; {alternative} has probability={probability:.8}; original choice and probabilities preserved",
                    distribution.selected, distribution.confidence()
                ));
            }
            let sum = distribution.probabilities.values().sum::<f64>();
            if (sum - 1.0).abs() > 0.001 + 1e-9 {
                warnings.push(format!(
                    "Jev rounded probabilities for {question} sum to {sum:.8}; original values preserved"
                ));
            }
        }
        answers.extend(validated);
        usage.push(json!({"model":response.get("model"),"usage":response.get("usage")}));
        if request["state"]["bounded"] == true
            && !warnings
                .iter()
                .any(|w| w == "Bounded evidence packets used; omitted context may affect judgments")
        {
            warnings
                .push("Bounded evidence packets used; omitted context may affect judgments".into());
        }
    }
    let mut session_answers = BTreeMap::new();
    let mut turns = BTreeMap::new();
    for (question, distribution) in answers {
        if let Some(name) = question.strip_prefix("session.") {
            session_answers.insert(name.into(), distribution);
        } else if let Some(rest) = question.strip_prefix("turn.") {
            let (id, name) = rest.split_once('.').context("invalid local question ID")?;
            let turn = turns
                .entry(id.parse::<u32>()?)
                .or_insert_with(TurnJudgment::default);
            turn.answers.insert(name.into(), distribution);
        } else {
            bail!("invalid local question ID");
        }
    }
    for turn in turns.values_mut() {
        turn.downstream_turn = turn
            .answers
            .get("downstream_turn")
            .and_then(|d| d.selected.strip_prefix("t_"))
            .and_then(|id| id.parse().ok());
        let primary = turn.answers.get("opportunity").map(|d| d.selected.as_str());
        turn.secondary_opportunities = turn
            .answers
            .iter()
            .filter_map(|(key, d)| {
                key.strip_prefix("secondary.")
                    .filter(|label| d.selected == "yes" && Some(*label) != primary)
                    .map(str::to_owned)
            })
            .collect();
        turn.inconsistencies = rubric::inconsistencies(&turn.answers);
    }
    let resolved_models: BTreeSet<_> = usage
        .iter()
        .filter_map(|r| r["model"].as_str().map(str::to_owned))
        .collect();
    if resolved_models.len() > 1 {
        warnings.push("Jev resolved to multiple model versions within this analysis".into());
    }
    let fingerprint = analysis_fingerprint(config);
    let created_at = chrono::Utc::now().to_rfc3339();
    Ok(Analysis {
        id: format!(
            "a_{}",
            &hash(
                format!(
                    "{}:{}:{}:{}",
                    session.id, session.revision, fingerprint, created_at
                )
                .as_bytes()
            )[..16]
        ),
        session_id: session.id.clone(),
        revision: session.revision.clone(),
        created_at,
        config_fingerprint: fingerprint,
        rubric_version: RUBRIC_VERSION.into(),
        session: session_answers,
        turns,
        warnings,
        usage: Some(
            json!({"requests":usage,"requested_model":config.jev.model,"resolved_models":resolved_models}),
        ),
    })
}

#[cfg(test)]
mod tests;
