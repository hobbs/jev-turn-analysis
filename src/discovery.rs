//! Explicit project discovery. Callers supply roots; this module never reads HOME.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_LINES: usize = 256;

#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    pub project: PathBuf,
    pub agent: Option<String>,
    pub codex_home: PathBuf,
    pub claude_config_dir: PathBuf,
}
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredSession {
    pub path: PathBuf,
    pub agent: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryResult {
    pub project_root: PathBuf,
    pub sessions: Vec<DiscoveredSession>,
    pub scanned_files: usize,
    pub skipped_files: usize,
    pub malformed_files: usize,
    pub missing_cwd_files: usize,
    pub metadata_limited_files: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Nearest repository marker, including Git worktree .git files; otherwise supplied directory.
pub fn resolve_project(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot resolve project {}", path.display()))?;
    let directory = if canonical.is_file() {
        canonical
            .parent()
            .context("project file has no parent")?
            .to_path_buf()
    } else {
        canonical
    };
    for ancestor in directory.ancestors() {
        let marker = ancestor.join(".git");
        if marker.is_file() || marker.is_dir() {
            return Ok(ancestor.to_path_buf());
        }
    }
    Ok(directory)
}

/// Resolve existing symlink prefixes while retaining removed historical child directories.
pub fn recorded_project_root(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("recorded cwd must be absolute")
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            part => {
                normalized.push(part.as_os_str());
                if normalized.symlink_metadata().is_ok() {
                    normalized = normalized.canonicalize()?;
                }
            }
        }
    }
    let mut existing = normalized.as_path();
    let mut suffix = vec![];
    while !existing.exists() {
        suffix.push(
            existing
                .file_name()
                .context("no existing cwd ancestor")?
                .to_owned(),
        );
        existing = existing.parent().context("no cwd parent")?;
    }
    let mut canonical = existing.canonicalize()?;
    for part in suffix.into_iter().rev() {
        canonical.push(part);
    }
    for ancestor in canonical.ancestors() {
        let marker = ancestor.join(".git");
        if marker.is_file() || marker.is_dir() {
            return Ok(ancestor.to_owned());
        }
    }
    Ok(canonical)
}

/// Component-safe containment with separate nested repository/worktree identities.
pub fn matches_project(cwd: &Path, project: &Path) -> bool {
    let Ok(root) = recorded_project_root(cwd) else {
        return false;
    };
    root == project
        || (root.starts_with(project)
            && !root.join(".git").exists()
            && !project.join(".git").exists())
}

pub fn discover(options: &DiscoveryOptions) -> Result<DiscoveryResult> {
    let project_root = resolve_project(&options.project)?;
    let selected = options.agent.as_deref().unwrap_or("all");
    if !["all", "codex", "claude", "claude_code"].contains(&selected) {
        bail!("unknown discovery agent: {selected}")
    }
    let mut result = DiscoveryResult {
        project_root,
        sessions: vec![],
        scanned_files: 0,
        skipped_files: 0,
        malformed_files: 0,
        missing_cwd_files: 0,
        metadata_limited_files: 0,
        errors: vec![],
        warnings: vec![],
    };
    let mut roots = vec![];
    if selected == "all" || selected == "codex" {
        roots.push((options.codex_home.join("sessions"), "codex"));
        roots.push((options.codex_home.join("archived_sessions"), "codex"));
    }
    if selected == "all" || selected == "claude" || selected == "claude_code" {
        roots.push((options.claude_config_dir.join("projects"), "claude_code"));
    }
    for (root, agent) in roots {
        match fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                result.warnings.push(format!(
                    "{agent}: source directory absent: {}",
                    root.display()
                ));
                continue;
            }
            Err(e) => {
                result
                    .errors
                    .push(format!("{agent}: cannot inspect {}: {e}", root.display()));
                continue;
            }
            Ok(m) if m.file_type().is_symlink() => {
                result.warnings.push(format!(
                    "{agent}: source directory symlink skipped: {}",
                    root.display()
                ));
                continue;
            }
            Ok(m) if !m.is_dir() => {
                result.errors.push(format!(
                    "{agent}: source path is not a directory: {}",
                    root.display()
                ));
                continue;
            }
            Ok(_) => {}
        }
        let walker = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !matches!(
                    e.file_name().to_str(),
                    Some("subagents" | "subagent" | ".git")
                )
            });
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    result
                        .errors
                        .push(format!("{agent}: cannot scan source: {e}"));
                    continue;
                }
            };
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "jsonl")
            {
                continue;
            }
            result.scanned_files += 1;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("agent-"))
            {
                result.skipped_files += 1;
                continue;
            }
            match metadata_match(entry.path(), &result.project_root, agent) {
                Ok((matched, malformed, reason)) => {
                    if reason == "missing_cwd" {
                        result.missing_cwd_files += 1;
                    }
                    if reason == "metadata_limit" {
                        result.metadata_limited_files += 1;
                    }
                    if malformed {
                        result.malformed_files += 1;
                        result.warnings.push(format!(
                            "{agent}: malformed metadata records in {}",
                            entry.path().display()
                        ));
                    }
                    if matched {
                        result.sessions.push(DiscoveredSession {
                            path: entry.into_path(),
                            agent: agent.into(),
                        });
                    } else {
                        result.skipped_files += 1;
                    }
                }
                Err(e) => {
                    result.skipped_files += 1;
                    result
                        .errors
                        .push(format!("{agent}: {}: {e}", entry.path().display()));
                }
            }
        }
    }
    result.sessions.sort_by(|a, b| a.path.cmp(&b.path));
    result.sessions.dedup_by(|a, b| a.path == b.path);
    Ok(result)
}

