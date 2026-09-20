use crate::{
    analytics::{self, Filters},
    config::{self, Config},
    discovery, ingest,
    model::{Analysis, Session},
    services,
    store::Workspace,
    ui,
};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Format {
    Text,
    Json,
    Markdown,
}
#[derive(Parser, Debug)]
#[command(
    name = "jta",
    version,
    about = "Find what helps your coding agent finish tasks — and what to improve"
)]
pub struct Cli {
    /// Use <DIR>/.jta for storage instead of the per-project user directory.
    #[arg(long, global = true, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub format: Format,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(clap::Args, Debug, Default)]
pub struct DiscoveryArgs {
    /// Project checkout to match (defaults to invocation directory).
    #[arg(long)]
    project: Option<PathBuf>,
    /// Select one source agent, or both.
    #[arg(long,value_parser=["codex","claude","claude_code","all"],default_value="all")]
    agent: String,
    /// Codex data root (otherwise CODEX_HOME or ~/.codex).
    #[arg(long)]
    codex_home: Option<PathBuf>,
    /// Claude Code data root (otherwise CLAUDE_CONFIG_DIR or ~/.claude).
    #[arg(long)]
    claude_config_dir: Option<PathBuf>,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Set up API keys and project preferences; supplied flags update existing settings.
    Init {
        /// Use defaults and flags without asking questions.
        #[arg(long)]
        no_input: bool,
        #[arg(long)]
        jev_endpoint: Option<String>,
        #[arg(long)]
        jev_api_key_env: Option<String>,
        #[arg(long)]
        jev_model: Option<String>,
        #[arg(long)]
        review_provider: Option<String>,
        #[arg(long)]
        review_endpoint: Option<String>,
        #[arg(long)]
        review_model: Option<String>,
        #[arg(long)]
        review_api_key_env: Option<String>,
        #[arg(long)]
        retention_days: Option<u32>,
        #[arg(long)]
        min_pattern_sessions: Option<usize>,
        #[arg(long)]
        redact_pattern: Vec<String>,
        #[arg(long)]
        no_redaction: bool,
    },
    /// Import exported session logs locally; no API calls.
    Import { path: PathBuf },
    /// Analyze project sessions discovered locally, an explicit path, or all imported sessions.
    Analyze {
        #[command(flatten)]
        discovery: DiscoveryArgs,
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        all: bool,
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// List matching native sessions locally without importing or scoring.
    Discover {
        #[command(flatten)]
        discovery: DiscoveryArgs,
    },
    /// Inspect a session, turn, pattern, or saved recommendation.
    Show {
        id: String,
        #[arg(long)]
        turn: Option<u32>,
        #[arg(long)]
        evidence: bool,
        #[arg(long)]
        run: Option<String>,
    },
    /// Generate a Markdown report with statistics and proposed workflow improvements.
    Report {
        #[command(flatten)]
        filters: Filters,
        /// Review this many priority patterns (default: 5, to bound cost and context).
        #[arg(long, conflicts_with_all = ["all", "session"], value_parser = clap::value_parser!(u32).range(1..))]
        top: Option<u32>,
        /// Include every eligible pattern; may send a much larger request.
        #[arg(long, conflicts_with = "session")]
        all: bool,
        /// Skip preliminary review when no pattern meets the recurrence threshold.
        #[arg(long, conflicts_with = "session")]
        recurring_only: bool,
        #[arg(long)]
        session: Option<String>,
        /// Include an additional instruction, skill, or implementation file in review context.
        #[arg(long = "context", value_name = "FILE")]
        context: Vec<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Read saved suggestions and their proposed edits.
    Recommendations,
    /// Save a baseline for a later comparison.
    Snapshot {
        name: String,
        #[command(flatten)]
        filters: Filters,
    },
    /// Compare outcomes before and after a workflow change.
    Compare { before: String, after: String },
    /// Export or import human judgments for calibration.
    Labels {
        #[command(subcommand)]
        action: LabelCommand,
    },
    /// Compare human labels with Jev judgments.
    Calibration,
    /// Remove retained data without touching original logs.
    Purge {
        #[arg(long)]
        older_than: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
}
#[derive(Subcommand, Debug)]
pub enum LabelCommand {
    Export {
        #[arg(long, default_value_t = 20)]
        sample: usize,
        #[arg(long)]
        output: PathBuf,
    },
    Import {
        path: PathBuf,
    },
}
#[derive(Clone, Serialize, Deserialize)]
struct Run {
    id: String,
    created_at: String,
    analysis_ids: Vec<String>,
    failures: Vec<String>,
    #[serde(default)]
    selection: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    id: String,
    created_at: String,
    filters: Filters,
    pairs: Vec<(Session, Analysis)>,
    report: Value,
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn identifier(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}
fn emit(format: Format, kind: &str, data: Value) -> Result<()> {
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({"schema_version":1,"kind":kind,"data":data}))?
        ),
        _ => {
            let heading = kind.replace('_', " ").to_uppercase();
            if matches!(format, Format::Markdown) {
                println!("# {heading}\n");
            } else {
                println!("{}", ui::heading(&heading));
            }
            if !compact(kind, &data, format) {
                render(&data, 0);
            }
        }
    }
    Ok(())
}
fn render(v: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                if v.is_object() || v.is_array() {
                    println!("{pad}{}:", k.replace('_', " "));
                    render(v, depth + 1);
                } else {
                    if k.ends_with("_ms") && v.is_number() {
                        println!(
                            "{pad}{}: {}",
                            k.trim_end_matches("_ms").replace('_', " "),
                            ui::duration(v.as_f64().unwrap_or(0.0))
                        );
                    } else {
                        println!("{pad}{}: {}", k.replace('_', " "), ui::value(v));
                    }
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                render(x, depth);
            }
        }
        _ => println!("{pad}{}", ui::value(v)),
    }
}
fn pairs(w: &Workspace, f: &Filters) -> Result<Vec<(Session, Analysis)>> {
    Ok(pairs_and_unscored(w, f)?.0)
}
type Corpus = Vec<(Session, Analysis)>;
fn pairs_and_unscored(w: &Workspace, f: &Filters) -> Result<(Corpus, usize)> {
    if let Some(since) = &f.since {
        analytics::cutoff(since).context("arguments: invalid --since")?;
    }
    let mut sessions = w.sessions()?;
    let mut analyses = w.analyses()?;
    let analyzed: BTreeSet<_> = analyses
        .iter()
        .map(|a| (&a.session_id, &a.revision))
        .collect();
    let unscored = sessions
        .iter()
        .filter(|s| !analyzed.contains(&(&s.id, &s.revision)))
        .count();
    if let Some(run) = &f.run {
        let r: Run = w.load_json("runs", run)?;
        analyses.retain(|a| r.analysis_ids.contains(&a.id));
        sessions = analyses
            .iter()
            .map(|a| w.session(&a.session_id, Some(&a.revision)))
            .collect::<Result<_>>()?;
    }
    Ok((analytics::cohort(&sessions, &analyses, f)?, unscored))
}
fn import(w: &Workspace, path: &Path, c: &Config) -> Result<(Vec<Session>, Vec<String>)> {
    let (s, errors) = ingest::import_batch(path, c)?;
    for e in &errors {
        eprintln!("{e}");
    }
    for x in &s {
        w.save_session(x)?;
        for warning in &x.warnings {
            eprintln!("{}: {warning}", x.id);
        }
    }
    Ok((s, errors))
}
pub async fn execute(cli: Cli) -> Result<i32> {
    let quiet = matches!(&cli.command, Command::Report { .. });
    ui::quiet(quiet, ui::track(execute_inner(cli))).await
}
async fn execute_inner(cli: Cli) -> Result<i32> {
    let format = cli.format;
    if let Command::Init {
        no_input,
        jev_endpoint,
        jev_api_key_env,
        jev_model,
        mut review_provider,
        review_endpoint,
        review_model,
        review_api_key_env,
        retention_days,
        min_pattern_sessions,
        redact_pattern,
        no_redaction,
    } = cli.command
    {
        if min_pattern_sessions == Some(0) {
            bail!("configuration: min-pattern-sessions must be positive");
        }
        let w = Workspace::open_or_init(cli.workspace.as_deref(), &std::env::current_dir()?)?;
        let mut c = w.config()?;
        let _ = dotenvy::from_path(w.root.join(".env"));
        if !no_input
            && matches!(format, Format::Text)
            && ui::interactive()
            && review_provider.is_none()
        {
            eprintln!(
                "Set up this project. Press Enter to keep the current choice.\nData: {}",
                w.data_dir().display()
            );
            review_provider = Some(ui::choose_provider(&c.review.provider)?);
        }
        if let Some(v) = retention_days {
            c.retention_days = v;
        }
        if let Some(v) = min_pattern_sessions {
            c.min_pattern_sessions = v;
        }
        if let Some(v) = jev_endpoint {
            c.jev.endpoint = v;
        }
        if let Some(v) = jev_api_key_env {
            c.jev.api_key_env = v;
        }
        if let Some(v) = jev_model {
            c.jev.model = v;
        }
        if let Some(v) = review_provider {
            if v == "openai" && c.review.provider != "openai" {
                c.review = config::ReviewConfig::default();
            }
            if v == "openrouter" && c.review.provider != "openrouter" {
                c.review.endpoint = "https://openrouter.ai/api/v1/chat/completions".into();
                c.review.api_key_env = "OPENROUTER_API_KEY".into();
                c.review.model = "openai/gpt-4.1".into();
            } else if v != "openai" && v != "none" && v != "openrouter" {
                bail!("configuration: unsupported review provider");
            }
            c.review.provider = v;
        }
        if let Some(v) = review_endpoint {
            c.review.endpoint = v;
        }
        if let Some(v) = review_model {
            c.review.model = v;
        }
        if let Some(v) = review_api_key_env {
            c.review.api_key_env = v;
        }
        if no_redaction {
            c.redaction.enabled = false;
        }
        if !redact_pattern.is_empty() {
            c.redaction.patterns = redact_pattern;
        }
        crate::redact::Redactor::new(&c.redaction).context("configuration: redaction")?;
        w.save_config(&c)?;
        if !no_input && matches!(format, Format::Text) && ui::interactive() {
            eprintln!(
                "Keys saved here work across projects: {}",
                crate::credentials::path()?.display()
            );
            eprintln!("Shell and project .env values override saved keys.");
            crate::credentials::prompt(&c.jev.api_key_env, "Jev")?;
            if c.review.provider != "none" && c.review.api_key_env != c.jev.api_key_env {
                crate::credentials::prompt(&c.review.api_key_env, &c.review.provider)?;
            }
        }
        emit(
            format,
            "workspace",
            json!({"root":w.root,"data_dir":w.data_dir(),"configuration":c,
            "credentials_file":crate::credentials::path().ok(),
            "jev_key_ready":crate::credentials::resolve(&c.jev.api_key_env)?.is_some(),
            "review_key_ready":crate::credentials::resolve(&c.review.api_key_env)?.is_some()}),
        )?;
        return Ok(0);
    }
    if let Command::Discover { discovery: args } = &cli.command {
        let options = discovery_options(args)?;
        let result = discovery::discover(&options)?;
        let config = match &cli.workspace {
            Some(path) => Workspace::discover(Some(path))?.config()?,
            None => Workspace::for_project(&result.project_root)
                .and_then(|w| w.config())
                .unwrap_or_default(),
        };
        let selection = discovery_value(&result, &config)?;
        emit(format, "discovery", selection)?;
        return Ok(i32::from(!result.errors.is_empty()));
    }
    if let Command::Analyze {
        path,
        all,
        discovery: args,
        ..
    } = &cli.command
    {
        if (path.is_some() || *all)
            && (args.project.is_some()
                || args.codex_home.is_some()
                || args.claude_config_dir.is_some())
        {
            bail!("arguments: --project, --codex-home and --claude-config-dir require no-path analyze");
        }
    }
    let project = if let Command::Analyze {
        discovery: args, ..
    } = &cli.command
    {
        args.project.clone().unwrap_or(std::env::current_dir()?)
    } else {
        std::env::current_dir()?
    };
    let w = if matches!(
        &cli.command,
        Command::Analyze { .. } | Command::Import { .. }
    ) {
        Workspace::open_or_init(cli.workspace.as_deref(), &project)?
    } else {
        Workspace::discover(cli.workspace.as_deref())?
    };
    let _ = dotenvy::from_path(w.root.join(".env"));
    let c = w.config().context("configuration")?;
    match cli.command {
        Command::Init { .. } | Command::Discover { .. } => unreachable!(),
        Command::Import { path } => {
            let (sessions, failures) = import(&w, &path, &c)?;
            emit(
                format,
                "import",
                json!({"sessions":sessions,"failures":failures}),
            )?;
            return Ok(i32::from(!failures.is_empty()));
        }
        Command::Analyze {
            discovery: args,
            path,
            all,
            refresh,
            dry_run,
        } => {
            eprintln!("Finding and importing sessions…");
            let started = std::time::Instant::now();
            let mut selection =
                json!({"mode":if all{"imported"}else{"explicit_path"},"agent":args.agent});
            let (mut sessions, import_failures) = if let Some(p) = path {
                import(&w, &p, &c)?
            } else if all {
                (w.sessions()?, vec![])
            } else {
                let found = discovery::discover(&discovery_options(&args)?)?;
                selection = discovery_value(&found, &c)?;
                let mut collected = vec![];
                let mut failures = found
                    .errors
                    .iter()
                    .map(|e| crate::redact::Redactor::new(&c.redaction).map(|r| r.text(e)))
                    .collect::<Result<Vec<_>>>()?;
                for candidate in &found.sessions {
                    match ingest::parse_file(&candidate.path, &c) {
                        Ok(mut session) => {
                            let redactor = crate::redact::Redactor::new(&c.redaction)?;
                            let expected = redactor.text(&found.project_root.to_string_lossy());
                            if session.agent != candidate.agent
                                || !(session.project_root.as_deref() == Some(expected.as_str())
                                    || session.repository.as_deref().is_some_and(|cwd| {
                                        discovery::matches_project(
                                            Path::new(cwd),
                                            &found.project_root,
                                        )
                                    }))
                            {
                                let message=redactor.text(&format!("{}: agent/project metadata changed or does not match selected project; skipped",candidate.path.display()));
                                eprintln!("{message}");
                                failures.push(message);
                                continue;
                            }
                            if session.project_root.as_deref() != Some(expected.as_str()) {
                                session.revision = config::hash(&serde_json::to_vec(&(
                                    "discovered_project_root",
                                    &session.revision,
                                    &expected,
                                ))?);
                                session.project_root = Some(expected);
                            }
                            w.save_session(&session)?;
                            collected.push(session);
                        }
                        Err(e) => {
                            let message = crate::redact::Redactor::new(&c.redaction)?
                                .text(&format!("{}: {e}", candidate.path.display()));
                            eprintln!("{message}");
                            failures.push(message);
                        }
                    }
                }
                (collected, failures)
            };
            sessions.retain(|s| agent_matches(&args.agent, &s.agent));
            sessions.sort_by(|a, b| a.id.cmp(&b.id));
            sessions.dedup_by(|a, b| a.id == b.id && a.revision == b.revision);
            selection["selected_sessions"] = json!(sessions.len());
            if dry_run {
                let payloads = sessions
                    .iter()
                    .map(|s| {
                        Ok(json!({"session_id":s.id,"requests":services::prepare_analysis(s,&c)?}))
                    })
                    .collect::<Result<Vec<_>>>()?;
                emit(
                    format,
                    "analysis_preview",
                    json!({"destination":c.jev.endpoint,"payloads":payloads,"failures":import_failures,"selection":selection}),
                )?;
                return Ok(i32::from(!import_failures.is_empty()));
            }
            let existing = w.analyses()?;
            let fingerprint = config::analysis_fingerprint(&c);
            let cached_for = |s: &Session| {
                existing
                    .iter()
                    .filter(|a| {
                        !refresh
                            && a.session_id == s.id
                            && a.revision == s.revision
                            && a.config_fingerprint == fingerprint
                            && a.rubric_version == crate::model::RUBRIC_VERSION
                    })
                    .max_by_key(|a| &a.created_at)
            };
            let cached_count = sessions.iter().filter(|s| cached_for(s).is_some()).count();
            eprintln!(
                "Preparing request batches for {} sessions ({} cached)…",
                sessions.len(),
                cached_count
            );
            let planned = sessions
                .iter()
                .filter(|s| cached_for(s).is_none())
                .filter_map(|s| services::prepare_analysis(s, &c).ok().map(|r| r.len()))
                .sum::<usize>();
            eprintln!(
                "{} sessions · {} cached · {} Jev requests planned, before retries",
                sessions.len(),
                cached_count,
                planned
            );
            selection["planned_requests"] = json!(planned);
            selection["cached_sessions"] = json!(cached_count);
            let total_sessions = sessions.len();
            let mut run = Run {
                id: identifier("run"),
                created_at: now(),
                analysis_ids: vec![],
                failures: import_failures,
                selection,
            };
            let progress = ui::AnalysisProgress::new(total_sessions);
            for (index, s) in sessions.into_iter().enumerate() {
                let mut row = progress.session(index, &s);
                let cached = cached_for(&s);
                if let Some(a) = cached.filter(|_| !refresh) {
                    run.analysis_ids.push(a.id.clone());
                    row.finish("cached");
                } else {
                    match row.run(services::score_session(&s, &c)).await {
                        Ok(a) => {
                            row.saving();
                            w.save_analysis(&a)?;
                            run.analysis_ids.push(a.id);
                            row.finish("scored");
                        }
                        Err(e) => {
                            row.finish("failed");
                            progress.error(&format!("  {}: {e}", s.id));
                            run.failures.push(s.id.clone());
                        }
                    }
                }
                run.selection["api_calls"] = json!(ui::requests());
                w.save_json("runs", &run.id, &run)?;
            }
            run.selection["api_calls"] = json!(ui::requests());
            run.selection["elapsed_seconds"] = json!(started.elapsed().as_secs());
            w.save_json("runs", &run.id, &run)?;
            progress.finish();
            let mut output = serde_json::to_value(&run)?;
            let filters = Filters {
                run: Some(run.id.clone()),
                ..Filters::default()
            };
            output["report"] = analytics::report(&pairs(&w, &filters)?, &filters)?;
            emit(format, "analysis_run", output)?;
            return Ok(i32::from(!run.failures.is_empty()));
        }
        Command::Show {
            id,
            turn,
            evidence,
            run,
        } => {
            let data = if id.starts_with("s_") {
                let p = pairs(
                    &w,
                    &Filters {
                        run: run.clone(),
                        ..Filters::default()
                    },
                )?;
                let pair = p.iter().find(|(s, _)| s.id == id);
                if run.is_some() && pair.is_none() {
                    bail!("session is not part of the selected run");
                }
                let session = match pair {
                    Some((s, _)) => s.clone(),
                    None => w.session(&id, None)?,
                };
                let analysis = pair.map(|(_, a)| a);
                if let Some(t) = turn {
                    let t = session
                        .turns
                        .iter()
                        .find(|x| x.id == t)
                        .context("turn not found")?;
                    let events = t
                        .event_indices
                        .iter()
                        .filter_map(|i| session.events.get(*i))
                        .collect::<Vec<_>>();
                    json!({"session_id":id,"turn":t,"events":events,"judgment":analysis.and_then(|a|a.turns.get(&t.id)),"outcome":analysis.map(|a|&a.session)})
                } else {
                    json!({"session":session,"analysis":analysis,"prepared_evidence":if evidence{Some(services::prepare_analysis(&session,&c)?)}else{None}})
                }
            } else if id.starts_with("p_") {
                let p = analytics::patterns(&pairs(
                    &w,
                    &Filters {
                        run,
                        ..Filters::default()
                    },
                )?);
                serde_json::to_value(p.iter().find(|p| p.id == id).context("pattern not found")?)?
            } else {
                let collection = if id.starts_with("run_") {
                    "runs"
                } else if id.starts_with("r_") {
                    "recommendations"
                } else {
                    "analyses"
                };
                w.load_json::<Value>(collection, &id)?
            };
            emit(format, "show", data)?;
        }
        Command::Report {
            filters,
            top,
            all,
            recurring_only,
            session,
            context,
            dry_run,
        } => {
            let (mut p, unscored) = pairs_and_unscored(&w, &filters)?;
            if let Some(id) = &session {
                p.retain(|(s, _)| &s.id == id);
                if p.is_empty() {
                    bail!("no analyzed session found");
                }
            }
            // Compute the full filtered corpus before sampling recommendation evidence.
            let mut statistics = analytics::report(&p, &filters)?;
            statistics["unscored_current_sessions"] = json!(unscored);
            let sources = p
                .iter()
                .map(|(s, _)| json!({"session_id":s.id,"revision":s.revision}))
                .collect::<Vec<_>>();
            let candidates =
                serde_json::from_value::<Vec<analytics::Pattern>>(statistics["patterns"].clone())?;
            let recurrence_minimum = c.min_pattern_sessions.max(1);
            let preliminary = session.is_none()
                && !recurring_only
                && !candidates.is_empty()
                && candidates.iter().all(|x| x.sessions < recurrence_minimum);
            let minimum_sessions = if session.is_some() || preliminary {
                1
            } else {
                recurrence_minimum
            };
            let eligible = candidates
                .into_iter()
                .filter(|x| x.sessions >= minimum_sessions)
                .collect::<Vec<_>>();
            let eligible_count = eligible.len();
            let limit = if all || session.is_some() {
                usize::MAX
            } else {
                top.unwrap_or(5) as usize
            };
            let patterns = eligible.into_iter().take(limit).collect::<Vec<_>>();
            if session.is_none() {
                let mut sampled = BTreeSet::new();
                for pattern in &patterns {
                    let support = pattern
                        .supporting
                        .iter()
                        .filter_map(|r| r["session_id"].as_str())
                        .collect::<BTreeSet<_>>();
                    let mut candidates = p
                        .iter()
                        .filter(|(s, _)| support.contains(s.id.as_str()))
                        .collect::<Vec<_>>();
                    candidates.sort_by_key(|(s, a)| {
                        let outcome = a.session.get("task_outcome").map(|d| d.selected.as_str());
                        let verification = a
                            .session
                            .get("outcome_verification")
                            .map(|d| d.selected.as_str());
                        (
                            if matches!(outcome, Some("failed" | "partially_complete")) {
                                0
                            } else if verification == Some("claimed_but_unverified") {
                                1
                            } else {
                                2
                            },
                            s.id.clone(),
                        )
                    });
                    let limit = c.min_pattern_sessions.max(3);
                    let mut chosen = candidates
                        .iter()
                        .take(limit)
                        .map(|(s, _)| s.id.clone())
                        .collect::<Vec<_>>();
                    if let Some((s, _)) = candidates.iter().find(|(_, a)| {
                        a.session
                            .get("task_outcome")
                            .is_some_and(|d| d.selected == "complete")
                    }) {
                        if !chosen.contains(&s.id) && chosen.len() == limit {
                            chosen.pop();
                            chosen.push(s.id.clone());
                        }
                    }
                    sampled.extend(chosen);
                }
                p.retain(|(s, _)| sampled.contains(&s.id));
            }
            let selection = json!({"eligible_patterns":eligible_count,"selected_patterns":patterns.len(),
                "sampled_sessions":p.len(),"minimum_sessions":minimum_sessions,
                "preliminary":preliminary,"recurrence_minimum_sessions":recurrence_minimum,
                "isolated":session.is_some(),"limit":if all || session.is_some(){None}else{Some(limit)}});
            let mut evidence = review_evidence(&p, &patterns, session.is_some());
            evidence["preliminary"] = json!(preliminary);
            evidence["project_context"] =
                crate::review_context::collect(&w.root, &p, &context, &c)?;
            crate::redact::Redactor::new(&c.redaction)?.value(&mut evidence);
            if dry_run {
                emit(
                    format,
                    "review_preview",
                    json!({"destination":c.review.endpoint,"provider":c.review.provider,"selection":selection,"request":services::prepare_review(&evidence,&c)?}),
                )?;
            } else if p.is_empty() {
                finish_review(
                    &w,
                    format,
                    json!({"recommendations":[],"selection":selection,"api_calls":0,
                    "message":if statistics["sessions"].as_u64() == Some(0) {
                        "No analyzed sessions match this report. Run jta analyze or adjust the report filters. No API request sent."
                    } else if recurring_only {
                        "No patterns meet the recurrence threshold. Run jta report without --recurring-only for a preliminary review, or analyze more sessions. No API request sent."
                    } else {
                        "No opportunity patterns match this report. Adjust the report filters or use jta report --session <id> for one analyzed session. No API request sent."
                    }}),
                    &statistics,
                    &sources,
                    &filters,
                )?;
            } else if !evidence["project_context"]["projects"]
                .as_array()
                .is_some_and(|projects| {
                    projects
                        .iter()
                        .any(|p| p["files"].as_array().is_some_and(|files| !files.is_empty()))
                })
            {
                finish_review(
                    &w,
                    format,
                    json!({"recommendations":[],"selection":selection,"api_calls":0,
                    "message":"No readable project files available to ground recommendations. Restore the selected project or supply --context <file>. No LLM request sent."}),
                    &statistics,
                    &sources,
                    &filters,
                )?;
            } else if c.review.provider == "none" {
                finish_review(
                    &w,
                    format,
                    json!({"recommendations":[],"selection":selection,"api_calls":0,
                    "message":"Recommendations are unavailable because no review provider is configured. Run jta init to choose OpenAI or OpenRouter, then generate the report again. No API request sent."}),
                    &statistics,
                    &sources,
                    &filters,
                )?;
            } else {
                if preliminary && !matches!(format, Format::Json) {
                    eprintln!("{}", ui::review_scope(&selection));
                }
                let result = services::review(&evidence, &c).await.with_context(|| {
                    format!(
                        "Review failed using configuration {}",
                        w.data_dir().join("config.json").display()
                    )
                })?;
                let mut saved = vec![];
                let mut skipped = vec![];
                let mut warnings = vec![];
                for (index, proposal) in result["recommendations"]
                    .as_array()
                    .context("review response missing recommendations")?
                    .iter()
                    .enumerate()
                {
                    let mut v = proposal.clone();
                    let refs = v["supporting_refs"]
                        .as_array()
                        .context("missing supporting_refs")?;
                    let mut support = BTreeSet::new();
                    for r in refs {
                        let sid = r["session_id"]
                            .as_str()
                            .context("invalid session reference")?;
                        let tid = r["turn_id"].as_u64().context("invalid turn reference")?;
                        if !evidence["sessions"].as_array().is_some_and(|sessions| {
                            sessions.iter().any(|s| {
                                s["session_id"] == sid
                                    && s["turns"].as_array().is_some_and(|turns| {
                                        turns.iter().any(|t| t["turn_id"] == tid)
                                    })
                            })
                        }) {
                            bail!("review contains unsupported evidence reference");
                        }
                        support.insert(sid.to_owned());
                    }
                    if support.len() < minimum_sessions {
                        skipped.push(json!({"index":index + 1,"supporting_sessions":support.len(),"required_sessions":minimum_sessions,"reason":"insufficient_session_support"}));
                        warnings.push(format!(
                            "Skipped recommendation {}: cites {} distinct sessions; at least {} required",
                            index + 1, support.len(), minimum_sessions
                        ));
                        continue;
                    }
                    let id = identifier("r");
                    v["id"] = json!(id);
                    v["isolated"] = json!(session.is_some());
                    v["preliminary"] = json!(preliminary);
                    v["supporting_sessions"] = json!(support.len());
                    v["grounding_version"] = json!(1);
                    v["created_at"] = json!(now());
                    v["source_evidence"] = evidence.clone();
                    v["agents"] = json!(p
                        .iter()
                        .map(|(s, _)| s.agent.clone())
                        .collect::<BTreeSet<_>>());
                    v["project_roots"] = json!(p
                        .iter()
                        .filter_map(|(s, _)| s.project_root.clone())
                        .collect::<BTreeSet<_>>());
                    crate::redact::Redactor::new(&c.redaction)?.value(&mut v);
                    saved.push(v);
                }
                for v in &saved {
                    w.save_json(
                        "recommendations",
                        v["id"].as_str().context("missing recommendation id")?,
                        v,
                    )?;
                }
                let mut output = json!({"recommendations":saved,"skipped_recommendations":skipped,"warnings":warnings,"selection":selection,"api_calls":ui::requests()});
                if saved.is_empty() && !skipped.is_empty() {
                    output["message"] = json!("No recommendations met the recurrence threshold. Use jta report --session <session-id> to review an isolated finding.");
                } else if saved.is_empty() {
                    output["message"] = json!("No project-specific edits were supported by the selected sessions and current project files.");
                }
                // An empty model result can still explain why no edit was justified.
                // Suppress prose only when local filtering rejected proposals it may describe.
                if skipped.is_empty() {
                    output["summary"] = result["summary"].clone();
                    output["themes"] = result["themes"].clone();
                }
                crate::redact::Redactor::new(&c.redaction)?.value(&mut output);
                finish_review(&w, format, output, &statistics, &sources, &filters)?;
            }
        }
        Command::Recommendations => emit(
            format,
            "recommendations",
            json!(w.list_json::<Value>("recommendations")?),
        )?,
        Command::Snapshot { name, filters } => {
            let p = pairs(&w, &filters)?;
            let snapshot = Snapshot {
                id: name.clone(),
                created_at: now(),
                report: analytics::report(&p, &filters)?,
                filters,
                pairs: p,
            };
            if w.load_json::<Value>("snapshots", &name).is_ok() {
                bail!("snapshot already exists; choose a new name");
            }
            w.save_json("snapshots", &name, &snapshot)?;
            emit(format, "snapshot", serde_json::to_value(snapshot)?)?;
        }
        Command::Compare { before, after } => {
            let b: Snapshot = w.load_json("snapshots", &before)?;
            let a: Snapshot = w.load_json("snapshots", &after)?;
            let mut warnings = vec!["Observed differences do not establish causation".to_owned()];
            if b.pairs
                .iter()
                .any(|(s, _)| a.pairs.iter().any(|(t, _)| s.id == t.id))
            {
                warnings.push("Overlapping session cohorts".into());
            }
            if b.report["configuration_fingerprints"] != a.report["configuration_fingerprints"] {
                warnings.push("Different scoring configurations".into());
            }
            if b.report["resolved_jev_models"] != a.report["resolved_jev_models"] {
                warnings.push("Resolved Jev models differ; model alias may have changed".into());
            }
            let mix = |s: &Snapshot| {
                let mut counts: BTreeMap<(String, Option<String>), f64> = BTreeMap::new();
                for (session, _) in &s.pairs {
                    *counts
                        .entry((session.agent.clone(), session.repository.clone()))
                        .or_default() += 1.0;
                }
                for n in counts.values_mut() {
                    *n /= s.pairs.len() as f64;
                }
                counts
            };
            if mix(&b) != mix(&a) {
                warnings
                    .push("Agent/repository mix differs; task difficulty is not controlled".into());
            }
            emit(
                format,
                "comparison",
                json!({"metrics":comparison_metrics(&b.report,&a.report),"before":b.report,"after":a.report,"warnings":warnings,"uncertainty_note":"Wilson 95% intervals describe each observed categorical proportion; these are not intervals for causal effects or for the difference. Denominators count included judgments for each question."}),
            )?;
        }
        Command::Labels { action } => labels(&w, format, action)?,
        Command::Calibration => calibration(&w, format)?,
        Command::Purge {
            older_than,
            dry_run,
        } => purge(
            &w,
            format,
            &older_than.unwrap_or_else(|| format!("{}d", c.retention_days)),
            dry_run,
        )?,
    }
    Ok(0)
}
fn labels(w: &Workspace, format: Format, action: LabelCommand) -> Result<()> {
    match action {
        LabelCommand::Export { sample, output } => {
            let p = pairs(w, &Filters::default())?;
            let mut rows = vec![];
            for (s, a) in p {
                rows.push(json!({"schema_version":1,"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"turn_id":null,"human_labels":{},"model":a.session,"context":s}));
                for t in &s.turns {
                    rows.push(json!({"schema_version":1,"session_id":s.id,"revision":s.revision,"analysis_id":a.id,"turn_id":t.id,"human_labels":{},"model":a.turns.get(&t.id).map(|j|&j.answers),"context":s}));
                }
            }
            rows.sort_by_key(|r| config::hash(r.to_string().as_bytes()));
            rows.truncate(sample);
            let mut bytes = String::new();
            for r in &rows {
                bytes.push_str(&serde_json::to_string(r)?);
                bytes.push('\n');
            }
            fs::write(&output, bytes)?;
            emit(
                format,
                "labels_export",
                json!({"output":output,"records":rows.len()}),
            )
        }
        LabelCommand::Import { path } => {
            let text = fs::read_to_string(path)?;
            let mut validated = vec![];
            for (line, s) in text.lines().enumerate() {
                if s.trim().is_empty() {
                    continue;
                }
                let v: Value =
                    serde_json::from_str(s).with_context(|| format!("labels line {}", line + 1))?;
                let sid = v["session_id"].as_str().context("missing session_id")?;
                let rev = v["revision"].as_str().context("missing revision")?;
                let session = w.session(sid, Some(rev))?;
                let analysis: Analysis = w.load_json(
                    "analyses",
                    v["analysis_id"].as_str().context("missing analysis_id")?,
                )?;
                if analysis.session_id != sid || analysis.revision != rev {
                    bail!("label source mismatch")
                };
                if !v["turn_id"].is_null()
                    && !v["turn_id"]
                        .as_u64()
                        .is_some_and(|t| t > 0 && t <= u64::from(u32::MAX))
                {
                    bail!("invalid label turn_id: expected null or positive u32");
                }
                let model = if let Some(t) = v["turn_id"].as_u64() {
                    if !session.turns.iter().any(|x| u64::from(x.id) == t) {
                        bail!("invalid label turn")
                    };
                    serde_json::to_value(
                        &analysis
                            .turns
                            .get(&(t as u32))
                            .context("unscored turn")?
                            .answers,
                    )?
                } else {
                    serde_json::to_value(&analysis.session)?
                };
                let labels = v["human_labels"]
                    .as_object()
                    .context("human_labels must be an object")?;
                if labels.is_empty() {
                    continue;
                }
                for (q, label) in labels {
                    let label = label
                        .as_str()
                        .context("human label must be categorical text")?;
                    if model[q]["probabilities"].get(label).is_none() {
                        bail!("invalid human label for {q}")
                    }
                }
                let id = format!(
                    "label_{}",
                    &config::hash(
                        format!("{sid}:{rev}:{}:{}", analysis.id, v["turn_id"]).as_bytes()
                    )[..20]
                );
                let stored = json!({"schema_version":1,"id":id,"session_id":sid,"revision":rev,"analysis_id":analysis.id,"turn_id":v["turn_id"],"human_labels":labels,"model":model,"context":session});
                validated.push((id, stored));
            }
            for (id, v) in &validated {
                w.save_json("labels", id, v)?;
            }
            emit(format, "labels_import", json!({"records":validated.len()}))
        }
    }
}
fn calibration(w: &Workspace, format: Format) -> Result<()> {
    let labels = w.list_json::<Value>("labels")?;
    let mut questions: BTreeMap<String, Value> = BTreeMap::new();
    let mut disagreements = vec![];
    for l in labels {
        for (q, human) in l["human_labels"]
            .as_object()
            .context("invalid stored labels")?
        {
            let m = &l["model"][q];
            let entry = questions
                .entry(q.clone())
                .or_insert(json!({"count":0,"agree":0,"brier_sum":0.0,"bins":{}}));
            entry["count"] = json!(entry["count"].as_u64().unwrap_or(0) + 1);
            let agree = m["selected"] == *human;
            if agree {
                entry["agree"] = json!(entry["agree"].as_u64().unwrap_or(0) + 1);
            } else {
                disagreements.push(json!({"session_id":l["session_id"],"turn_id":l["turn_id"],"question":q,"human":human,"model":m}));
            }
            let mut brier = 0.0;
            if let Some(probs) = m["probabilities"].as_object() {
                for (k, p) in probs {
                    let p = p.as_f64().unwrap_or(0.0);
                    brier += (p - if Some(k.as_str()) == human.as_str() {
                        1.0
                    } else {
                        0.0
                    })
                    .powi(2);
                }
            }
            entry["brier_sum"] = json!(entry["brier_sum"].as_f64().unwrap_or(0.0) + brier);
            let conf = m["probabilities"][m["selected"].as_str().unwrap_or("")]
                .as_f64()
                .unwrap_or(0.0);
            let bin = ((conf * 10.0).floor() as usize).min(9).to_string();
            if entry["bins"].get(&bin).is_none() {
                entry["bins"][&bin] = json!({"count":0,"correct":0,"confidence_sum":0.0});
            }
            let b = &mut entry["bins"][&bin];
            b["count"] = json!(b["count"].as_u64().unwrap_or(0) + 1);
            b["correct"] = json!(b["correct"].as_u64().unwrap_or(0) + u64::from(agree));
            b["confidence_sum"] = json!(b["confidence_sum"].as_f64().unwrap_or(0.0) + conf);
        }
    }
    for q in questions.values_mut() {
        let n = q["count"].as_f64().unwrap_or(1.0);
        q["agreement"] = json!(q["agree"].as_f64().unwrap_or(0.0) / n);
        q["brier_score"] = json!(q["brier_sum"].as_f64().unwrap_or(0.0) / n);
    }
    emit(
        format,
        "calibration",
        json!({"questions":questions,"disagreements":disagreements,"note":"Human labels remain separate from model judgments"}),
    )
}
fn purge(w: &Workspace, format: Format, age: &str, dry_run: bool) -> Result<()> {
    let cutoff = analytics::cutoff(age).context("arguments: invalid --older-than")?;
    let dir = w.data_dir();
    let revisions = w.list_json::<Session>("revisions")?;
    let expired = revisions
        .iter()
        .filter(|s| {
            chrono::DateTime::parse_from_rfc3339(&s.imported_at)
                .is_ok_and(|t| t.with_timezone(&chrono::Utc) < cutoff)
        })
        .map(|s| (s.id.clone(), s.revision.clone()))
        .collect::<BTreeSet<_>>();
    let mut artifact_ids = BTreeSet::new();
    let mut records = vec![];
    for entry in walkdir::WalkDir::new(&dir).follow_links(false) {
        let e = entry?;
        if !e.file_type().is_file()
            || e.path().extension().and_then(|x| x.to_str()) != Some("json")
            || e.file_name() == "config.json"
        {
            continue;
        }
        let v: Value = serde_json::from_slice(&fs::read(e.path())?)?;
        records.push((e.path().to_owned(), v));
    }
    loop {
        let before = artifact_ids.len();
        for (_, v) in &records {
            if retained_expired(v, &expired, &artifact_ids) {
                if let Some(id) = v["id"].as_str().filter(|id| !id.starts_with("s_")) {
                    artifact_ids.insert(id.to_owned());
                }
            }
        }
        if artifact_ids.len() == before {
            break;
        }
    }
    let mut objects = records
        .iter()
        .filter(|(_, v)| retained_expired(v, &expired, &artifact_ids))
        .map(|(p, _)| p.clone())
        .collect::<Vec<_>>();
    let markdown = objects
        .iter()
        .filter(|p| p.parent() == Some(dir.join("reports").as_path()))
        .map(|p| p.with_extension("md"))
        .filter(|p| p.exists())
        .collect::<Vec<_>>();
    objects.extend(markdown);
    if !dry_run {
        for p in &objects {
            fs::remove_file(p)?;
        }
    }
    emit(
        format,
        "purge",
        json!({"dry_run":dry_run,"expired_revisions":expired,"affected_objects":objects,"original_logs_deleted":false}),
    )
}
fn retained_expired(
    v: &Value,
    expired: &BTreeSet<(String, String)>,
    ids: &BTreeSet<String>,
) -> bool {
    match v {
        Value::String(s) => ids.contains(s),
        Value::Array(a) => a.iter().any(|v| retained_expired(v, expired, ids)),
        Value::Object(o) => {
            let sid = o
                .get("session_id")
                .or_else(|| o.get("id"))
                .and_then(Value::as_str);
            let rev = o.get("revision").and_then(Value::as_str);
            if let (Some(s), Some(r)) = (sid, rev) {
                if expired.contains(&(s.to_owned(), r.to_owned())) {
                    return true;
                }
            }
            o.values().any(|v| retained_expired(v, expired, ids))
        }
        _ => false,
    }
}
fn finish_review(
    w: &Workspace,
    format: Format,
    mut output: Value,
    statistics: &Value,
    sources: &[Value],
    filters: &Filters,
) -> Result<()> {
    let id = identifier("report");
    output["id"] = json!(id);
    output["created_at"] = json!(now());
    output["statistics"] = statistics.clone();
    output["filters"] = serde_json::to_value(filters)?;
    output["sources"] = json!(sources);
    let report = crate::report::markdown(&output);
    let path = w.save_report(&id, &report)?;
    output["report_path"] = json!(path);
    let data_path = w.data_dir().join("reports").join(format!("{id}.json"));
    // The exact statistics and all source revisions stay available for scripts and retention.
    w.save_json("reports", &id, &output)?;
    if matches!(format, Format::Json) {
        emit(
            format,
            "report_generated",
            json!({"report_path":path,"data_path":data_path}),
        )
    } else {
        if let Some(message) = output["message"].as_str() {
            eprintln!("{message}");
        }
        println!("Generated Markdown report: {}", path.display());
        Ok(())
    }
}

fn compact(kind: &str, data: &Value, format: Format) -> bool {
    match kind {
        "workspace" => {
            println!(
                "Ready for {}",
                data["root"].as_str().unwrap_or("this project")
            );
            println!(
                "Data and config: {}",
                data["data_dir"].as_str().unwrap_or("?")
            );
            if let Some(path) = data["credentials_file"].as_str() {
                println!("Global credentials: {path}");
            }
            println!(
                "Jev: {} · {} {}",
                data["configuration"]["jev"]["model"]
                    .as_str()
                    .unwrap_or("?"),
                data["configuration"]["jev"]["api_key_env"]
                    .as_str()
                    .unwrap_or("JEV_API_KEY"),
                if data["jev_key_ready"] == true {
                    "is set"
                } else {
                    "is missing"
                }
            );
            println!(
                "Review provider: {}",
                data["configuration"]["review"]["provider"]
                    .as_str()
                    .unwrap_or("none")
            );
            if data["configuration"]["review"]["provider"] != "none"
                && data["review_key_ready"] != true
            {
                println!(
                    "Run jta init to save {}, or set it in your shell/project .env.",
                    data["configuration"]["review"]["api_key_env"]
                        .as_str()
                        .unwrap_or("?")
                );
            }
            println!("\nNext: jta analyze     (or jta analyze --dry-run to preview)");
            true
        }
        "analysis_preview" => {
            let sessions = data["payloads"].as_array().cloned().unwrap_or_default();
            let requests = sessions
                .iter()
                .map(|s| s["requests"].as_array().map_or(0, Vec::len))
                .sum::<usize>();
            println!(
                "{} sessions · {} Jev request batches for a fresh analysis",
                sessions.len(),
                requests
            );
            println!(
                "Destination: {}",
                data["destination"].as_str().unwrap_or("?")
            );
            println!("No requests sent. Cached results can reduce calls on a real run.");
            println!("Use --format json to inspect the full redacted payload.");
            render(&data["failures"], 0);
            true
        }
        "review_preview" => {
            println!("{}", ui::review_scope(&data["selection"]));
            println!(
                "Destination: {} · {}",
                data["provider"].as_str().unwrap_or("none"),
                data["destination"].as_str().unwrap_or("?")
            );
            println!("No requests sent. Use --format json to inspect the full redacted payload.");
            true
        }
        "discovery" => {
            println!(
                "Project: {}",
                data["project_root"].as_str().unwrap_or("unknown")
            );
            println!(
                "{} selected · {} scanned · {} skipped · {} malformed",
                data["selected_sessions"],
                data["scanned_files"],
                data["skipped_files"],
                data["malformed_files"]
            );
            println!(
                "Agents: {} Codex · {} Claude Code",
                data["agent_counts"]["codex"], data["agent_counts"]["claude_code"]
            );
            if data["selected_sessions"] == 0 {
                println!("No matching sessions. Check project metadata or supply explicit source-root overrides. No scoring request sent.");
            }
            if let Some(sessions) = data["sessions"].as_array() {
                for session in sessions {
                    println!(
                        "  {}  {}",
                        session["agent"].as_str().unwrap_or("unknown"),
                        session["path"].as_str().unwrap_or("unknown")
                    );
                }
            }
            render(&data["warnings"], 0);
            render(&data["errors"], 0);
            true
        }
        "analysis_run" => {
            if let Some(project) = data["selection"]["project_root"].as_str() {
                println!("Project: {project}");
            }
            println!("Run: {}", data["id"].as_str().unwrap_or("unknown"));
            println!(
                "{} analyses · {} failures",
                data["analysis_ids"].as_array().map_or(0, Vec::len),
                data["failures"].as_array().map_or(0, Vec::len)
            );
            println!(
                "{} Jev API calls · {} cached sessions · {}s",
                ui::value(&data["selection"]["api_calls"]),
                ui::value(&data["selection"]["cached_sessions"]),
                data["selection"]["elapsed_seconds"]
            );
            if let Some(counts) = data["report"]["counts"].as_object() {
                println!("\nTask outcomes");
                ui::counts(&counts["task_outcome"], matches!(format, Format::Text));
                println!("\nVerification");
                ui::counts(
                    &counts["outcome_verification"],
                    matches!(format, Format::Text),
                );
            }
            println!("\nNext: jta report to generate statistics and suggested changes.");
            render(&data["failures"], 0);
            true
        }
        "comparison" => {
            println!("Completion and verification lead this comparison.\n");
            if matches!(format, Format::Markdown) {
                println!("| Metric | Before | After | Change (pp) |\n| --- | ---: | ---: | ---: |");
            }
            for (q, first) in [
                ("task_outcome", "complete"),
                ("outcome_verification", "verified"),
                ("user_intervention", "none"),
            ] {
                if let Some(metrics) = data["metrics"][q].as_object() {
                    let labels = std::iter::once(first)
                        .chain(metrics.keys().map(String::as_str).filter(|x| *x != first));
                    for label in labels {
                        let Some(m) = metrics.get(label) else {
                            continue;
                        };
                        let display = |v: &Value| match v["proportion"].as_f64() {
                            Some(p) => format!(
                                "{:.1}% ({}/{}) [95% {:.1}–{:.1}%]",
                                100.0 * p,
                                v["count"],
                                v["denominator"],
                                v["wilson95"]["lower"].as_f64().unwrap_or(0.0) * 100.0,
                                v["wilson95"]["upper"].as_f64().unwrap_or(0.0) * 100.0
                            ),
                            None => "unknown (0 included judgments)".into(),
                        };
                        let delta = m["delta_percentage_points"]
                            .as_f64()
                            .map(|d| format!("{d:+.1}"))
                            .unwrap_or_else(|| "unknown".into());
                        if matches!(format, Format::Markdown) {
                            println!(
                                "| {q}: {label} | {} | {} | {delta} |",
                                display(&m["before"]),
                                display(&m["after"])
                            );
                        } else {
                            println!(
                                "{q}: {label}\n  {} → {}  ({delta} pp)",
                                display(&m["before"]),
                                display(&m["after"])
                            );
                        }
                    }
                }
            }
            println!("\n{}", data["uncertainty_note"].as_str().unwrap_or(""));
            render(&data["warnings"], 0);
            true
        }
        "import" => {
            let s = data["sessions"].as_array().cloned().unwrap_or_default();
            println!("Imported {} sessions", s.len());
            for x in s {
                println!(
                    "{}  {}  {} turns",
                    x["id"].as_str().unwrap_or("?"),
                    x["agent"].as_str().unwrap_or("?"),
                    x["turns"].as_array().map_or(0, Vec::len)
                );
            }
            true
        }
        "recommendations" => {
            let output = json!({"recommendations":data});
            let report = ui::review_report(&output);
            let body = report.trim_start_matches("# Review report\n\n");
            if matches!(format, Format::Markdown) {
                print!("{body}");
            } else {
                for line in body.lines() {
                    if line.starts_with("##") {
                        println!("{}", ui::heading(line.trim_start_matches('#').trim()));
                    } else {
                        println!("{line}");
                    }
                }
            }
            true
        }
        "snapshot" => {
            println!(
                "Saved {}: {} sessions, {} turns",
                data["id"].as_str().unwrap_or("?"),
                data["report"]["sessions"],
                data["report"]["turns"]
            );
            true
        }
        _ => false,
    }
}
fn excerpt(value: &str, limit: usize) -> Value {
    let text = value.chars().take(limit).collect::<String>();
    json!({"text":text,"omitted_chars":value.chars().count().saturating_sub(limit)})
}
fn review_evidence(
    pairs: &[(Session, Analysis)],
    patterns: &[analytics::Pattern],
    isolated: bool,
) -> Value {
    let sessions=pairs.iter().map(|(s,a)|{
 let mut chosen=BTreeSet::new();
 for pattern in patterns{for r in &pattern.supporting{if r["session_id"]==s.id{if let Some(id)=r["turn_id"].as_u64(){chosen.insert(id as u32);}}}}
 if isolated&&chosen.is_empty(){chosen.extend(s.turns.iter().take(10).map(|t|t.id));}
 let selected=chosen.iter().take(10).copied().collect::<Vec<_>>();chosen=selected.iter().copied().collect();
 for id in &selected{if let Some(i)=s.turns.iter().position(|t|t.id==*id){for t in s.turns.iter().skip(i.saturating_sub(1)).take(3){chosen.insert(t.id);}for n in s.turns[i].candidate_downstream.iter().take(3){chosen.insert(*n);}}}
 if let Some(t)=s.turns.last(){chosen.insert(t.id);}
 for t in s.turns.iter().rev().filter(|t|!t.verification.is_empty()).take(3){chosen.insert(t.id);}
 let turns=s.turns.iter().filter(|t|chosen.contains(&t.id)).map(|t|json!({"turn_id":t.id,"intent":excerpt(&t.intent,600),"tools":t.tools,"candidate_downstream":t.candidate_downstream,"judgment":a.turns.get(&t.id),"events":t.event_indices.iter().filter_map(|i|s.events.get(*i)).take(5).map(|e|json!({"line":e.line,"kind":e.kind,"text":excerpt(&e.text,1000),"input":e.input.as_ref().map(|v|excerpt(&v.to_string(),1000)),"is_error":e.is_error})).collect::<Vec<_>>(),"omitted_events":t.event_indices.len().saturating_sub(5)})).collect::<Vec<_>>();
 json!({"session_id":s.id,"revision":s.revision,"agent":s.agent,"project_root":s.project_root,"outcome":a.session,"goals_and_corrections":s.events.iter().filter(|e|e.kind=="user").take(8).map(|e|json!({"line":e.line,"text":excerpt(&e.text,1000)})).collect::<Vec<_>>(),"omitted_turns":s.turns.len().saturating_sub(turns.len()),"turns":turns,"warnings":s.warnings})
 }).collect::<Vec<_>>();
    let corpus_ids = patterns
        .iter()
        .flat_map(|p| p.supporting.iter().filter_map(|r| r["session_id"].as_str()))
        .collect::<BTreeSet<_>>();
    let selected_refs = sessions
        .iter()
        .flat_map(|s| {
            s["turns"].as_array().into_iter().flatten().filter_map(|t| {
                Some((s["session_id"].as_str()?.to_owned(), t["turn_id"].as_u64()?))
            })
        })
        .collect::<BTreeSet<_>>();
    let bounded_patterns = patterns
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).expect("serializable pattern");
            let refs = p
                .supporting
                .iter()
                .filter(|r| {
                    r["session_id"]
                        .as_str()
                        .zip(r["turn_id"].as_u64())
                        .is_some_and(|(s, t)| selected_refs.contains(&(s.to_owned(), t)))
                })
                .cloned()
                .collect::<Vec<_>>();
            let included = refs
                .iter()
                .filter_map(|r| r["session_id"].as_str())
                .collect::<BTreeSet<_>>()
                .len();
            v["corpus_sessions"] = json!(p.sessions);
            v["selected_sessions"] = json!(included);
            v["omitted_sessions"] = json!(p.sessions.saturating_sub(included));
            v["omitted_supporting_turns"] = json!(p.supporting.len().saturating_sub(refs.len()));
            v["supporting"] = json!(refs);
            v
        })
        .collect::<Vec<_>>();
    json!({"isolated":isolated,"patterns":bounded_patterns,"corpus_supporting_sessions":corpus_ids.len(),"selected_sessions":sessions.len(),"omitted_supporting_sessions":corpus_ids.len().saturating_sub(sessions.len()),"sessions":sessions,"selection":"Per pattern sample max(3, configured recurrence threshold) distinct sessions, prioritizing failed/partial outcomes then verification gaps, including one successful-outcome comparison when available. Up to 10 supporting turns per session plus neighbors, linked downstream turns, final response and recorded verification; excerpts disclose truncation."})
}

