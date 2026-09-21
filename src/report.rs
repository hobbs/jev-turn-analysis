//! Markdown presentation of the complete corpus and the sampled recommendations.
use crate::ui;
use serde_json::Value;
use std::fmt::Write;

fn cell(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "&#92;")
        .replace('|', "&#124;")
        .replace('`', "&#96;")
        .replace('*', "&#42;")
        .replace('_', "&#95;")
        .replace(['\n', '\r'], " ")
}

fn label(text: &str) -> String {
    cell(&text.replace('_', " "))
}

fn display_value(v: &Value) -> String {
    if v.is_null() {
        "unknown".into()
    } else {
        ui::value(v)
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn share(n: u64, total: u64) -> String {
    if total == 0 {
        "unknown".into()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / total as f64)
    }
}

fn distribution(out: &mut String, values: &Value) {
    out.push_str("| Result | Count | Share of included judgments |\n| --- | ---: | ---: |\n");
    if let Some(values) = values.as_object() {
        let total: u64 = values.values().filter_map(Value::as_u64).sum();
        for (name, value) in values {
            let n = value.as_u64().unwrap_or(0);
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                label(name),
                count(n),
                share(n, total)
            );
        }
    }
    out.push('\n');
}

pub fn markdown(data: &Value) -> String {
    let stats = &data["statistics"];
    let sessions = stats["sessions"].as_u64().unwrap_or(0);
    let turns = stats["turns"].as_u64().unwrap_or(0);
    let mut out = format!(
        "# Coding agent report\n\nGenerated {}.\n\n",
        cell(data["created_at"].as_str().unwrap_or(""))
    );
    let review = ui::review_report(data);
    let review = review
        .strip_prefix("# Review report\n\n")
        .unwrap_or(&review);
    // Keep the takeaway first, followed by the statistics and the detailed proposals.
    let (summary, recommendations) = review.split_once("## Themes\n").unwrap_or((review, ""));
    out.push_str(summary);
    out.push_str("## Corpus statistics\n\n");
    let _ = writeln!(out, "| Analyzed sessions | Included turns | Initially sampled sessions |\n| ---: | ---: | ---: |\n| {} | {} | {} |\n", count(sessions), count(turns), count(data["selection"]["sampled_sessions"].as_u64().unwrap_or(0)));
    out.push_str("Statistics cover all analyzed sessions selected by the filters. Investigations start with the sample shown above and can retrieve more staged evidence; reported inspection appears separately. Missing measurements are shown as unknown.\n\n");
    let _ = writeln!(out, "{} current sessions in the workspace have no matching analysis and are excluded from these statistics. This workspace-wide count is independent of report filters.\n", display_value(&stats["unscored_current_sessions"]));
    if let Some(filters) = data["filters"].as_object() {
        let active: Vec<_> = filters
            .iter()
            .filter(|(key, value)| {
                !value.is_null()
                    && !(key.as_str() == "min_confidence" && value.as_f64() == Some(0.0))
            })
            .collect();
        if !active.is_empty() {
            out.push_str("### Applied filters\n\n| Filter | Value |\n| --- | --- |\n");
            for (key, value) in active {
                let _ = writeln!(out, "| {} | {} |", label(key), cell(&display_value(value)));
            }
            out.push('\n');
        }
    }
    out.push_str("### Outcomes and turn assessments\n\nShares use the included judgments for each question as their denominator. Session questions and turn questions have different denominators. Turn filters do not narrow the session outcome counts.\n\n");
    if let Some(counts) = stats["counts"].as_object() {
        let questions = [
            ("task_outcome", "Task outcomes"),
            ("outcome_verification", "Verification"),
            ("user_intervention", "User intervention"),
            ("usefulness", "Turn usefulness"),
            ("functional_role", "Role in the task"),
            ("downstream_use", "Downstream use"),
            ("outcome_effect", "Effect on the outcome"),
            ("opportunity", "Opportunities"),
            ("remediation_surface", "Where changes could help"),
        ];
        for (question, heading) in questions {
            if let Some(values) = counts.get(question) {
                let _ = writeln!(out, "#### {heading}\n");
                distribution(&mut out, values);
            }
        }
        for (question, values) in counts
            .iter()
            .filter(|(key, _)| !questions.iter().any(|(q, _)| q == key))
        {
            let _ = writeln!(out, "#### {}\n", label(question));
            distribution(&mut out, values);
        }
    }
    out.push_str("### Recurring patterns\n\nThese are broad opportunity and remediation categories, not discovered root causes. A session can appear in several rows. Ordering puts categories associated with incomplete tasks first, then verification gaps, then other opportunities; within each tier, more sessions come first.\n\n");
    out.push_str("Use `jta show <pattern-id>` to inspect the original supporting turns.\n\n| Pattern ID | Opportunity | Where to make a change | Sessions | Share of corpus | Supporting turns |\n| --- | --- | --- | ---: | ---: | ---: |\n");
    if let Some(patterns) = stats["patterns"].as_array() {
        for p in patterns {
            let n = p["sessions"].as_u64().unwrap_or(0);
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} |",
                cell(p["id"].as_str().unwrap_or("unknown")),
                label(p["opportunity"].as_str().unwrap_or("unknown")),
                label(p["surface"].as_str().unwrap_or("unknown")),
                count(n),
                share(n, sessions),
                count(p["supporting"].as_array().map_or(0, Vec::len) as u64)
            );
        }
        if patterns.is_empty() {
            out.push_str("| No recurring patterns found | | | | | |\n");
        }
    }
    out.push_str("\n### Tokens and time\n\nToken totals include only turns with recorded usage. Elapsed time comes from session timestamps and can include idle time; it is not active work time.\n\n| Measurement | Observed value | Coverage |\n| --- | ---: | --- |\n");
    let resources = &stats["resources"];
    for (key, name) in [
        ("input_tokens", "Input tokens"),
        ("output_tokens", "Output tokens"),
        ("cached_input_tokens", "Cached input tokens"),
    ] {
        let metric = &resources[key];
        let _ = writeln!(
            out,
            "| {name} | {} | {}/{} turns; {}/{} sessions |",
            display_value(&metric["total"]),
            display_value(&metric["turn_coverage"]),
            count(turns),
            display_value(&metric["session_coverage"]),
            count(sessions)
        );
    }
    let _ = writeln!(
        out,
        "| Tokens in turns judged wasted | {} | {}/{} turns |",
        display_value(&resources["observed_wasted_tokens"]),
        display_value(&resources["wasted_token_turn_coverage"]),
        count(turns)
    );
    for (key, name) in [
        ("elapsed_ms", "Total session time"),
        ("median_session_ms", "Median session time"),
    ] {
        let value = resources[key]
            .as_f64()
            .map(ui::duration)
            .unwrap_or_else(|| "unknown".into());
        let _ = writeln!(
            out,
            "| {name} | {value} | {}/{} sessions |",
            display_value(&resources["timing_session_coverage"]),
            count(sessions)
        );
    }
    out.push('\n');
    for (key, heading) in [
        ("agent_breakdown", "By agent"),
        ("project_breakdown", "By project"),
    ] {
        let _ = writeln!(out, "### {heading}\n\n| Cohort | Sessions | Turns | Complete | Verified | Input tokens (turn coverage) |\n| --- | ---: | ---: | ---: | ---: | --- | ");
        if let Some(groups) = stats[key].as_object() {
            for (name, group) in groups {
                let outcome_total: u64 = group["outcomes"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.values())
                    .filter_map(Value::as_u64)
                    .sum();
                let verification_total: u64 = group["verification"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.values())
                    .filter_map(Value::as_u64)
                    .sum();
                let complete = group["outcomes"]["complete"].as_u64().unwrap_or(0);
                let verified = group["verification"]["verified"].as_u64().unwrap_or(0);
                let metric = &group["resources"]["input_tokens"];
                let _ = writeln!(
                    out,
                    "| {} | {} | {} | {}/{} ({}) | {}/{} ({}) | {} ({}/{} turns) |",
                    cell(name),
                    display_value(&group["sessions"]),
                    display_value(&group["turns"]),
                    count(complete),
                    count(outcome_total),
                    share(complete, outcome_total),
                    count(verified),
                    count(verification_total),
                    share(verified, verification_total),
                    display_value(&metric["total"]),
                    display_value(&metric["turn_coverage"]),
                    display_value(&group["turns"])
                );
            }
        }
        out.push('\n');
    }
    out.push_str("Cohort differences do not establish that one agent or project workflow caused better results.\n\n");
    if stats["groups"].as_object().is_some_and(|g| !g.is_empty()) {
        out.push_str("### Requested grouping\n\n");
        distribution(&mut out, &stats["groups"]);
    }
    out.push_str("### Coverage and uncertainty\n\n");
    let _ = writeln!(out, "{} judgments were excluded by confidence or consistency checks. {} turns have low-confidence or inconsistent judgments across the selected sessions (before turn filters).\n", display_value(&stats["excluded_judgments"]), display_value(&stats["uncertain_turns"]));
    if stats["excluded_by_question"]
        .as_object()
        .is_some_and(|m| !m.is_empty())
    {
        distribution(&mut out, &stats["excluded_by_question"]);
    }
    for key in [
        "warnings",
        "configuration_fingerprints",
        "resolved_jev_models",
    ] {
        if let Some(values) = stats[key].as_array().filter(|a| !a.is_empty()) {
            let _ = writeln!(out, "{}:\n", label(key));
            for value in values {
                let _ = writeln!(out, "- {}", cell(&display_value(value)));
            }
            out.push('\n');
        }
    }
    if let Some(warnings) = stats["evidence_warnings"]
        .as_array()
        .filter(|a| !a.is_empty())
    {
        out.push_str("#### Source and analysis warnings\n\n| Session | Warning |\n| --- | --- |\n");
        for item in warnings {
            for key in ["source_warnings", "analysis_warnings"] {
                for warning in item[key].as_array().into_iter().flatten() {
                    let _ = writeln!(
                        out,
                        "| {} | {} |",
                        cell(item["session_id"].as_str().unwrap_or("unknown")),
                        cell(&display_value(warning))
                    );
                }
            }
        }
        out.push('\n');
    }
    if !recommendations.is_empty() {
        out.push_str("## Themes\n");
        out.push_str(recommendations);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_cells_cannot_break_rows_or_add_markup() {
        assert_eq!(cell("a|b\n<script>`*"), "a&#124;b &lt;script&gt;&#96;&#42;");
        assert_eq!(count(300000), "300,000");
        assert_eq!(share(3, 3000), "0.1%");
        assert_eq!(share(0, 0), "unknown");
    }
}