fn metadata_match(path: &Path, project: &Path, agent: &str) -> Result<(bool, bool, &'static str)> {
    let mut malformed = false;
    let file = File::open(path).context("cannot read metadata")?;
    let mut reader = BufReader::new(file.take(MAX_METADATA_BYTES));
    let mut line = String::new();
    for _ in 0..MAX_METADATA_LINES {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok((
                false,
                malformed,
                if reader.get_ref().limit() == 0 {
                    "metadata_limit"
                } else {
                    "missing_cwd"
                },
            ));
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            malformed = true;
            continue;
        };
        if value["isSidechain"] == true || value["is_sidechain"] == true {
            return Ok((false, malformed, "helper"));
        }
        let payload = &value["payload"];
        if agent == "codex" && value["type"] == "session_meta" {
            let source = &payload["source"];
            if source.is_object() && source.get("subagent").is_some() {
                return Ok((false, malformed, "helper"));
            }
            if source.as_str().is_some_and(|s| s.contains("subagent")) {
                return Ok((false, malformed, "helper"));
            }
        }
        let cwd = if agent == "codex" {
            if value["type"] == "session_meta" || value["type"] == "turn_context" {
                payload["cwd"].as_str()
            } else {
                None
            }
        } else {
            value["cwd"].as_str()
        };
        if let Some(cwd) = cwd {
            return Ok((matches_project(Path::new(cwd), project), malformed, "cwd"));
        }
    }
    Ok((false, malformed, "metadata_limit"))
}