fn comparison_metrics(before: &Value, after: &Value) -> Value {
    let mut metrics = serde_json::Map::new();
    for (question, options) in crate::model::SESSION_QUESTIONS {
        let denominator = |report: &Value| {
            report["counts"][*question]
                .as_object()
                .map(|counts| counts.values().filter_map(Value::as_u64).sum::<u64>())
                .unwrap_or(0)
        };
        let bn = denominator(before);
        let an = denominator(after);
        let mut categories = serde_json::Map::new();
        for category in *options {
            let b = proportion(
                before["counts"][*question][*category].as_u64().unwrap_or(0),
                bn,
            );
            let a = proportion(
                after["counts"][*question][*category].as_u64().unwrap_or(0),
                an,
            );
            let delta = b["proportion"]
                .as_f64()
                .zip(a["proportion"].as_f64())
                .map(|(b, a)| (a - b) * 100.0);
            categories.insert(
                category.to_string(),
                json!({"before":b,"after":a,"delta_percentage_points":delta}),
            );
        }
        metrics.insert(question.to_string(), Value::Object(categories));
    }
    Value::Object(metrics)
}
fn proportion(count: u64, denominator: u64) -> Value {
    if denominator == 0 {
        return json!({"count":count,"denominator":0,"proportion":null,"wilson95":null});
    }
    let n = denominator as f64;
    let p = count as f64 / n;
    let z = 1.959963984540054;
    let z2 = z * z;
    let scale = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / scale;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / scale;
    json!({"count":count,"denominator":denominator,"proportion":p,"wilson95":{"lower":(center-half).max(0.0),"upper":(center+half).min(1.0)}})
}
fn agent_matches(requested: &str, actual: &str) -> bool {
    requested == "all" || requested == actual || (requested == "claude" && actual == "claude_code")
}
fn discovery_options(args: &DiscoveryArgs) -> Result<discovery::DiscoveryOptions> {
    let root = |explicit: &Option<PathBuf>, env: &str, suffix: &str| -> Result<PathBuf> {
        if let Some(p) = explicit {
            return Ok(p.clone());
        }
        if let Some(p) = std::env::var_os(env) {
            return Ok(PathBuf::from(p));
        }
        Ok(PathBuf::from(
            std::env::var_os("HOME")
                .context("configuration: HOME unavailable; supply native source roots")?,
        )
        .join(suffix))
    };
    Ok(discovery::DiscoveryOptions {
        project: args.project.clone().unwrap_or(std::env::current_dir()?),
        agent: if args.agent == "all" {
            None
        } else {
            Some(args.agent.clone())
        },
        codex_home: if args.agent == "all" || args.agent == "codex" {
            root(&args.codex_home, "CODEX_HOME", ".codex")?
        } else {
            PathBuf::new()
        },
        claude_config_dir: if args.agent == "all"
            || args.agent == "claude"
            || args.agent == "claude_code"
        {
            root(&args.claude_config_dir, "CLAUDE_CONFIG_DIR", ".claude")?
        } else {
            PathBuf::new()
        },
    })
}
fn discovery_value(result: &discovery::DiscoveryResult, c: &Config) -> Result<Value> {
    let mut value = json!({"mode":"project_discovery","project_root":result.project_root,"sessions":result.sessions.iter().map(|s|json!({"path":s.path,"agent":s.agent})).collect::<Vec<_>>(),"selected_sessions":result.sessions.len(),"scanned_files":result.scanned_files,"skipped_files":result.skipped_files,"malformed_files":result.malformed_files,"missing_cwd_files":result.missing_cwd_files,"metadata_limited_files":result.metadata_limited_files,"errors":result.errors,"complete":result.errors.is_empty(),"warnings":result.warnings});
    let mut agents = BTreeMap::from([("codex", 0usize), ("claude_code", 0usize)]);
    for session in &result.sessions {
        *agents.entry(session.agent.as_str()).or_default() += 1;
    }
    value["agent_counts"] = json!(agents);
    crate::redact::Redactor::new(&c.redaction)?.value(&mut value);
    Ok(value)
}
