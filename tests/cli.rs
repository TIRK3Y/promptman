//! End-to-end tests of `promptman-cli` against a local mock of an OpenAI-compatible server.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use promptman::core_engine::db::Db;

/// Answers every chat request with `answer`; returns the base URL and the request bodies seen.
fn mock_server(answer: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut len = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = reader.read_exact(&mut body);
            seen.lock().unwrap().push(String::from_utf8_lossy(&body).into_owned());
            let delta = serde_json::json!({"choices": [{"delta": {"content": answer}}]});
            let sse = format!("data: {delta}\n\ndata: [DONE]\n\n");
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
                sse.len()
            );
            let _ = reader.get_mut().write_all(reply.as_bytes());
        }
    });
    (base, bodies)
}

/// A fresh data folder whose "custom" provider points at `base`. Removed on drop.
struct Profile(PathBuf);

impl Profile {
    fn new(name: &str, base: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("promptman-cli-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Db::open(&dir.join("promptman.db")).unwrap().set_setting("base_url.custom", &base.to_string()).unwrap();
        Self(dir)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_promptman-cli"))
            .args(args)
            .env("PROMPTMAN_DATA_DIR", &self.0)
            .output()
            .unwrap()
    }

    fn saved_runs(&self) -> usize {
        Db::open(&self.0.join("promptman.db")).unwrap().history(100).unwrap().len()
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn passing_checks_exit_zero_and_save_the_run() {
    let (base, bodies) = mock_server("hello world");
    let p = Profile::new("pass", &base);
    let out = p.run(&["--models", "custom/m", "--prompt", "say hi", "--contains", "hello", "--format", "jsonl"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let line: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(line["passed"], true);
    assert_eq!(line["output"], "hello world");
    assert_eq!(line["model"], "custom/m");
    assert!(bodies.lock().unwrap()[0].contains("say hi"));
    assert_eq!(p.saved_runs(), 1);
}

#[test]
fn failing_checks_exit_one() {
    let (base, _) = mock_server("hello world");
    let p = Profile::new("fail", &base);
    let out = p.run(&["--models", "custom/m", "--prompt", "x", "--json", "--no-save"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).contains("FAIL"));
    assert_eq!(p.saved_runs(), 0, "--no-save keeps history empty");
}

#[test]
fn unreachable_model_exits_one() {
    let p = Profile::new("down", "http://127.0.0.1:9/v1");
    let out = p.run(&["--models", "custom/m", "--prompt", "x", "--no-save", "--format", "jsonl"]);
    assert_eq!(out.status.code(), Some(1));
    let line: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert!(line["failures"][0].as_str().unwrap().starts_with("error:"));
}

#[test]
fn csv_rows_fill_the_template() {
    let (base, bodies) = mock_server("ok");
    let p = Profile::new("csv", &base);
    let csv = p.0.join("rows.csv");
    std::fs::write(&csv, "name,expected\nAda,ok\n\"Grace, Hopper\",ok\n").unwrap();
    let out = p.run(&[
        "--models",
        "custom/m",
        "--prompt",
        "Hi {{name}}",
        "--csv",
        csv.to_str().unwrap(),
        "--format",
        "jsonl",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out).lines().count(), 2);
    let bodies = bodies.lock().unwrap();
    assert!(bodies.iter().any(|b| b.contains("Hi Ada")) && bodies.iter().any(|b| b.contains("Hi Grace, Hopper")));
    assert_eq!(p.saved_runs(), 2);
}

#[test]
fn usage_errors_exit_two_without_calling_a_model() {
    let (base, bodies) = mock_server("ok");
    let p = Profile::new("usage", &base);
    let header_only = p.0.join("empty.csv");
    std::fs::write(&header_only, "prompt\n").unwrap();
    for args in [
        vec!["--prompt", "x"],
        vec!["--models", "nonsense", "--prompt", "x"],
        vec!["--models", "custom/m"],
        vec!["--models", "custom/m", "--csv", header_only.to_str().unwrap()],
        vec!["--models", "custom/m", "--prompt", "x", "--format", "xml"],
    ] {
        assert_eq!(p.run(&args).status.code(), Some(2), "{args:?}");
    }
    assert!(bodies.lock().unwrap().is_empty());
}

#[test]
fn models_lists_providers() {
    let p = Profile::new("models", "http://127.0.0.1:9/v1");
    let out = p.run(&["models"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("custom ("));
}
