use crate::config::RedactionConfig;
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::Value;

fn placeholder(value: &str) -> String {
    format!(
        "[REDACTED:{}]",
        &crate::config::hash(value.as_bytes())[..12]
    )
}

pub struct Redactor {
    patterns: Vec<Regex>,
}
impl Redactor {
    pub fn new(config: &RedactionConfig) -> Result<Self> {
        let mut patterns = vec![];
        if config.enabled {
            for pattern in [
                r"(?i)\b(?:sk|ghp|gho|github_pat|xoxb|xoxp)-?[A-Za-z0-9_\-]{20,}\b",
                r"(?i)(?:bearer\s+)[A-Za-z0-9._~+/\-=]{8,}",
                r#"(?i)(?:api[_-]?key|access[_-]?token|secret|password|authorization)[\"']?\s*[=:]\s*[\"']?[^\s\"',;}]{4,}"#,
                r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
                r"https?://[^/\s:@]+:[^/\s@]+@[^/\s]+",
                r"\bAKIA[0-9A-Z]{16}\b",
            ] {
                patterns.push(Regex::new(pattern)?);
            }
            for pattern in &config.patterns {
                patterns.push(Regex::new(pattern).context("invalid custom redaction regex")?);
            }
        }
        Ok(Self { patterns })
    }
    pub fn text(&self, input: &str) -> String {
        if (input.starts_with('{') || input.starts_with('[')) && !self.patterns.is_empty() {
            if let Ok(mut value) = serde_json::from_str::<Value>(input) {
                if value.is_object() || value.is_array() {
                    self.value(&mut value);
                    return value.to_string();
                }
            }
        }
        self.patterns.iter().fold(input.to_owned(), |s, p| {
            p.replace_all(&s, |caps: &regex::Captures| placeholder(&caps[0]))
                .into_owned()
        })
    }
    pub fn value(&self, value: &mut Value) {
        match value {
            Value::String(s) => *s = self.text(s),
            Value::Array(a) => a.iter_mut().for_each(|v| self.value(v)),
            Value::Object(m) => {
                for (k, v) in m.iter_mut() {
                    let key = k.to_ascii_lowercase();
                    if !self.patterns.is_empty()
                        && [
                            "api_key",
                            "apikey",
                            "access_token",
                            "secret",
                            "password",
                            "authorization",
                        ]
                        .contains(&key.as_str())
                    {
                        if !v
                            .as_str()
                            .is_some_and(|s| s.starts_with("[REDACTED:") && s.ends_with(']'))
                        {
                            *v = Value::String(placeholder(v.as_str().unwrap_or(&v.to_string())));
                        }
                    } else {
                        self.value(v);
                    }
                }
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_distinct_and_idempotent() {
        let r = Redactor::new(&RedactionConfig::default()).unwrap();
        let mut a = serde_json::json!({"api_key":"first-secret", "password":"second-secret"});
        r.value(&mut a);
        assert_ne!(a["api_key"], a["password"]);
        let once = a.clone();
        r.value(&mut a);
        assert_eq!(a, once);
        let output = r.text(r#"{"access_token":"embedded-secret"}"#);
        assert!(!output.contains("embedded-secret"));
        let disabled = Redactor::new(&RedactionConfig {
            enabled: false,
            patterns: vec![],
        })
        .unwrap();
        assert_eq!(disabled.text("password=hello"), "password=hello");
    }
    #[test]
    fn nested_secrets() {
        let r = Redactor::new(&RedactionConfig::default()).unwrap();
        let mut v =
            serde_json::json!({"x":["Authorization: Bearer abcdefghijklmnop"],"api_key":"abc"});
        r.value(&mut v);
        assert!(!v.to_string().contains("abcdefghijklmnop"));
        assert!(v["api_key"].as_str().unwrap().starts_with("[REDACTED:"));
    }
    #[test]
    fn custom_and_disabled() {
        let r = Redactor::new(&RedactionConfig {
            enabled: true,
            patterns: vec!["private\\.org".into()],
        })
        .unwrap();
        assert!(r.text("private.org").starts_with("[REDACTED:"));
        assert!(Redactor::new(&RedactionConfig {
            enabled: true,
            patterns: vec!["[".into()]
        })
        .is_err());
    }
}
