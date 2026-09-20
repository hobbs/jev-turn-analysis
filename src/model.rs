use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PARSER_VERSION: &str = "2";
pub const RUBRIC_VERSION: &str = "1";
pub const SCHEMA_VERSION: u32 = 1;
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub revision: String,
    pub source_path: String,
    pub agent: String,
    pub repository: Option<String>,
    #[serde(default)]
    pub project_root: Option<String>,
    pub imported_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub events: Vec<Event>,
    pub turns: Vec<Turn>,
    pub warnings: Vec<String>,
    pub parser_version: String,
    pub redaction_fingerprint: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Event {
    pub line: usize,
    pub kind: String,
    pub timestamp: Option<String>,
    pub text: String,
    pub tool_name: Option<String>,
    pub call_id: Option<String>,
    pub input: Option<Value>,
    pub is_error: Option<bool>,
    pub usage: Option<Usage>,
    pub message_id: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Turn {
    pub id: u32,
    pub event_indices: Vec<usize>,
    pub intent: String,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub errors: Vec<String>,
    pub retry_of: Vec<u32>,
    pub candidate_downstream: Vec<u32>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub downstream_links: Vec<DownstreamLink>,
    pub verification: Vec<String>,
    pub usage: Option<Usage>,
    pub duration_ms: Option<u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Distribution {
    #[serde(default)]
    pub provider_confidence: Option<f64>,
    pub selected: String,
    pub probabilities: BTreeMap<String, f64>,
}
impl Distribution {
    pub(crate) fn higher_probability_alternative(&self) -> Option<(&str, f64)> {
        self.probabilities
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .filter(|(_, probability)| **probability > self.confidence() + 1e-9)
            .map(|(alternative, probability)| (alternative.as_str(), *probability))
    }

    pub fn confidence(&self) -> f64 {
        self.probabilities
            .get(&self.selected)
            .copied()
            .unwrap_or(0.0)
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub id: String,
    pub session_id: String,
    pub revision: String,
    pub created_at: String,
    pub config_fingerprint: String,
    pub rubric_version: String,
    pub session: BTreeMap<String, Distribution>,
    pub turns: BTreeMap<u32, TurnJudgment>,
    pub warnings: Vec<String>,
    pub usage: Option<Value>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnJudgment {
    pub answers: BTreeMap<String, Distribution>,
    pub secondary_opportunities: Vec<String>,
    pub downstream_turn: Option<u32>,
    pub inconsistencies: Vec<String>,
}
pub const SESSION_QUESTIONS: &[(&str, &[&str])] = &[
    (
        "task_outcome",
        &[
            "complete",
            "partially_complete",
            "failed",
            "abandoned",
            "unclear",
        ],
    ),
    (
        "outcome_verification",
        &[
            "verified",
            "claimed_but_unverified",
            "known_incomplete",
            "unclear",
        ],
    ),
    (
        "user_intervention",
        &[
            "none",
            "clarification_only",
            "corrected_agent",
            "redirected_approach",
            "unclear",
        ],
    ),
];
pub const TURN_QUESTIONS: &[(&str, &[&str])] = &[
    (
        "functional_role",
        &[
            "orient",
            "gather_context",
            "plan",
            "execute",
            "verify",
            "recover",
            "clarify",
            "communicate",
            "coordinate_or_wait",
            "other",
        ],
    ),
    (
        "immediate_effect",
        &[
            "advanced",
            "enabled_later_work",
            "no_observable_progress",
            "regressed",
            "unclear",
        ],
    ),
    (
        "downstream_use",
        &[
            "consumed_immediately",
            "consumed_later",
            "not_consumed",
            "superseded",
            "reverted",
            "unclear",
        ],
    ),
    (
        "counterfactual_necessity",
        &[
            "outcome_worse_or_impossible",
            "loop_less_informed",
            "no_material_difference",
            "outcome_improves",
            "unknowable",
        ],
    ),
    (
        "usefulness",
        &["essential", "useful", "neutral", "wasted", "harmful"],
    ),
    (
        "outcome_contribution",
        &[
            "helped_success",
            "reduced_risk",
            "hindered_success",
            "increased_risk",
            "no_material_effect",
            "unclear",
        ],
    ),
    (
        "opportunity",
        &[
            "redundant_work",
            "missing_or_poorly_selected_context",
            "wrong_or_ineffective_tool_use",
            "repeated_failed_approach",
            "planning_or_sequencing_problem",
            "missed_verification",
            "instruction_conflict_or_ambiguity",
            "missing_capability",
            "effective_behavior_to_reinforce",
            "environment_or_tool_limitation",
            "other",
            "none",
        ],
    ),
    (
        "remediation_surface",
        &[
            "AGENTS.md",
            "skill",
            "prompt",
            "tool_description",
            "tool_implementation",
            "orchestration_or_runtime",
            "context_packaging",
            "missing_capability",
            "other",
            "none",
        ],
    ),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownstreamLink {
    pub turn_id: u32,
    pub reason: String,
    pub evidence: Vec<String>,
}
