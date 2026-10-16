//! Prompt templates, CSV inputs, output checks and LLM-as-judge.
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

fn var_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{\s*([A-Za-z0-9_]+)\s*\}\}").expect("valid regex"))
}

/// Variable names used as `{{name}}` in a template, in order of first use.
pub fn template_vars(template: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for cap in var_regex().captures_iter(template) {
        let name = cap[1].to_string();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// Replaces `{{name}}` with values; unknown variables are left as written.
pub fn fill_template(template: &str, vars: &HashMap<String, String>) -> String {
    var_regex()
        .replace_all(template, |cap: &regex::Captures| vars.get(&cap[1]).cloned().unwrap_or_else(|| cap[0].to_string()))
        .into_owned()
}

/// A CSV file: header names and one map per row.
pub struct CsvTable {
    pub headers: Vec<String>,
    pub rows: Vec<HashMap<String, String>>,
}

pub fn load_csv(path: &Path) -> Result<CsvTable, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    let headers: Vec<String> =
        reader.headers().map_err(|e| e.to_string())?.iter().map(|h| h.trim().to_string()).collect();
    let mut rows = Vec::new();
    for rec in reader.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        rows.push(headers.iter().cloned().zip(rec.iter().map(str::to_string)).collect());
    }
    Ok(CsvTable { headers, rows })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Check {
    Contains(String),
    NotContains(String),
    Regex(String),
    ValidJson,
}

impl Check {
    pub fn label(&self) -> String {
        match self {
            Check::Contains(s) => format!("contains \"{s}\""),
            Check::NotContains(s) => format!("does not contain \"{s}\""),
            Check::Regex(s) => format!("matches /{s}/"),
            Check::ValidJson => "is valid JSON".to_string(),
        }
    }

    /// Checks an output; an invalid regex fails with its error.
    pub fn passes(&self, output: &str) -> Result<bool, String> {
        Ok(match self {
            Check::Contains(s) => output.to_lowercase().contains(&s.to_lowercase()),
            Check::NotContains(s) => !output.to_lowercase().contains(&s.to_lowercase()),
            Check::Regex(p) => Regex::new(p).map_err(|e| e.to_string())?.is_match(output),
            Check::ValidJson => serde_json::from_str::<serde_json::Value>(strip_code_fence(output)).is_ok(),
        })
    }
}

/// Runs every check, plus `contains(expected)` when the row has an `expected` value.
pub fn run_checks(checks: &[Check], expected: Option<&str>, output: &str) -> (bool, Vec<String>) {
    let mut failures = Vec::new();
    let implicit = expected.filter(|e| !e.trim().is_empty()).map(|e| Check::Contains(e.to_string()));
    for check in checks.iter().chain(implicit.as_ref()) {
        match check.passes(output) {
            Ok(true) => {}
            Ok(false) => failures.push(check.label()),
            Err(e) => failures.push(format!("{}: {e}", check.label())),
        }
    }
    (failures.is_empty(), failures)
}

/// Models often wrap JSON in a ```json fence; look inside it.
fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .and_then(|r| r.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(t)
}

pub const JUDGE_SYSTEM: &str = "You are an impartial evaluator. Score each answer from 0 to 10 for \
correctness, completeness and clarity in answering the user's prompt. Reply with JSON only, in the form \
{\"scores\":[{\"answer\":\"A\",\"score\":7,\"reason\":\"one sentence\"}]}, with one entry per answer in order.";

pub fn judge_prompt(prompt: &str, answers: &[&str]) -> String {
    let mut s = format!("User prompt:\n<prompt>\n{prompt}\n</prompt>\n");
    for (i, a) in answers.iter().enumerate() {
        let label = (b'A' + i as u8) as char;
        s.push_str(&format!("\nAnswer {label}:\n<answer_{label}>\n{a}\n</answer_{label}>\n"));
    }
    s
}

#[derive(Deserialize)]
struct JudgeReply {
    scores: Vec<JudgeScore>,
}

#[derive(Deserialize)]
struct JudgeScore {
    score: f32,
    #[serde(default)]
    reason: String,
}

/// Parses the judge's JSON into (score, reason) per answer.
pub fn parse_judge(reply: &str, answers: usize) -> Result<Vec<(f32, String)>, String> {
    let start = reply.find('{').ok_or("judge reply had no JSON")?;
    let end = reply.rfind('}').ok_or("judge reply had no JSON")?;
    let parsed: JudgeReply =
        serde_json::from_str(&reply[start..=end]).map_err(|e| format!("judge reply was not valid JSON: {e}"))?;
    if parsed.scores.len() != answers {
        return Err(format!("judge scored {} answers, expected {answers}", parsed.scores.len()));
    }
    Ok(parsed.scores.into_iter().map(|s| (s.score.clamp(0.0, 10.0), s.reason)).collect())
}

pub const OPTIMIZE_SYSTEM: &str = "You rewrite prompts for large language models. Rewrite the user's draft so it is clear, specific and as short as possible without losing any requirement: state the task, the inputs, any constraints, and the expected output format. Remove filler, repetition and pleasantries. Keep every {{placeholder}} exactly as written. Do not answer the draft and do not add requirements it doesn't imply. Reply with only the rewritten prompt: no preamble, no quotes, no code fences.";

pub fn optimize_request(draft: &str) -> String {
    format!(
        "<draft>
{draft}
</draft>"
    )
}

/// Strips wrappers models sometimes add around a rewritten prompt.
pub fn clean_rewrite(reply: &str) -> String {
    let mut t = strip_code_fence(reply).trim();
    for tag in ["<draft>", "<prompt>"] {
        if let Some(rest) = t.strip_prefix(tag) {
            let close = tag.replace('<', "</");
            t = rest.strip_suffix(close.as_str()).unwrap_or(rest).trim();
        }
    }
    let quoted =
        t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')));
    if quoted {
        t[1..t.len() - 1].trim().to_string()
    } else {
        t.to_string()
    }
}

/// Variables in the original that the rewrite dropped.
pub fn missing_vars(original: &str, rewritten: &str) -> Vec<String> {
    let kept = template_vars(rewritten);
    template_vars(original).into_iter().filter(|v| !kept.contains(v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        let t = "Translate {{text}} to {{ lang }}. Again: {{text}} {{missing}}";
        assert_eq!(template_vars(t), vec!["text", "lang", "missing"]);
        let vars = HashMap::from([("text".to_string(), "hi".to_string()), ("lang".to_string(), "French".to_string())]);
        assert_eq!(fill_template(t, &vars), "Translate hi to French. Again: hi {{missing}}");
    }

    #[test]
    fn checks() {
        assert!(Check::Contains("PARIS".into()).passes("It is Paris.").unwrap());
        assert!(!Check::NotContains("paris".into()).passes("Paris").unwrap());
        assert!(Check::Regex(r"^\d+$".into()).passes("42").unwrap());
        assert!(Check::Regex("(".into()).passes("x").is_err());
        assert!(Check::ValidJson.passes("```json\n{\"a\": 1}\n```").unwrap());
        assert!(!Check::ValidJson.passes("{nope}").unwrap());

        let (ok, fails) = run_checks(&[Check::ValidJson], Some("42"), "{\"answer\": 41}");
        assert!(!ok);
        assert_eq!(fails, vec!["contains \"42\""]);
    }

    #[test]
    fn judge() {
        let p = judge_prompt("q", &["x", "y"]);
        assert!(p.contains("<answer_A>") && p.contains("<answer_B>"));
        let reply = "Here you go: {\"scores\":[{\"answer\":\"A\",\"score\":8,\"reason\":\"ok\"},{\"answer\":\"B\",\"score\":12}]}";
        assert_eq!(parse_judge(reply, 2).unwrap(), vec![(8.0, "ok".to_string()), (10.0, String::new())]);
        assert!(parse_judge(reply, 3).is_err());
        assert!(parse_judge("no json", 1).is_err());
    }

    #[test]
    fn rewrite_cleanup() {
        assert_eq!(
            clean_rewrite(
                "```
Summarize {{text}}.
```"
            ),
            "Summarize {{text}}."
        );
        assert_eq!(clean_rewrite("\"List 3 risks.\""), "List 3 risks.");
        assert_eq!(clean_rewrite("<prompt>Do X.</prompt>"), "Do X.");
        assert_eq!(missing_vars("Fix {{code}} in {{lang}}", "Fix {{code}}"), vec!["lang"]);
        assert!(optimize_request("hi").contains(
            "<draft>
hi
</draft>"
        ));
    }

    #[test]
    fn csv_with_unicode_newlines_and_empty_cells() {
        let path = std::env::temp_dir().join(format!("promptman-csv2-{}.csv", std::process::id()));
        std::fs::write(&path, "text,expected\n\"line one\nline two\",✓\n,\n\"日本語, ok\",\"say \"\"hi\"\"\"\n")
            .unwrap();
        let table = load_csv(&path).unwrap();
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[0]["text"], "line one\nline two");
        assert_eq!(table.rows[1]["text"], "");
        assert_eq!(table.rows[2]["expected"], "say \"hi\"");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn malformed_csv_is_an_error_not_a_panic() {
        let path = std::env::temp_dir().join(format!("promptman-csv3-{}.csv", std::process::id()));
        std::fs::write(&path, "a,b\n1,2,3\n").unwrap();
        assert!(load_csv(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(load_csv(&path).is_err(), "missing file");
    }

    #[test]
    fn no_checks_means_every_answer_passes() {
        assert_eq!(run_checks(&[], None, "anything"), (true, vec![]));
        assert_eq!(run_checks(&[], Some("  "), "anything"), (true, vec![]), "a blank expected value is ignored");
    }

    #[test]
    fn fill_template_with_no_variables_is_unchanged() {
        assert_eq!(fill_template("plain {not a var}", &HashMap::new()), "plain {not a var}");
        assert!(template_vars("{{ }} {{bad-name}}").is_empty());
    }

    #[test]
    fn csv_load() {
        let path = std::env::temp_dir().join(format!("promptman-csv-{}.csv", std::process::id()));
        std::fs::write(&path, "text,expected\n\"a, b\",x\nc,y\n").unwrap();
        let table = load_csv(&path).unwrap();
        assert_eq!(table.headers, vec!["text", "expected"]);
        assert_eq!(table.rows[0]["text"], "a, b");
        assert_eq!(table.rows.len(), 2);
        std::fs::remove_file(path).unwrap();
    }
}