/// Stored project identity takes precedence; old evidence can use its recorded cwd.
pub fn session_project_root(session: &crate::model::Session) -> Option<PathBuf> {
    session
        .project_root
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| {
            session.repository.as_ref().and_then(|cwd| {
                let path = Path::new(cwd);
                if path.is_absolute() {
                    recorded_project_root(path).ok()
                } else {
                    None
                }
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn put(path: &Path, value: Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("{value}\n")).unwrap();
    }
    fn opts(root: &Path) -> DiscoveryOptions {
        DiscoveryOptions {
            project: root.join("project"),
            agent: None,
            codex_home: root.join("codex"),
            claude_config_dir: root.join("claude"),
        }
    }
    #[test]
    fn git_worktrees_nested_and_component_boundaries() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("project");
        fs::create_dir_all(p.join(".git")).unwrap();
        fs::create_dir_all(p.join("src/deep")).unwrap();
        let canonical = p.canonicalize().unwrap();
        assert_eq!(resolve_project(&p.join("src/deep")).unwrap(), canonical);
        assert!(matches_project(&p.join("removed/deep"), &canonical));
        assert!(!matches_project(
            &d.path().join("project-extra/removed"),
            &canonical
        ));
        let nested = p.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(".git"), "gitdir: elsewhere").unwrap();
        assert!(!matches_project(&nested, &canonical));
        assert_eq!(
            resolve_project(&nested).unwrap(),
            nested.canonicalize().unwrap()
        );
        let sibling = d.path().join("project-extra");
        fs::create_dir_all(&sibling).unwrap();
        assert!(!matches_project(&sibling, &canonical));
        let plain = d.path().join("plain");
        fs::create_dir_all(plain.join("sub")).unwrap();
        assert!(matches_project(
            &plain.join("sub"),
            &plain.canonicalize().unwrap()
        ));
        fs::write(plain.join("sub/.git"), "gitdir: elsewhere").unwrap();
        assert!(!matches_project(
            &plain.join("sub"),
            &plain.canonicalize().unwrap()
        ));
    }
    #[test]
    fn discovers_both_archives_and_metadata_not_encoded_directories() {
        let d = tempfile::tempdir().unwrap();
        let o = opts(d.path());
        fs::create_dir_all(o.project.join(".git")).unwrap();
        fs::create_dir_all(o.project.join("sub")).unwrap();
        let cwd = o.project.canonicalize().unwrap();
        let other = d.path().join("project-other");
        fs::create_dir_all(&other).unwrap();
        put(
            &o.codex_home.join("sessions/2026/a.jsonl"),
            json!({"type":"session_meta","payload":{"id":"a","cwd":cwd}}),
        );
        put(
            &o.codex_home.join("archived_sessions/b.jsonl"),
            json!({"type":"session_meta","payload":{"id":"b","cwd":cwd.join("sub")}}),
        );
        put(
            &o.claude_config_dir.join("projects/collision/a.jsonl"),
            json!({"type":"user","cwd":cwd,"message":{"role":"user","content":"hello"}}),
        );
        put(
            &o.claude_config_dir.join("projects/collision/b.jsonl"),
            json!({"type":"user","cwd":other,"message":{"role":"user","content":"wrong project"}}),
        );
        put(
            &o.claude_config_dir
                .join("projects/collision/subagents/agent-x.jsonl"),
            json!({"cwd":cwd}),
        );
        put(
            &o.codex_home.join("sessions/helper.jsonl"),
            json!({"type":"session_meta","payload":{"cwd":cwd,"source":{"subagent":{"thread_spawn":{}}}}}),
        );
        put(
            &o.codex_home.join("sessions/no-cwd.jsonl"),
            json!({"type":"session_meta","payload":{"id":"missing"}}),
        );
        let found = discover(&o).unwrap();
        assert_eq!(found.sessions.len(), 3);
        assert_eq!(
            found.sessions.iter().filter(|s| s.agent == "codex").count(),
            2
        );
        assert_eq!(found.skipped_files, 3);
        let only = discover(&DiscoveryOptions {
            agent: Some("claude".into()),
            ..o
        })
        .unwrap();
        assert_eq!(only.sessions.len(), 1);
    }
    #[test]
    fn malformed_bounded_and_absent_roots() {
        let d = tempfile::tempdir().unwrap();
        let o = opts(d.path());
        fs::create_dir_all(&o.project).unwrap();
        let empty = discover(&o).unwrap();
        assert_eq!(empty.warnings.len(), 3);
        let p = o.codex_home.join("sessions/broken.jsonl");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, "not-json\n").unwrap();
        let found = discover(&o).unwrap();
        assert_eq!(found.malformed_files, 1);
        assert_eq!(found.skipped_files, 1);
        assert_eq!(found.missing_cwd_files, 1);
        let late = o.codex_home.join("sessions/late.jsonl");
        let mut text = "{}\n".repeat(MAX_METADATA_LINES);
        text += &json!({"type":"session_meta","payload":{"cwd":o.project}}).to_string();
        fs::write(&late, text).unwrap();
        let bounded = discover(&o).unwrap();
        assert!(bounded.sessions.is_empty());
        assert_eq!(bounded.metadata_limited_files, 1);
    }
    #[cfg(unix)]
    #[test]
    fn no_symlink_traversal_but_project_alias_resolves() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let o = opts(d.path());
        fs::create_dir_all(o.project.join(".git")).unwrap();
        let outside = d.path().join("outside");
        put(
            &outside.join("hidden.jsonl"),
            json!({"type":"session_meta","payload":{"cwd":o.project}}),
        );
        fs::create_dir_all(o.codex_home.join("sessions")).unwrap();
        symlink(&outside, o.codex_home.join("sessions/link")).unwrap();
        assert!(discover(&o).unwrap().sessions.is_empty());
        symlink(&outside, o.project.join("escape")).unwrap();
        assert!(!matches_project(
            &o.project.join("escape/../removed"),
            &o.project.canonicalize().unwrap()
        ));
        let alias = d.path().join("alias");
        symlink(&o.project, &alias).unwrap();
        assert_eq!(
            resolve_project(&alias).unwrap(),
            o.project.canonicalize().unwrap()
        );
    }
    #[test]
    fn real_fixture_metadata_and_ingest_project() {
        let d = tempfile::tempdir().unwrap();
        let o = opts(d.path());
        fs::create_dir_all(o.project.join(".git")).unwrap();
        let cwd = o.project.canonicalize().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/public/claude-4c09dfa9-ff25-4b4c-a2d0-d2fd702c64ee.jsonl");
        let raw = fs::read_to_string(fixture).unwrap();
        let text = raw
            .lines()
            .map(|line| {
                let mut v: Value = serde_json::from_str(line).unwrap();
                v["cwd"] = json!(cwd);
                v.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let path = o
            .claude_config_dir
            .join("projects/unreliable-encoded-name/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        let found = discover(&o).unwrap();
        assert_eq!(found.sessions.len(), 1);
        let session = crate::ingest::parse_file(&path, &crate::config::Config::default()).unwrap();
        assert_eq!(
            session.project_root,
            Some(cwd.to_string_lossy().into_owned())
        );
        assert_eq!(session.repository, session.project_root);
        assert_eq!(session.turns.len(), 1);
    }
}
