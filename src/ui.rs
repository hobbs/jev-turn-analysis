//! Human-readable presentation. JSON payloads retain their exact values.
use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;
use std::{
    cell::Cell,
    fmt::Write as _,
    future::Future,
    io::{self, IsTerminal, Write},
    time::Instant,
};

tokio::task_local! { static REQUESTS: Cell<u64>; }
tokio::task_local! { static QUIET: bool; }
pub async fn quiet<T>(enabled: bool, future: impl Future<Output = T>) -> T {
    QUIET.scope(enabled, future).await
}
pub fn is_quiet() -> bool {
    QUIET.try_with(|quiet| *quiet).unwrap_or(false)
}
pub async fn track<T>(future: impl Future<Output = T>) -> T {
    REQUESTS.scope(Cell::new(0), future).await
}
pub fn requests() -> u64 {
    REQUESTS.try_with(Cell::get).unwrap_or(0)
}
pub fn request_started() -> u64 {
    REQUESTS
        .try_with(|n| {
            n.set(n.get() + 1);
            n.get()
        })
        .unwrap_or(0)
}
pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}
pub fn choose_provider(current: &str) -> Result<String> {
    loop {
        eprint!("Optional LLM review: none / openai / openrouter [{current}]: ");
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        let answer = answer.trim();
        if answer.is_empty() {
            return Ok(current.to_owned());
        }
        if ["none", "openai", "openrouter"].contains(&answer) {
            return Ok(answer.to_owned());
        }
        eprintln!("Choose none, openai, or openrouter.");
    }
}
pub fn number(n: u64) -> String {
    for (unit, scale) in [
        ("T", 1_000_000_000_000u64),
        ("B", 1_000_000_000),
        ("M", 1_000_000),
        ("K", 1_000),
    ] {
        if n >= scale {
            return format!("{:.1}{unit}", n as f64 / scale as f64);
        }
    }
    n.to_string()
}
pub fn heading(text: &str) -> String {
    if io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb")
    {
        format!("\x1b[1;36m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}
pub fn duration(ms: f64) -> String {
    let seconds = (ms / 1000.0).max(0.0) as u64;
    if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{:.1}s", ms / 1000.0)
    }
}
pub fn counts(counts: &Value, bars: bool) {
    if let Some(counts) = counts.as_object() {
        let total = counts
            .values()
            .filter_map(Value::as_u64)
            .sum::<u64>()
            .max(1);
        for (label, count) in counts {
            let n = count.as_u64().unwrap_or(0);
            let bar = if bars {
                "█".repeat((n as f64 / total as f64 * 16.0).round() as usize)
            } else {
                String::new()
            };
            println!(
                "  {:<28} {:>6}  {bar}",
                label.replace('_', " "),
                value(count)
            );
        }
    }
}
pub fn value(v: &Value) -> String {
    if let Some(n) = v.as_u64() {
        number(n)
    } else {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    }
}

/// One live session and a batch-level overall indicator. Completed rows are printed
/// into scrollback rather than retaining a progress bar for every historical session.
pub struct AnalysisProgress {
    display: Option<MultiProgress>,
    overall: ProgressBar,
    total: usize,
}
#[derive(Clone)]
struct SessionFeedback {
    bar: ProgressBar,
    overall: ProgressBar,
}
tokio::task_local! { static SESSION: SessionFeedback; }

fn animated_stderr() -> bool {
    io::stderr().is_terminal() && std::env::var("TERM").as_deref() != Ok("dumb")
}
fn progress_style(template: &str) -> ProgressStyle {
    let template = if std::env::var_os("NO_COLOR").is_some() {
        template.replace(".cyan", "").replace(".dim", "")
    } else {
        template.to_owned()
    };
    ProgressStyle::with_template(&template)
        .expect("valid progress template")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
        .progress_chars("━╸─")
}
impl AnalysisProgress {
    pub fn new(total: usize) -> Self {
        let display = animated_stderr()
            .then(|| MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(12)));
        let overall = if let Some(display) = &display {
            let bar = display.add(ProgressBar::new(total as u64));
            bar.set_style(progress_style(
                "  Overall [{wide_bar:.cyan}] {pos}/{len} sessions · {msg} · {elapsed_precise}",
            ));
            bar.set_message("0 API calls");
            bar.enable_steady_tick(std::time::Duration::from_millis(100));
            bar
        } else {
            ProgressBar::hidden()
        };
        Self {
            display,
            overall,
            total,
        }
    }
    pub fn session(&self, index: usize, session: &crate::model::Session) -> SessionProgress {
        let agent = match session.agent.as_str() {
            "claude_code" => "Claude Code",
            "codex" => "Codex",
            other => other,
        };
        let label = format!(
            "[{}/{}] {agent} {} · {} turns",
            index + 1,
            self.total,
            session.id,
            number(session.turns.len() as u64)
        );
        let feedback = self.display.as_ref().map(|display| {
            let bar = display.insert_before(&self.overall, ProgressBar::new_spinner());
            bar.set_style(progress_style(
                "  {spinner:.cyan} {prefix}\n    {wide_msg:.dim}",
            ));
            bar.set_prefix(label.clone());
            bar.set_message("Preparing this session…");
            bar.enable_steady_tick(std::time::Duration::from_millis(80));
            SessionFeedback {
                bar,
                overall: self.overall.clone(),
            }
        });
        if feedback.is_none() {
            eprintln!("{label}");
        }
        SessionProgress {
            feedback,
            display: self.display.clone(),
            overall: self.overall.clone(),
            label,
            started: Instant::now(),
            initial_calls: requests(),
            finished: false,
        }
    }
    pub fn error(&self, message: &str) {
        if let Some(display) = &self.display {
            let _ = display.println(message);
        } else {
            eprintln!("{message}");
        }
    }
    pub fn finish(&self) {
        self.overall.finish_and_clear();
    }
}
impl Drop for AnalysisProgress {
    fn drop(&mut self) {
        self.overall.finish_and_clear();
    }
}
pub struct SessionProgress {
    feedback: Option<SessionFeedback>,
    display: Option<MultiProgress>,
    overall: ProgressBar,
    label: String,
    started: Instant,
    initial_calls: u64,
    finished: bool,
}
impl SessionProgress {
    pub async fn run<T>(&self, future: impl Future<Output = T>) -> T {
        if let Some(feedback) = &self.feedback {
            SESSION.scope(feedback.clone(), future).await
        } else {
            future.await
        }
    }
    pub fn saving(&self) {
        if let Some(feedback) = &self.feedback {
            feedback.bar.set_message("Saving analysis…");
        }
    }
    pub fn finish(&mut self, status: &str) {
        let marker = if matches!(status, "failed" | "interrupted") {
            "×"
        } else {
            "✓"
        };
        let detail = if status == "cached" {
            "cached · no API calls".to_owned()
        } else {
            let calls = requests().saturating_sub(self.initial_calls);
            format!(
                "{status} · {calls} API {} · {}",
                if calls == 1 { "call" } else { "calls" },
                duration(self.started.elapsed().as_secs_f64() * 1000.0)
            )
        };
        let line = format!("  {marker} {} · {detail}", self.label);
        if let Some(feedback) = &self.feedback {
            feedback.bar.finish_and_clear();
            if let Some(display) = &self.display {
                display.remove(&feedback.bar);
                let _ = display.println(&line);
            }
        } else {
            eprintln!("{line}");
        }
        self.overall.inc(1);
        self.finished = true;
    }
}
impl Drop for SessionProgress {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("interrupted");
        }
    }
}
/// Starting or retrying a request never advances progress; only validated batches do.
pub fn scoring_batch(completed: usize, total: usize, session: &str) {
    if SESSION
        .try_with(|state| {
            if state.bar.length().is_none() {
                state.bar.set_style(progress_style("  {spinner:.cyan} {prefix}\n    [{wide_bar:.cyan}] {pos}/{len} batches · {elapsed_precise}\n    {wide_msg:.dim}"));
            }
            state.bar.set_length(total as u64);
            state.bar.set_position(completed as u64);
            state
                .bar
                .set_message(format!("Scoring batch {} of {total}…", completed + 1));
        })
        .is_err()
    {
        eprintln!("  Scoring batch {}/{} for {session}", completed + 1, total);
    }
}
pub fn scored_batch(completed: usize) {
    let _ = SESSION.try_with(|state| state.bar.set_position(completed as u64));
}
pub fn retry(message: String) {
    if is_quiet() {
        return;
    }
    if SESSION
        .try_with(|state| state.bar.set_message(message.clone()))
        .is_err()
    {
        eprintln!("  {message}");
    }
}

