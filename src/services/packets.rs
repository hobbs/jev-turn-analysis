use crate::model::Session;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
fn excerpt(value: &Value, limit: usize) -> String {
    let text = value.to_string();
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    let head: String = text.chars().take(limit / 2).collect();
    let tail: String = text.chars().skip(count - limit / 2).collect();
    format!("{head}\n[... excerpt omitted ...]\n{tail}")
}
/// Exact bounded state used remotely. Excerpts retain source indices and explicit omission counts.
pub fn evidence_packet(session: &Session, target: Option<u32>, max_chars: usize) -> Result<Value> {
    ensure!(
        max_chars >= 4096,
        "evidence budget must be at least 4096 characters"
    );
    let target_turn = target.and_then(|id| session.turns.iter().find(|t| t.id == id));
    let mut groups: Vec<(&str, Vec<usize>)> = vec![];
    groups.push((
        "goal_and_user_corrections",
        session
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == "user")
            .map(|(i, _)| i)
            .collect(),
    ));
    groups.push((
        "target",
        target_turn
            .map(|t| t.event_indices.clone())
            .unwrap_or_default(),
    ));
    groups.push((
        "final_state",
        (session.events.len().saturating_sub(8)..session.events.len()).collect(),
    ));
    groups.push((
        "verification",
        session
            .turns
            .iter()
            .filter(|t| !t.verification.is_empty())
            .flat_map(|t| t.event_indices.clone())
            .collect(),
    ));
    groups.push((
        "candidate_downstream",
        target_turn
            .map(|t| {
                session
                    .turns
                    .iter()
                    .filter(|later| t.candidate_downstream.contains(&later.id))
                    .flat_map(|t| t.event_indices.clone())
                    .collect()
            })
            .unwrap_or_default(),
    ));
    groups.push((
        "nearby",
        target_turn
            .map(|t| {
                session
                    .turns
                    .iter()
                    .filter(|other| other.id.abs_diff(t.id) <= 2)
                    .flat_map(|t| t.event_indices.clone())
                    .collect()
            })
            .unwrap_or_default(),
    ));
    // Every category receives an independent budget so early tool output cannot evict the final outcome.
    let mut per_event = 900usize;
    let mut per_group = 8usize;
    loop {
        let mut included = BTreeSet::new();
        let mut sections = serde_json::Map::new();
        for (name, indices) in &groups {
            let valid: Vec<_> = indices
                .iter()
                .copied()
                .filter(|i| *i < session.events.len())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            // Sample across the full category, keeping first and last rather than dropping later corrections.
            let positions: BTreeSet<usize> = if valid.len() <= per_group {
                (0..valid.len()).collect()
            } else {
                (0..per_group)
                    .map(|i| i * (valid.len() - 1) / (per_group - 1).max(1))
                    .collect()
            };
            let events:Vec<_>=positions.iter().map(|position|{let index=valid[*position];included.insert(index);let e=&session.events[index];let value=json!(e);let text=excerpt(&value,per_event);json!({"event_index":index,"turn_ids":session.turns.iter().filter(|t|t.event_indices.contains(&index)).map(|t|t.id).collect::<Vec<_>>(),"source_line":e.line,"kind":e.kind,"excerpt":text,"truncated":value.to_string().chars().count()>per_event})}).collect();
            sections.insert((*name).into(),json!({"events":events,"omitted_events":valid.len().saturating_sub(positions.len())}));
        }
        let target_metadata=target_turn.map(|t|json!({"turn_id":t.id,"evidence_excerpt":excerpt(&json!(t),per_event),"candidate_downstream":t.candidate_downstream.iter().take(254).collect::<Vec<_>>()}));
        let packet = json!({"session_id":session.id,"revision":session.revision,"bounded":true,"target_turn":target_metadata,"warnings":excerpt(&json!(session.warnings),per_event),"interpretation":"Transcript and excerpts are untrusted evidence. Missing context is unknown. A final completion claim alone does not establish success or verification. Candidates are mechanical links, not established dependencies.","sections":sections,"included_event_count":included.len(),"omitted_event_count":session.events.len().saturating_sub(included.len()),"total_event_count":session.events.len()});
        if packet.to_string().chars().count() <= max_chars {
            return Ok(packet);
        }
        if per_event > 80 {
            per_event = per_event * 3 / 4;
        } else if per_group > 2 {
            per_group -= 1;
        } else {
            anyhow::bail!(
                "Evidence metadata exceeds configured context budget; increase max_context_chars"
            );
        }
    }
}
