//! Process adapters. No review API keys, shell interpolation, or project writes.
use crate::config::Config;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::process::Command;

const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Every child starts a new process group. Killing that group also stops
        // tool subprocesses on timeout, cancellation, or completion.
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

async fn execute(
    mut command: Command,
    directory: &Path,
    prompt: &str,
    seconds: u64,
) -> Result<(String, String)> {
    let input = tempfile::NamedTempFile::new()?;
    fs::write(input.path(), prompt)?;
    let stdout = tempfile::NamedTempFile::new()?;
    let stderr = tempfile::NamedTempFile::new()?;
    command
        .current_dir(directory)
        .stdin(Stdio::from(fs::File::open(input.path())?))
        .stdout(Stdio::from(stdout.reopen()?))
        .stderr(Stdio::from(stderr.reopen()?))
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("Cannot start report CLI. Install the configured codex or claude executable and authenticate using that CLI")?;
    #[cfg(unix)]
    let _group = ProcessGroup(child.id().context("CLI has no process ID")?);
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    let mut check = tokio::time::interval(Duration::from_millis(100));
    let started = Instant::now();
    let mut last_progress = Instant::now();
    let status = loop {
        tokio::select! {
            result = child.wait() => break result?,
            _ = &mut timer => {
                let _ = child.kill().await;
                bail!("Report CLI exceeded its {seconds}s wall-clock budget; no result accepted");
            }
            _ = check.tick() => {
                if !prompt.is_empty() && last_progress.elapsed() >= Duration::from_secs(15) {
                    eprintln!("  Investigation running: {}s elapsed ({}s budget)", started.elapsed().as_secs(), seconds);
                    last_progress = Instant::now();
                }
                if stdout.as_file().metadata()?.len() + stderr.as_file().metadata()?.len() > MAX_OUTPUT_BYTES {
                    let _ = child.kill().await;
                    bail!("Report CLI output exceeded 8 MiB; no result accepted");
                }
            }
        }
    };
    ensure!(
        stdout.as_file().metadata()?.len() + stderr.as_file().metadata()?.len() <= MAX_OUTPUT_BYTES,
        "Report CLI output exceeded 8 MiB"
    );
    // Error streams may contain transcript data or credentials. Never echo them.
    ensure!(status.success(), "Report CLI exited unsuccessfully ({status}). Check the selected CLI's login and installation; no result accepted");
    Ok((
        fs::read_to_string(stdout.path())?,
        fs::read_to_string(stderr.path())?,
    ))
}

pub(crate) async fn cli_version(config: &Config) -> Result<String> {
    super::validate_review_config(config)?;
    let temp = tempfile::tempdir()?;
    let mut cmd = Command::new(&config.review.backend);
    cmd.arg("--version").env_remove(&config.jev.api_key_env);
    let (out, _) = execute(cmd, temp.path(), "", 10).await?;
    ensure!(!out.trim().is_empty(), "Report CLI returned no version");
    Ok(out.trim().to_owned())
}

