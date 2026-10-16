//! Command-line runner for comparisons and regression checks (CI, scripts).
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use promptman::core_engine::db::RunRecord;
use promptman::core_engine::engine::{Engine, RunHooks};
use promptman::core_engine::evals::{fill_template, load_csv, run_checks, Check};
use promptman::core_engine::llm::{Message, Params};
use promptman::core_engine::providers::{ModelRef, PROVIDERS};
use tokio_util::sync::CancellationToken;

const USAGE: &str = "\
Usage:
  promptman-cli --models <provider/model,...> (--prompt <text> | --prompt-file <path>) [options]
  promptman-cli models

Options:
  --csv <path>           Run once per row; {{column}} in the prompt is filled from the row.
                         A `prompt` column is used when no prompt is given; an `expected`
                         column adds a contains-check for that row.
  --system <text>        System prompt
  --temperature <f>      Default 0.7
  --top-p <f>            Default 1.0
  --max-tokens <n>       Default 4096
  --contains <text>      Fail unless the output contains text (repeatable)
  --not-contains <text>  Fail if the output contains text (repeatable)
  --regex <pattern>      Fail unless the output matches (repeatable)
  --json                 Fail unless the output is valid JSON
  --format table|jsonl   Output format (default table)
  --no-save              Don't record runs in the PromptMan history

Exit code: 0 when every run succeeds and passes its checks, 1 otherwise, 2 for usage errors.
API keys come from the PromptMan keychain entries or <PROVIDER>_API_KEY variables.";

struct Args {
    models: Vec<ModelRef>,
    prompt: Option<String>,
    csv: Option<PathBuf>,
    system: String,
    params: Params,
    checks: Vec<Check>,
    jsonl: bool,
    save: bool,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut a = Args {
        models: Vec::new(),
        prompt: None,
        csv: None,
        system: String::new(),
        params: Params::default(),
        checks: Vec::new(),
        jsonl: false,
        save: true,
    };
    let mut it = raw.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--models" => {
                for m in value()?.split(',').filter(|s| !s.trim().is_empty()) {
                    a.models.push(ModelRef::parse(m.trim())?);
                }
            }
            "--prompt" => a.prompt = Some(value()?),
            "--prompt-file" => {
                let path = value()?;
                a.prompt = Some(std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?);
            }
            "--csv" => a.csv = Some(PathBuf::from(value()?)),
            "--system" => a.system = value()?,
            "--temperature" => a.params.temperature = value()?.parse().map_err(|_| "bad --temperature")?,
            "--top-p" => a.params.top_p = value()?.parse().map_err(|_| "bad --top-p")?,
            "--max-tokens" => a.params.max_tokens = value()?.parse().map_err(|_| "bad --max-tokens")?,
            "--contains" => a.checks.push(Check::Contains(value()?)),
            "--not-contains" => a.checks.push(Check::NotContains(value()?)),
            "--regex" => a.checks.push(Check::Regex(value()?)),
            "--json" => a.checks.push(Check::ValidJson),
            "--format" => {
                a.jsonl = match value()?.as_str() {
                    "jsonl" => true,
                    "table" => false,
                    other => return Err(format!("unknown format {other}")),
                }
            }
            "--no-save" => a.save = false,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if a.models.is_empty() {
        return Err("--models is required".into());
    }
    Ok(a)
}

