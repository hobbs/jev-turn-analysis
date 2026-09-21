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
    pub backend: String,
    pub model: Option<String>,
    /// Wall-clock limit for each CLI invocation, including tool use.
    pub timeout_secs: u64,
    /// Hard cap on project/category investigations in a report.
    pub max_investigations: usize,
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
            backend: "codex".into(),
            model: None,
            timeout_secs: 600,
            max_investigations: 20,
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

/// Shared user-level base for per-project data and global credentials.
pub fn data_home() -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    use std::path::PathBuf;
    std::env::var_os("JTA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".jta"))
        })
        .context("configuration: set HOME or JTA_HOME to store jta data")
}