pub(crate) async fn run_cli(directory: &Path, prompt: &str, config: &Config) -> Result<Value> {
    let schema = super::review::schema();
    fs::write(directory.join("schema.json"), serde_json::to_vec(&schema)?)?;
    let output_path = directory.join("answer.json");
    if output_path.exists() {
        fs::remove_file(&output_path)?;
    }
    let mut command = Command::new(&config.review.backend);
    // Jev credentials are never needed by a report agent. Preserve the CLI
    // environment for its own authentication, but exclude the Jev key.
    command.env_remove(&config.jev.api_key_env);
    match config.review.backend.as_str() {
        "codex" => {
            command.args([
                "exec",
                "--sandbox",
                "read-only",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "--ephemeral",
                "--json",
                "--color",
                "never",
                "--output-schema",
                "schema.json",
                "--output-last-message",
                "answer.json",
            ]);
            if let Some(model) = &config.review.model {
                command.args(["--model", model]);
            }
            command.arg("-");
        }
        "claude" => {
            // This is an explicitly requested independent report, even when
            // jta was launched from a Claude Code terminal.
            command.env_remove("CLAUDECODE");
            // Safe mode retains CLI login while disabling hooks, plugins, MCP,
            // skills, and automatic instruction loading. Bare mode loses OAuth.
            command
                .args([
                    "--print",
                    "--safe-mode",
                    "--no-session-persistence",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-mode",
                    "dontAsk",
                    "--tools",
                    "Read,Glob,Grep",
                    "--allowedTools",
                    "Read,Glob,Grep",
                    "--strict-mcp-config",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                    "--json-schema",
                ])
                .arg(schema.to_string());
            if let Some(model) = &config.review.model {
                command.args(["--model", model]);
            }
        }
        _ => bail!("configuration: unsupported report CLI"),
    }
    let start = Instant::now();
    let (stdout, _) = execute(command, directory, prompt, config.review.timeout_secs)
        .await
        .with_context(|| format!("{} report CLI failed", config.review.backend))?;
    let mut result = parse_events(&stdout, &config.review.backend)?;
    if config.review.backend == "codex" {
        ensure!(
            fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0) <= MAX_OUTPUT_BYTES,
            "CLI answer exceeded 8 MiB"
        );
        result["response"] = serde_json::from_slice(
            &fs::read(&output_path).context("Codex did not produce a final structured answer")?,
        )
        .context("Codex answer is not valid JSON")?;
    }
    ensure!(
        result["response"].is_object(),
        "Report CLI did not return a structured result"
    );
    result["elapsed_ms"] = json!(start.elapsed().as_millis() as u64);
    Ok(result)
}

fn parse_events(stdout: &str, backend: &str) -> Result<Value> {
    let mut events = Vec::new();
    let mut usage = Vec::new();
    let mut response = Value::Null;
    let mut completed = false;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value =
            serde_json::from_str(line).context("Report CLI emitted malformed JSON events")?;
        let kind = event["type"].as_str().unwrap_or("");
        ensure!(
            !matches!(kind, "error" | "turn.failed"),
            "Report CLI reported a failed turn; check CLI authentication and limits"
        );
        if backend == "codex" && kind == "turn.completed" {
            completed = true;
            if event["usage"].is_object() {
                usage.push(event["usage"].clone());
            }
        }
        if backend == "claude" && kind == "result" {
            ensure!(
                event["is_error"] != true && event["subtype"] == "success",
                "Claude investigation failed or exhausted its budget"
            );
            completed = true;
            response = event["structured_output"].clone();
            usage.push(
                json!({"usage":event["usage"],"model_usage":event["modelUsage"],
                "estimated_cost_usd":event["total_cost_usd"],"num_turns":event["num_turns"]}),
            );
        }
        // Keep a bounded event trace for inspection; the caller redacts it before caching.
        events.push(event);
    }
    ensure!(completed, "Report CLI did not report successful completion");
    Ok(json!({"response":response,"usage":usage,"events":events}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_backend_envelopes_and_rejects_failures() {
        let codex = parse_events(
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":12}}\n",
            "codex",
        )
        .unwrap();
        assert_eq!(codex["usage"][0]["input_tokens"], 12);
        let claude = parse_events(
            &json!({"type":"result","subtype":"success","is_error":false,
            "structured_output":{"summary":"ok"},"usage":{"input_tokens":10},"num_turns":3})
            .to_string(),
            "claude",
        )
        .unwrap();
        assert_eq!(claude["response"]["summary"], "ok");
        for text in [
            "{}",
            "not json",
            "{\"type\":\"turn.failed\"}",
            "{\"type\":\"result\",\"subtype\":\"error_max_turns\"}",
        ] {
            assert!(parse_events(text, "claude").is_err());
        }
    }
    #[tokio::test]
    #[cfg(unix)]
    async fn kills_timed_out_processes_and_suppresses_sensitive_errors() {
        let temp = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 10"]);
        assert!(execute(cmd, temp.path(), "", 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("wall-clock"));
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo secret-sentinel >&2; exit 1"]);
        assert!(!execute(cmd, temp.path(), "", 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("secret-sentinel"));
    }
}