/// CSV rows to run; a file without data rows is an error so CI can't pass on nothing.
fn read_rows(path: &std::path::Path) -> Result<Vec<HashMap<String, String>>, String> {
    let table = load_csv(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if table.rows.is_empty() {
        return Err(format!("{} has no data rows", path.display()));
    }
    Ok(table.rows)
}

fn list_models(engine: &Engine) {
    let pricing = engine.pricing.read().unwrap_or_else(|p| p.into_inner());
    for p in PROVIDERS {
        println!("{} ({})", p.id, p.name);
        for m in pricing.models_for(p.id) {
            println!("  {}/{m}", p.id);
        }
        if p.local {
            println!("  {}/<any installed model>", p.id);
        }
    }
    println!("\nPrices: {}", pricing.path.display());
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let (engine, notes) = match Engine::open() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    notes.iter().for_each(|n| eprintln!("note: {n}"));

    if raw.first().map(String::as_str) == Some("models") {
        list_models(&engine);
        return ExitCode::SUCCESS;
    }
    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let rows: Vec<HashMap<String, String>> = match &args.csv {
        Some(path) => match read_rows(path) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        },
        None => vec![HashMap::new()],
    };
    let template = match (&args.prompt, rows.first().is_some_and(|r| r.contains_key("prompt"))) {
        (Some(p), _) => p.clone(),
        (None, true) => "{{prompt}}".to_string(),
        (None, false) => {
            eprintln!("error: give --prompt, --prompt-file, or a CSV with a `prompt` column\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let engine = Arc::new(engine);
    let mut all_ok = true;
    if !args.jsonl {
        println!("{:<4} {:<34} {:<6} {:>9} {:>10}  output", "row", "model", "result", "latency", "cost");
    }

    for (i, row) in rows.iter().enumerate() {
        let prompt = fill_template(&template, row);
        let convs = vec![vec![Message::user(prompt.clone())]; args.models.len()];
        let cancels = vec![CancellationToken::new(); args.models.len()];
        let results = runtime.block_on(engine.run_all(
            &args.models,
            &args.system,
            convs,
            &args.params,
            cancels,
            RunHooks::none(),
        ));

        for r in &results {
            let (passed, failures) = match &r.error {
                Some(e) => (false, vec![format!("error: {e}")]),
                None => run_checks(&args.checks, row.get("expected").map(String::as_str), &r.output),
            };
            all_ok &= passed;
            if args.jsonl {
                let line = serde_json::json!({
                    "row": i, "model": r.model_ref(), "passed": passed, "failures": failures,
                    "latency_ms": r.latency_ms, "ttft_ms": r.ttft_ms, "prompt_tokens": r.prompt_tokens,
                    "completion_tokens": r.completion_tokens, "cost_usd": r.cost_usd, "output": r.output,
                });
                println!("{line}");
            } else {
                let first_line: String = r.output.lines().next().unwrap_or("").chars().take(60).collect();
                let cost = r.cost_usd.map_or("?".to_string(), |c| format!("${c:.5}"));
                println!(
                    "{:<4} {:<34} {:<6} {:>7.0}ms {:>10}  {}",
                    i,
                    r.model_ref(),
                    if passed { "pass" } else { "FAIL" },
                    r.latency_ms,
                    cost,
                    if passed { first_line } else { failures.join("; ") },
                );
            }
        }

        if args.save {
            let run = RunRecord {
                kind: "cli".into(),
                prompt,
                system: args.system.clone(),
                params: args.params.clone(),
                ..Default::default()
            };
            if let Err(e) = engine.db.save_run(&run, &results) {
                eprintln!("warning: could not save run: {e}");
            }
        }
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn csv_without_rows_is_an_error() {
        let path = std::env::temp_dir().join(format!("promptman-cli-empty-{}.csv", std::process::id()));
        std::fs::write(&path, "text,expected\n").unwrap();
        assert!(read_rows(&path).unwrap_err().contains("no data rows"));
        std::fs::write(&path, "text\nhello\n").unwrap();
        assert_eq!(read_rows(&path).unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn parses_flags() {
        let a = parse_args(&s(&[
            "--models",
            "openai/gpt-4o,ollama/llama3",
            "--prompt",
            "hi",
            "--json",
            "--contains",
            "x",
            "--max-tokens",
            "64",
            "--format",
            "jsonl",
            "--no-save",
        ]))
        .unwrap();
        assert_eq!(a.models.len(), 2);
        assert_eq!(a.checks, vec![Check::ValidJson, Check::Contains("x".into())]);
        assert_eq!(a.params.max_tokens, 64);
        assert!(a.jsonl && !a.save);
        assert!(parse_args(&s(&["--prompt", "hi"])).is_err());
        assert!(parse_args(&s(&["--models", "bad"])).is_err());
        assert!(parse_args(&s(&["--models"])).is_err());
    }
}