/// An in-place spinner only for terminals; redirected stderr remains line-oriented.
pub struct Activity {
    task: Option<tokio::task::JoinHandle<()>>,
    started: Instant,
    silent: bool,
}
impl Activity {
    pub fn new(label: String) -> Self {
        let started = Instant::now();
        if is_quiet() {
            return Self {
                task: None,
                started,
                silent: true,
            };
        }
        if SESSION
            .try_with(|state| {
                state
                    .bar
                    .set_message(format!("{label} · waiting for response"));
                state
                    .overall
                    .set_message(format!("{} API calls", requests()));
            })
            .is_ok()
        {
            return Self {
                task: None,
                started,
                silent: true,
            };
        }
        let animated = animated_stderr();
        eprintln!("{label}");
        let task = animated.then(|| {
            tokio::spawn(async move {
                let frames = ['|', '/', '-', '\\'];
                let mut i = 0;
                loop {
                    eprint!(
                        "\r\x1b[2K  {} Waiting for response · {}s",
                        frames[i % frames.len()],
                        started.elapsed().as_secs()
                    );
                    let _ = io::stderr().flush();
                    i += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                }
            })
        });
        Self {
            task,
            started,
            silent: false,
        }
    }
}
impl Drop for Activity {
    fn drop(&mut self) {
        if self.silent {
            return;
        }
        if let Some(task) = &self.task {
            task.abort();
            eprint!("\r\x1b[2K");
        }
        eprintln!(
            "  Request finished after {:.1}s",
            self.started.elapsed().as_secs_f64()
        );
    }
}
pub fn review_scope(s: &Value) -> String {
    let selected = s["selected_patterns"].as_u64().unwrap_or(0);
    let eligible = s["eligible_patterns"].as_u64().unwrap_or(0);
    let sessions = s["sampled_sessions"].as_u64().unwrap_or(0);
    if s["isolated"] == true {
        return format!("Reviewing one session in isolation ({selected} patterns); findings may not generalize.");
    }
    if s["preliminary"] == true {
        return format!("Preliminary review of {selected} of {eligible} available patterns, sampled from {sessions} sessions. No pattern met the recurrence threshold of {} sessions. Suggestions are tentative and may not generalize.", s["recurrence_minimum_sessions"]);
    }
    let mut line = format!("Reviewing {selected} of {eligible} eligible patterns, sampled from {sessions} sessions. Each pattern needs support from {} sessions.", s["minimum_sessions"]);
    if selected < eligible {
        line.push_str(
            " The pattern limit bounds cost and context; use --top N or --all to expand it.",
        );
    }
    line
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn paragraph(out: &mut String, v: &Value, key: &str, label: &str) {
    let content = text(v, key);
    if !content.is_empty() {
        let _ = writeln!(out, "{label}{content}\n");
    }
}
fn code(out: &mut String, content: &str) {
    // Indented blocks are safe even when proposed instructions contain Markdown fences.
    for line in content.lines() {
        let _ = writeln!(out, "    {line}");
    }
    out.push('\n');
}
pub fn recommendation(r: &Value) -> String {
    let mut out = format!("### {}\n\n", text(r, "title"));
    paragraph(&mut out, r, "proposed_change", "");
    paragraph(&mut out, r, "observed_pattern", "What happened: ");
    paragraph(&mut out, r, "outcome_effect", "Why it matters: ");
    paragraph(&mut out, r, "scope", "Applies to: ");
    paragraph(&mut out, r, "project_root", "Project: ");
    paragraph(&mut out, r, "remediation_surface", "Change in: ");
    if let Some(targets) = r["targets"].as_array() {
        for target in targets {
            let _ = writeln!(
                out,
                "{}: {}\n",
                if target["action"] == "create" {
                    "Create"
                } else {
                    "Edit"
                },
                text(target, "path")
            );
            if !text(target, "before").is_empty() {
                out.push_str("Replace:\n\n");
                code(&mut out, text(target, "before"));
            }
            out.push_str("Suggested wording:\n\n");
            code(&mut out, text(target, "after"));
            paragraph(&mut out, target, "rationale", "Why here: ");
            if let Some(refs) = target["context_refs"].as_array() {
                for reference in refs {
                    let _ = writeln!(out, "From {}:\n", text(reference, "path"));
                    code(&mut out, text(reference, "quote"));
                }
            }
        }
    }
    paragraph(&mut out, r, "evaluation_plan", "How to check the change: ");
    paragraph(&mut out, r, "risk", "Tradeoff: ");
    paragraph(&mut out, r, "uncertainty", "What remains uncertain: ");
    if r["isolated"] == true {
        out.push_str("This is an isolated finding from one session.\n\n");
    }
    if r["preliminary"] == true {
        let _ = writeln!(out, "Preliminary suggestion supported by {} session(s); recurrence has not been established.\n", r["supporting_sessions"]);
    }
    if let Some(examples) = r["counterexamples"].as_array().filter(|a| !a.is_empty()) {
        out.push_str("Where this may not apply:\n\n");
        for example in examples.iter().filter_map(Value::as_str) {
            let _ = writeln!(out, "- {example}");
        }
        out.push('\n');
    }
    if let Some(refs) = r["supporting_refs"].as_array() {
        out.push_str("See the original turns:\n\n");
        for reference in refs {
            let _ = writeln!(
                out,
                "- `jta show {} --turn {}`",
                text(reference, "session_id"),
                reference["turn_id"]
            );
        }
        out.push('\n');
    }
    let _ = writeln!(out, "Reference: {}\n", text(r, "id"));
    out
}
pub fn review_report(data: &Value) -> String {
    let mut out = String::from("# Review report\n\n## Summary\n\n");
    let rs = data["recommendations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let grounded: Vec<_> = rs
        .iter()
        .filter(|r| r["targets"].as_array().is_some_and(|a| !a.is_empty()))
        .collect();
    paragraph(
        &mut out,
        data,
        "summary",
        if grounded.is_empty() {
            "Model explanation: "
        } else {
            ""
        },
    );
    if grounded.is_empty() {
        if data["api_calls"].as_u64() == Some(0) {
            out.push_str("Model review was skipped. This report contains statistics only; no assessment of possible workflow improvements was made.\n\n");
        } else {
            out.push_str("No actionable recommendations in this review.\n\n");
            if data["api_calls"].as_u64().is_some_and(|n| n > 0)
                && text(data, "summary").trim().is_empty()
                && !data["skipped_recommendations"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
            {
                out.push_str("The model did not provide an explanation for returning no recommendations.\n\n");
            }
        }
    } else {
        let _ = writeln!(out, "{} proposed changes to your agent workflow. Start with the first recommendation below. No project files have been changed.\n", grounded.len());
        for (i, r) in grounded.iter().enumerate() {
            let _ = writeln!(
                out,
                "{}. {} — {}",
                i + 1,
                text(r, "title"),
                text(r, "proposed_change")
            );
        }
        out.push('\n');
    }
    if data["selection"].is_object() {
        let _ = writeln!(out, "{}\n", review_scope(&data["selection"]));
    }
    paragraph(&mut out, data, "message", "");
    if grounded.is_empty() {
        if let Some(themes) = data["themes"].as_array().filter(|a| !a.is_empty()) {
            out.push_str("### Model observations\n\n");
            for theme in themes.iter().filter_map(Value::as_str) {
                let _ = writeln!(out, "- {theme}");
            }
            out.push('\n');
        }
    }
    if !grounded.is_empty() {
        out.push_str("## Themes\n\n");
        if let Some(themes) = data["themes"].as_array().filter(|a| !a.is_empty()) {
            for theme in themes.iter().filter_map(Value::as_str) {
                let _ = writeln!(out, "- {theme}");
            }
        } else {
            for r in &grounded {
                let _ = writeln!(out, "- {}", text(r, "observed_pattern"));
            }
        }
        out.push_str("\n## Recommendations\n\n");
        for r in grounded {
            out.push_str(&recommendation(r));
        }
    }
    for r in &rs {
        if !r["targets"].as_array().is_some_and(|a| !a.is_empty()) {
            let _ = writeln!(out, "Older review {} has no project file targets and is omitted. Run jta report to replace it; jta show {} retains the original record.\n", text(r,"id"), text(r,"id"));
        }
    }
    if let Some(warnings) = data["warnings"].as_array().filter(|a| !a.is_empty()) {
        out.push_str("## Review notes\n\n");
        for warning in warnings.iter().filter_map(Value::as_str) {
            let _ = writeln!(out, "- {warning}");
        }
        out.push('\n');
    }
    if let Some(n) = data["api_calls"].as_u64() {
        let _ = writeln!(
            out,
            "Provider HTTP attempts: {n} (including retries and corrections).\n"
        );
    }
    out
}

#[cfg(test)]
pub(crate) async fn observe_session<T>(future: impl Future<Output = T>) -> (T, u64) {
    let bar = ProgressBar::hidden();
    let state = SessionFeedback {
        bar: bar.clone(),
        overall: ProgressBar::hidden(),
    };
    let result = SESSION.scope(state, future).await;
    (result, bar.position())
}
