use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub jev: JevConfig,
    pub review: ReviewConfig,
    pub redaction: RedactionConfig,
    pub retention_days: u32,
    pub min_pattern_sessions: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JevConfig {
    pub endpoint: String,
    pub api_key_env: String,
    pub model: String,
    pub max_context_chars: usize,
    pub max_questions: usize,
    pub timeout_secs: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewConfig {
    pub provider: String,
    pub endpoint: String,
    pub api_key_env: String,
    pub model: String,
    pub timeout_secs: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RedactionConfig {
    pub enabled: bool,
    pub patterns: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            jev: JevConfig::default(),
            review: ReviewConfig::default(),
            redaction: RedactionConfig::default(),
            retention_days: 90,
            min_pattern_sessions: 3,
        }
    }
}
impl Default for JevConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://api.typesafe.ai/v1/systemone".into(),
            api_key_env: "JEV_API_KEY".into(),
            model: "jev-latest".into(),
            max_context_chars: 48000,
            max_questions: 64,
            timeout_secs: 120,
        }
    }
}
impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            endpoint: "https://api.openai.com/v1/chat/completions".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            model: "gpt-5.6-terra".into(),
            timeout_secs: 120,
        }
    }
}
impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            patterns: vec![],
        }
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn analysis_fingerprint(config: &Config) -> String {
    hash(
        &serde_json::to_vec(&(
            crate::model::PARSER_VERSION,
            crate::model::RUBRIC_VERSION,
            &config.jev,
            &config.redaction,
        ))
        .expect("serializable config"),
    )
}
