//! Deterministic aggregation. No semantic labels are inferred here.
use crate::model::{Analysis, Session};
use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, clap::Args, Serialize, Deserialize)]
pub struct Filters {
    #[arg(long, value_parser = ["codex", "claude", "claude_code", "all"])]
    #[serde(default)]
    pub agent: Option<String>,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub opportunity: Option<String>,
    #[arg(long)]
    pub usefulness: Option<String>,
    #[arg(long, default_value_t = 0.0)]
    pub min_confidence: f64,
    #[arg(long)]
    pub group_by: Option<String>,
    #[arg(long)]
    pub run: Option<String>,
}
/// Normalize the user-facing Claude alias without changing stored source identity.
pub fn normalize_agent(agent: Option<&str>) -> Result<Option<&'static str>> {
    match agent {
        None | Some("all") => Ok(None),
        Some("codex") => Ok(Some("codex")),
        Some("claude" | "claude_code") => Ok(Some("claude_code")),
        _ => bail!("agent must be codex, claude, claude_code, or all"),
    }
}
fn project_key(session: &Session) -> String {
    session
        .project_root
        .as_ref()
        .or(session.repository.as_ref())
        .cloned()
        .unwrap_or_else(|| "unknown".into())
}
fn matches_repo(session: &Session, requested: &str) -> bool {
    // Keep exact historical cwd filtering, also accepting canonical project roots.
    if session.repository.as_deref() == Some(requested)
        || session.project_root.as_deref() == Some(requested)
    {
        return true;
    }
    let requested = std::fs::canonicalize(requested).ok();
    requested.is_some_and(|path| {
        session
            .project_root
            .as_ref()
            .or(session.repository.as_ref())
            .is_some_and(|stored| std::fs::canonicalize(stored).ok().as_ref() == Some(&path))
    })
}
pub fn cutoff(s: &str) -> Result<DateTime<Utc>> {
    if let Some(n) = s.strip_suffix('d') {
        let days: i64 = n.parse()?;
        if !(0..=365000).contains(&days) {
            bail!("invalid day interval");
        }
        return Ok(Utc::now() - Duration::days(days));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}
pub fn cohort(
    sessions: &[Session],
    analyses: &[Analysis],
    f: &Filters,
) -> Result<Vec<(Session, Analysis)>> {
    if !(0.0..=1.0).contains(&f.min_confidence) {
        bail!("min-confidence must be between 0 and 1");
    }
    let agent = normalize_agent(f.agent.as_deref())?;
    let since = f.since.as_deref().map(cutoff).transpose()?;
    let mut out = Vec::new();
    for s in sessions {
        if agent.is_some_and(|agent| s.agent != agent) {
            continue;
        }
        if f.repo.as_ref().is_some_and(|r| !matches_repo(s, r)) {
            continue;
        }
        if let Some(c) = since {
            if s.started_at
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc) < c)
                .unwrap_or(true)
            {
                continue;
            }
        }
        if let Some(a) = analyses
            .iter()
            .filter(|a| a.session_id == s.id && a.revision == s.revision)
            .max_by_key(|a| &a.created_at)
        {
            out.push((s.clone(), a.clone()));
        }
    }
    Ok(out)
}
fn selected(a: &Analysis, q: &str) -> String {
    a.session
        .get(q)
        .map(|d| d.selected.clone())
        .unwrap_or_else(|| "unknown".into())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub opportunity: String,
    pub surface: String,
    pub sessions: usize,
    pub priority: usize,
    pub supporting: Vec<Value>,
    /// Distinct supporting sessions by original source agent.
    #[serde(default)]
    pub agent_counts: BTreeMap<String, usize>,
}
pub fn patterns(pairs: &[(Session, Analysis)]) -> Vec<Pattern> {
    let mut groups: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    for (s, a) in pairs {
        for (id, t) in &a.turns {
            let Some(o) = t.answers.get("opportunity") else {
                continue;
            };
            if o.selected == "none" {
                continue;
            }
            let surface = t
                .answers
                .get("remediation_surface")
                .map(|d| d.selected.clone())
                .unwrap_or_else(|| "unknown".into());
            groups.entry((o.selected.clone(),surface)).or_default().push(json!({"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"turn_id":id,"agent":s.agent,"project_root":s.project_root,"repository":s.repository,"confidence":o.confidence(),"outcome":selected(a,"task_outcome"),"inconsistencies":t.inconsistencies}));
        }
    }
    let mut out: Vec<_> = groups
        .into_iter()
        .map(|((opportunity, surface), supporting)| {
            let sessions = supporting
                .iter()
                .filter_map(|v| v["session_id"].as_str())
                .collect::<BTreeSet<_>>()
                .len();
            let priority = if supporting
                .iter()
                .any(|v| matches!(v["outcome"].as_str(), Some("failed" | "partially_complete")))
            {
                0
            } else if opportunity == "missed_verification" {
                1
            } else {
                2
            };
            let id = format!(
                "p_{}",
                &crate::config::hash(format!("{opportunity}:{surface}").as_bytes())[..12]
            );
            let mut by_agent: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for evidence in &supporting {
                if let (Some(agent), Some(session)) =
                    (evidence["agent"].as_str(), evidence["session_id"].as_str())
                {
                    by_agent
                        .entry(agent.into())
                        .or_default()
                        .insert(session.into());
                }
            }
            let agent_counts = by_agent
                .into_iter()
                .map(|(agent, sessions)| (agent, sessions.len()))
                .collect();
            Pattern {
                id,
                opportunity,
                surface,
                sessions,
                priority,
                supporting,
                agent_counts,
            }
        })
        .collect();
    out.sort_by_key(|p| (p.priority, std::cmp::Reverse(p.sessions), p.id.clone()));
    out
}
pub fn report(pairs: &[(Session, Analysis)], f: &Filters) -> Result<Value> {
    let agent = normalize_agent(f.agent.as_deref())?;
    let filtered: Vec<_> = pairs
        .iter()
        .filter(|(s, _)| {
            agent.is_none_or(|a| s.agent == a)
                && f.repo.as_ref().is_none_or(|repo| matches_repo(s, repo))
        })
        .cloned()
        .collect();
    let mut result = report_metrics(&filtered, f)?;
    let mut agents: BTreeMap<String, Vec<(Session, Analysis)>> = BTreeMap::new();
    let mut projects: BTreeMap<String, Vec<(Session, Analysis)>> = BTreeMap::new();
    for (s, a) in &filtered {
        agents
            .entry(s.agent.clone())
            .or_default()
            .push((s.clone(), a.clone()));
        projects
            .entry(project_key(s))
            .or_default()
            .push((s.clone(), a.clone()));
    }
    let breakdown = |groups: BTreeMap<String, Vec<(Session, Analysis)>>| -> Result<Value> {
        let mut out = serde_json::Map::new();
        for (key, pairs) in groups {
            let metrics = report_metrics(&pairs, f)?;
            out.insert(key,json!({"sessions":metrics["sessions"],"turns":metrics["turns"],"outcomes":metrics["counts"]["task_outcome"],"verification":metrics["counts"]["outcome_verification"],"resources":metrics["resources"],"excluded_by_question":metrics["excluded_by_question"],"uncertain_turns":metrics["uncertain_turns"],"session_ids":pairs.iter().map(|(s,_)|s.id.clone()).collect::<Vec<_>>()}));
        }
        Ok(Value::Object(out))
    };
    result["agent_breakdown"] = breakdown(agents)?;
    result["project_breakdown"] = breakdown(projects)?;
    result["comparison_note"]=json!("Agent and project breakdowns describe observed cohorts and their evidence coverage; differences do not establish causal agent quality.");
    Ok(result)
}
fn report_metrics(pairs: &[(Session, Analysis)], f: &Filters) -> Result<Value> {
    if !(0.0..=1.0).contains(&f.min_confidence) {
        bail!("min-confidence must be between 0 and 1");
    }
    let valid_groups = [
        "repository",
        "repo",
        "agent",
        "source_agent",
        "tool",
        "workflow_stage",
        "functional_role",
        "opportunity",
        "remediation_surface",
    ];
    if f.group_by
        .as_ref()
        .is_some_and(|g| !valid_groups.contains(&g.as_str()))
    {
        bail!("unsupported group-by field");
    }
    let mut counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut refs = Vec::new();
    let mut session_refs = Vec::new();
    let mut excluded = 0;
    let mut excluded_by_question: BTreeMap<String, usize> = BTreeMap::new();
    let mut uncertain = 0;
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let mut totals = [0u64; 3];
    let mut coverage = [0usize; 3];
    let mut session_coverage = [BTreeSet::new(), BTreeSet::new(), BTreeSet::new()];
    let mut elapsed = Vec::new();
    let mut wasted = 0u64;
    let mut wasted_coverage = 0;
    let mut filtered_pairs = vec![];
    for (s, a) in pairs {
        for (q, d) in &a.session {
            if d.confidence() >= f.min_confidence {
                *counts
                    .entry(q.clone())
                    .or_default()
                    .entry(d.selected.clone())
                    .or_default() += 1;
                session_refs.push(json!({"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"agent":s.agent,"project_root":s.project_root,"question":q}));
            } else {
                excluded += 1;
                *excluded_by_question.entry(q.clone()).or_default() += 1;
            }
        }
        let mut filtered = a.clone();
        filtered.turns.clear();
        for (id, t) in &a.turns {
            if !t.inconsistencies.is_empty() || t.answers.values().any(|d| d.confidence() < 0.6) {
                uncertain += 1;
            }
            if f.opportunity
                .as_ref()
                .is_some_and(|x| t.answers.get("opportunity").map(|d| &d.selected) != Some(x))
                || f.usefulness
                    .as_ref()
                    .is_some_and(|x| t.answers.get("usefulness").map(|d| &d.selected) != Some(x))
            {
                continue;
            }
            let required = [
                f.opportunity.as_ref().map(|_| "opportunity"),
                f.usefulness.as_ref().map(|_| "usefulness"),
                f.group_by.as_deref().and_then(|g| match g {
                    "workflow_stage" => Some("functional_role"),
                    "functional_role" | "opportunity" | "remediation_surface" => Some(g),
                    _ => None,
                }),
            ];
            let failed_required: Vec<_> = required
                .iter()
                .flatten()
                .filter(|q| {
                    t.answers
                        .get(**q)
                        .is_none_or(|d| d.confidence() < f.min_confidence)
                })
                .copied()
                .collect();
            let contradiction = t
                .inconsistencies
                .iter()
                .any(|reason| !reason.starts_with("Ambiguous distribution:"));
            if !failed_required.is_empty() || (f.min_confidence > 0.0 && contradiction) {
                excluded += 1;
                for q in failed_required {
                    *excluded_by_question.entry(q.into()).or_default() += 1;
                }
                if contradiction {
                    *excluded_by_question
                        .entry("cross_question_contradiction".into())
                        .or_default() += 1;
                }
                continue;
            }
            let mut included = vec![];
            for (q, d) in &t.answers {
                if d.confidence() >= f.min_confidence {
                    *counts
                        .entry(q.clone())
                        .or_default()
                        .entry(d.selected.clone())
                        .or_default() += 1;
                    included.push(q);
                } else {
                    excluded += 1;
                    *excluded_by_question.entry(q.clone()).or_default() += 1;
                }
            }
            if included.is_empty() {
                continue;
            }
            refs.push(json!({"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"agent":s.agent,"project_root":s.project_root,"turn_id":id,"included_questions":included}));
            if ["opportunity", "remediation_surface"].iter().all(|q| {
                t.answers
                    .get(*q)
                    .is_some_and(|d| d.confidence() >= f.min_confidence)
            }) {
                filtered.turns.insert(*id, t.clone());
            }
            let turn = s.turns.iter().find(|x| x.id == *id);
            if let Some(g) = &f.group_by {
                let vals = match g.as_str() {
                    "repo" | "repository" => {
                        vec![project_key(s)]
                    }
                    "agent" | "source_agent" => vec![s.agent.clone()],
                    "tool" => turn
                        .map(|t| {
                            if t.tools.is_empty() {
                                vec!["unknown".into()]
                            } else {
                                t.tools.clone()
                            }
                        })
                        .unwrap_or_default(),
                    _ => vec![t
                        .answers
                        .get(if g == "workflow_stage" {
                            "functional_role"
                        } else {
                            g
                        })
                        .map(|d| d.selected.clone())
                        .unwrap_or_else(|| "unknown".into())],
                };
                for v in vals {
                    *groups.entry(v).or_default() += 1;
                }
            }
            if let Some(u) = turn.and_then(|t| t.usage.as_ref()) {
                for (i, n) in [u.input_tokens, u.output_tokens, u.cached_input_tokens]
                    .iter()
                    .enumerate()
                {
                    if let Some(n) = n {
                        totals[i] += n;
                        coverage[i] += 1;
                        session_coverage[i].insert(s.id.clone());
                    }
                }
                if t.answers
                    .get("usefulness")
                    .is_some_and(|d| d.selected == "wasted" && d.confidence() >= f.min_confidence)
                {
                    if let (Some(i), Some(o)) = (u.input_tokens, u.output_tokens) {
                        wasted += i + o;
                        wasted_coverage += 1;
                    }
                }
            }
        }
        filtered_pairs.push((s.clone(), filtered));
        if let (Some(start), Some(end)) = (&s.started_at, &s.ended_at) {
            if let (Ok(start), Ok(end)) = (
                DateTime::parse_from_rfc3339(start),
                DateTime::parse_from_rfc3339(end),
            ) {
                let ms = (end - start).num_milliseconds();
                if ms >= 0 {
                    elapsed.push(ms);
                }
            }
        }
    }
    elapsed.sort();
    let median_session_ms = if elapsed.is_empty() {
        None
    } else if elapsed.len() % 2 == 0 {
        Some(elapsed[elapsed.len() / 2 - 1] as f64 / 2.0 + elapsed[elapsed.len() / 2] as f64 / 2.0)
    } else {
        Some(elapsed[elapsed.len() / 2] as f64)
    };
    let configurations = pairs
        .iter()
        .map(|(_, a)| a.config_fingerprint.clone())
        .collect::<BTreeSet<_>>();
    let resolved_models: BTreeSet<String> =
        pairs.iter().flat_map(|(_, a)| resolved_models(a)).collect();
    let evidence_warnings: Vec<_> = pairs.iter().filter(|(s,a)| !s.warnings.is_empty() || !a.warnings.is_empty()).map(|(s,a)| json!({"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"source_warnings":s.warnings,"analysis_warnings":a.warnings})).collect();
    let mut warnings = Vec::new();
    if configurations.len() > 1 {
        warnings.push("Mixed scoring configurations");
    }
    if resolved_models.len() > 1 {
        warnings.push("Mixed resolved Jev model versions");
    }
    if !evidence_warnings.is_empty() {
        warnings.push("Source or analysis evidence gaps are present; inspect evidence_warnings before interpreting outcomes");
    }
    let metric = |i: usize| json!({"total":if coverage[i]>0{Some(totals[i])}else{None},"turn_coverage":coverage[i],"session_coverage":session_coverage[i].len()});
    Ok(
        json!({"sessions":pairs.len(),"turns":refs.len(),"counts":counts,"groups":groups,"excluded_judgments":excluded,"excluded_by_question":excluded_by_question,"uncertain_turns":uncertain,"references":refs,"session_references":session_refs,"patterns":patterns(&filtered_pairs),"resources":{"input_tokens":metric(0),"output_tokens":metric(1),"cached_input_tokens":metric(2),"observed_wasted_tokens":if wasted_coverage>0{Some(wasted)}else{None},"wasted_token_turn_coverage":wasted_coverage,"timing_session_coverage":elapsed.len(),"elapsed_ms":if elapsed.is_empty(){None}else{Some(elapsed.iter().sum::<i64>())},"median_session_ms":median_session_ms},"configuration_fingerprints":configurations,"resolved_jev_models":resolved_models,"evidence_warnings":evidence_warnings,"warnings":warnings}),
    )
}

/// Actual provider model identities, including older analyses storing only request provenance.
pub fn resolved_models(analysis: &Analysis) -> BTreeSet<String> {
    analysis
        .usage
        .as_ref()
        .and_then(|u| u["requests"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| r["model"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Distribution, Turn, TurnJudgment, Usage};
    fn distribution(label: &str, p: f64) -> Distribution {
        Distribution {
            selected: label.into(),
            probabilities: BTreeMap::from([(label.into(), p), ("other".into(), 1.0 - p)]),
            ..Default::default()
        }
    }
    fn fixture() -> Vec<(Session, Analysis)> {
        let session = Session {
            id: "s_one".into(),
            revision: "revision".into(),
            turns: vec![
                Turn {
                    id: 1,
                    usage: Some(Usage {
                        input_tokens: Some(100),
                        output_tokens: None,
                        cached_input_tokens: None,
                    }),
                    ..Default::default()
                },
                Turn {
                    id: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let judgment = |opportunity: &str| TurnJudgment {
            answers: BTreeMap::from([
                ("usefulness".into(), distribution("wasted", 0.95)),
                ("opportunity".into(), distribution(opportunity, 0.9)),
                ("remediation_surface".into(), distribution("skill", 0.95)),
                ("secondary.other".into(), distribution("no", 0.51)),
            ]),
            inconsistencies: vec!["Ambiguous distribution: secondary.other".into()],
            ..Default::default()
        };
        let analysis = Analysis {
            id: "a_one".into(),
            session_id: session.id.clone(),
            revision: session.revision.clone(),
            session: BTreeMap::from([("task_outcome".into(), distribution("failed", 0.99))]),
            turns: BTreeMap::from([
                (1, judgment("missed_verification")),
                (2, judgment("redundant_work")),
            ]),
            ..Default::default()
        };
        vec![(session, analysis)]
    }
    #[test]
    fn unrelated_ambiguous_secondary_does_not_exclude_confident_metric() {
        let result = report(
            &fixture(),
            &Filters {
                min_confidence: 0.8,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["counts"]["usefulness"]["wasted"], 2);
        assert_eq!(result["excluded_by_question"]["secondary.other"], 2);
        assert_eq!(result["turns"], 2);
        assert_eq!(result["patterns"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn missing_resource_metrics_remain_null() {
        let result = report(&fixture(), &Filters::default()).unwrap();
        assert_eq!(result["resources"]["input_tokens"]["total"], 100);
        assert_eq!(result["resources"]["input_tokens"]["turn_coverage"], 1);
        assert!(result["resources"]["output_tokens"]["total"].is_null());
        assert!(result["resources"]["cached_input_tokens"]["total"].is_null());
        assert!(result["resources"]["observed_wasted_tokens"].is_null());
        assert!(result["resources"]["elapsed_ms"].is_null());
    }
    #[test]
    fn pattern_and_resource_cohorts_follow_turn_filters() {
        let result = report(
            &fixture(),
            &Filters {
                opportunity: Some("redundant_work".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["patterns"].as_array().unwrap().len(), 1);
        assert_eq!(result["patterns"][0]["opportunity"], "redundant_work");
        assert_eq!(result["references"][0]["turn_id"], 2);
        assert!(result["resources"]["input_tokens"]["total"].is_null());
    }
    #[test]
    fn pattern_surface_requires_confidence_and_contradictions_exclude() {
        let mut pairs = fixture();
        pairs[0]
            .1
            .turns
            .get_mut(&1)
            .unwrap()
            .answers
            .insert("remediation_surface".into(), distribution("skill", 0.55));
        pairs[0]
            .1
            .turns
            .get_mut(&2)
            .unwrap()
            .inconsistencies
            .push("Usefulness conflicts with counterfactual necessity".into());
        let result = report(
            &pairs,
            &Filters {
                min_confidence: 0.8,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result["patterns"].as_array().unwrap().len(), 0);
        assert_eq!(result["turns"], 1);
        assert_eq!(
            result["excluded_by_question"]["cross_question_contradiction"],
            1
        );
    }
    #[test]
    fn model_alias_drift_and_evidence_gaps_are_visible() {
        let mut pairs = fixture();
        pairs[0].1.usage = Some(json!({"requests":[{"model":"jev-version-a"}]}));
        pairs[0].0.warnings.push("Malformed source record".into());
        let mut later = pairs[0].clone();
        later.0.id = "s_two".into();
        later.1.usage = Some(json!({"requests":[{"model":"jev-version-b"}]}));
        pairs.push(later);
        let report = report(&pairs, &Filters::default()).unwrap();
        assert_eq!(
            report["configuration_fingerprints"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(report["resolved_jev_models"].as_array().unwrap().len(), 2);
        assert!(report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Mixed resolved Jev model versions"));
        assert_eq!(report["evidence_warnings"].as_array().unwrap().len(), 2);
    }
    fn mixed_fixture() -> Vec<(Session, Analysis)> {
        let mut pairs = fixture();
        pairs[0].0.agent = "codex".into();
        pairs[0].0.repository = Some("/project/src".into());
        pairs[0].0.project_root = Some("/project".into());
        let mut claude = pairs[0].clone();
        claude.0.id = "s_claude".into();
        claude.0.agent = "claude_code".into();
        claude.0.repository = Some("/project/tests".into());
        claude.1.session_id = claude.0.id.clone();
        claude.1.id = "a_claude".into();
        claude.0.turns[0].usage = Some(Usage {
            input_tokens: Some(30),
            output_tokens: Some(10),
            cached_input_tokens: None,
        });
        claude
            .1
            .session
            .insert("task_outcome".into(), distribution("complete", 0.99));
        claude.1.session.insert(
            "outcome_verification".into(),
            distribution("verified", 0.99),
        );
        pairs.push(claude);
        pairs
    }
    #[test]
    fn agent_alias_filters_preserve_source_identity() {
        let pairs = mixed_fixture();
        let sessions: Vec<_> = pairs.iter().map(|(s, _)| s.clone()).collect();
        let analyses: Vec<_> = pairs.iter().map(|(_, a)| a.clone()).collect();
        for alias in ["claude", "claude_code"] {
            let selected = cohort(
                &sessions,
                &analyses,
                &Filters {
                    agent: Some(alias.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].0.agent, "claude_code");
            assert_eq!(selected[0].0.repository.as_deref(), Some("/project/tests"));
        }
        assert_eq!(
            cohort(
                &sessions,
                &analyses,
                &Filters {
                    agent: Some("all".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .len(),
            2
        );
        assert!(cohort(
            &sessions,
            &analyses,
            &Filters {
                agent: Some("unknown".into()),
                ..Default::default()
            }
        )
        .is_err());
    }
    #[test]
    fn mixed_breakdowns_show_independent_resources_and_outcomes() {
        let pairs = mixed_fixture();
        let r = report(&pairs, &Filters::default()).unwrap();
        assert_eq!(r["sessions"], 2);
        assert_eq!(r["project_breakdown"]["/project"]["sessions"], 2);
        assert_eq!(r["agent_breakdown"]["codex"]["outcomes"]["failed"], 1);
        assert_eq!(
            r["agent_breakdown"]["claude_code"]["verification"]["verified"],
            1
        );
        assert_eq!(
            r["agent_breakdown"]["claude_code"]["resources"]["output_tokens"]["total"],
            10
        );
        assert!(r["agent_breakdown"]["codex"]["resources"]["output_tokens"]["total"].is_null());
        assert_eq!(r["agent_breakdown"]["codex"]["turns"], 2);
        let patterns = patterns(&pairs);
        assert_eq!(patterns[0].agent_counts.get("codex"), Some(&1));
        assert_eq!(patterns[0].agent_counts.get("claude_code"), Some(&1));
        assert_eq!(patterns[0].sessions, 2);
        assert!(patterns[0]
            .supporting
            .iter()
            .all(|r| r["agent"].is_string()));
        let filtered = report(
            &pairs,
            &Filters {
                agent: Some("codex".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered["sessions"], 1);
        assert!(filtered["agent_breakdown"].get("claude_code").is_none());
    }
    #[test]
    fn project_filters_preserve_legacy_exact_cwd() {
        let pairs = mixed_fixture();
        let sessions: Vec<_> = pairs.iter().map(|(s, _)| s.clone()).collect();
        let analyses: Vec<_> = pairs.iter().map(|(_, a)| a.clone()).collect();
        assert_eq!(
            cohort(
                &sessions,
                &analyses,
                &Filters {
                    repo: Some("/project".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            cohort(
                &sessions,
                &analyses,
                &Filters {
                    repo: Some("/project/src".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .len(),
            1
        );
        assert!(cohort(
            &sessions,
            &analyses,
            &Filters {
                repo: Some("/proj".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty());
        let mut legacy = serde_json::to_value(&sessions[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("project_root");
        let old: Session = serde_json::from_value(legacy).unwrap();
        assert!(old.project_root.is_none());
        assert!(matches_repo(&old, "/project/src"));
    }
}
