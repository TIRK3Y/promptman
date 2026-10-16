//! Ollama discovery and model management.
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelInfo {
    pub name: String,
    pub size_gb: f64,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalBridgeStatus {
    pub is_running: bool,
    pub host: String,
    pub version: Option<String>,
    pub available_models: Vec<LocalModelInfo>,
}

pub struct LocalBridge {
    host: String,
    client: Client,
}

impl LocalBridge {
    pub fn new(host: &str) -> Self {
        let client = Client::builder().connect_timeout(Duration::from_secs(3)).build().unwrap_or_default();
        Self { host: host.trim_end_matches('/').to_string(), client }
    }

    pub async fn check_status(&self) -> LocalBridgeStatus {
        let get_json = |path: &str| {
            let req = self.client.get(format!("{}{path}", self.host)).timeout(Duration::from_secs(5));
            async move { req.send().await.ok()?.error_for_status().ok()?.json::<Value>().await.ok() }
        };

        let version = get_json("/api/version").await.and_then(|v| v["version"].as_str().map(str::to_string));
        let mut available_models = Vec::new();
        if version.is_some() {
            if let Some(tags) = get_json("/api/tags").await {
                for m in tags["models"].as_array().into_iter().flatten() {
                    let details = &m["details"];
                    available_models.push(LocalModelInfo {
                        name: m["name"].as_str().unwrap_or("unknown").to_string(),
                        size_gb: (m["size"].as_f64().unwrap_or(0.0) / 1_073_741_824.0 * 10.0).round() / 10.0,
                        parameter_size: details["parameter_size"].as_str().map(str::to_string),
                        quantization: details["quantization_level"].as_str().map(str::to_string),
                    });
                }
            }
        }
        available_models.sort_by(|a, b| a.name.cmp(&b.name));

        LocalBridgeStatus { is_running: version.is_some(), host: self.host.clone(), version, available_models }
    }

    /// Downloads a model, reporting (status, percent) as it goes.
    pub async fn pull(&self, model: &str, mut progress: impl FnMut(&str, Option<f32>)) -> Result<(), String> {
        let mut resp = self
            .client
            .post(format!("{}/api/pull", self.host))
            .json(&json!({"model": model, "stream": true}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()));
        }
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let Ok(v) = serde_json::from_slice::<Value>(&line) else { continue };
                if let Some(e) = v["error"].as_str() {
                    return Err(e.to_string());
                }
                let pct = match (v["completed"].as_f64(), v["total"].as_f64()) {
                    (Some(c), Some(t)) if t > 0.0 => Some((c / t) as f32),
                    _ => None,
                };
                progress(v["status"].as_str().unwrap_or(""), pct);
            }
        }
        Ok(())
    }

    pub async fn delete(&self, model: &str) -> Result<(), String> {
        let resp = self
            .client
            .delete(format!("{}/api/delete", self.host))
            .json(&json!({"model": model}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))
        }
    }

    /// Loads a model into memory so the next run doesn't pay the load time.
    pub async fn preload(&self, model: &str) -> Result<(), String> {
        let resp = self
            .client
            .post(format!("{}/api/generate", self.host))
            .json(&json!({"model": model, "keep_alive": "15m"}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::llm::test_server::{json_response, serve_recording, Step};

    #[tokio::test]
    async fn status_lists_installed_models() {
        let tags = r#"{"models":[
            {"name":"mistral:7b","size":4294967296,"details":{"parameter_size":"7B","quantization_level":"Q4_0"}},
            {"name":"llama3:8b","size":1073741824}
        ]}"#;
        let (base, requests) = serve_recording(vec![
            vec![Step::Send(json_response("200 OK", r#"{"version":"0.9.1"}"#))],
            vec![Step::Send(json_response("200 OK", tags))],
        ])
        .await;
        let status = LocalBridge::new(&base).check_status().await;
        assert!(status.is_running);
        assert_eq!(status.version.as_deref(), Some("0.9.1"));
        let names: Vec<_> = status.available_models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["llama3:8b", "mistral:7b"], "sorted by name");
        assert_eq!(status.available_models[1].size_gb, 4.0);
        assert_eq!(status.available_models[1].quantization.as_deref(), Some("Q4_0"));
        assert_eq!(status.available_models[0].parameter_size, None);
        let reqs = requests.lock().unwrap();
        assert!(reqs[0].starts_with("GET /v1/api/version") && reqs[1].starts_with("GET /v1/api/tags"));
    }

    #[tokio::test]
    async fn offline_server_is_reported_as_not_running() {
        let status = LocalBridge::new("http://127.0.0.1:9").check_status().await;
        assert!(!status.is_running);
        assert!(status.available_models.is_empty());
    }

    #[tokio::test]
    async fn pull_reports_progress_and_errors() {
        let lines = "{\"status\":\"pulling\",\"total\":100,\"completed\":25}\n{\"status\":\"pulling\",\"total\":100,\"completed\":100}\n{\"status\":\"success\"}\n";
        let ok = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\n\r\n{lines}",
            lines.len()
        );
        let failed = "{\"error\":\"model not found\"}\n";
        let bad = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{failed}", failed.len());
        let (base, _) = serve_recording(vec![vec![Step::Send(ok)], vec![Step::Send(bad)]]).await;
        let bridge = LocalBridge::new(&base);
        let mut seen = Vec::new();
        bridge.pull("llama3", |status, pct| seen.push((status.to_string(), pct))).await.unwrap();
        assert_eq!(seen[0], ("pulling".to_string(), Some(0.25)));
        assert_eq!(seen[2], ("success".to_string(), None));
        assert_eq!(bridge.pull("nope", |_, _| {}).await.unwrap_err(), "model not found");
    }

    #[tokio::test]
    async fn delete_and_preload_report_http_errors() {
        let (base, requests) = serve_recording(vec![
            vec![Step::Send(json_response("404 Not Found", r#"{"error":"not found"}"#))],
            vec![Step::Send(json_response("200 OK", "{}"))],
        ])
        .await;
        let bridge = LocalBridge::new(&base);
        assert!(bridge.delete("ghost").await.unwrap_err().contains("404"));
        bridge.preload("llama3").await.unwrap();
        let reqs = requests.lock().unwrap();
        assert!(reqs[0].starts_with("DELETE /v1/api/delete"));
        assert!(reqs[1].contains("\"keep_alive\":\"15m\""));
    }
}
