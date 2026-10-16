//! Streaming chat across the four wire protocols.
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::providers::Protocol;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(s: impl Into<String>) -> Self {
        Self { role: Role::User, content: s.into() }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: s.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self { temperature: 0.7, top_p: 1.0, max_tokens: 4096 }
    }
}

/// Where and how to send a request.
pub struct Endpoint {
    pub provider_id: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: Option<String>,
    /// Longest wait for the next piece of a reply. A total limit would cut off long
    /// generations that are still making progress; this only catches stalls.
    pub idle_timeout: Duration,
}

/// Default for [`Endpoint::idle_timeout`].
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Retries for rate limits and server errors, only before any of the reply has arrived.
const MAX_RETRIES: u32 = 2;
/// Longest `Retry-After` honoured; longer waits fail with the provider's message instead.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(20);

/// What to send: shared by every provider and by every feature that asks a model something.
#[derive(Clone, Copy)]
pub struct Prompt<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub params: &'a Params,
}

/// Why a reply ended early. Whatever text had arrived is kept in `partial`.
#[derive(Debug)]
pub struct StreamError {
    pub message: String,
    pub partial: Completion,
    /// The user stopped it.
    pub cancelled: bool,
    /// The provider accepted the request, so it may have billed for it.
    pub started: bool,
}

impl StreamError {
    fn new(message: impl Into<String>, partial: Completion, started: bool) -> Box<Self> {
        Box::new(Self { message: message.into(), partial, cancelled: false, started })
    }

    fn stopped(partial: Completion, started: bool) -> Box<Self> {
        Box::new(Self { message: "Stopped".into(), partial, cancelled: true, started })
    }
}

#[derive(Debug, Default)]
pub struct Completion {
    pub text: String,
    /// Token counts reported by the provider, when it reports them.
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub ttft_ms: Option<f64>,
    /// Something the user should know about this answer (fallback, truncation, …).
    pub note: Option<String>,
}

/// Anthropic models that support server-side refusal fallbacks (`fallbacks: "default"`).
const ANTHROPIC_FALLBACK_MODELS: &[&str] =
    &["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];

/// Streams one chat completion, calling `on_delta` for each text fragment.
pub async fn stream_chat(
    client: &Client,
    ep: &Endpoint,
    model: &str,
    prompt: &Prompt<'_>,
    cancel: &CancellationToken,
    mut on_delta: impl FnMut(&str),
) -> Result<Completion, Box<StreamError>> {
    let Prompt { system, messages, params } = *prompt;
    let start = Instant::now();
    let base = ep.base_url.trim_end_matches('/');

    let request = match ep.protocol {
        Protocol::OpenAi => {
            let mut msgs: Vec<Value> = Vec::new();
            if !system.trim().is_empty() {
                msgs.push(json!({"role": "system", "content": system}));
            }
            msgs.extend(messages.iter().map(|m| json!({"role": m.role, "content": m.content})));
            let mut body = json!({
                "model": model,
                "messages": msgs,
                "stream": true,
                "temperature": params.temperature,
                "top_p": params.top_p,
            });
            // OpenAI renamed max_tokens; other OpenAI-compatible servers still use the old name.
            let max_key = if ep.provider_id == "openai" { "max_completion_tokens" } else { "max_tokens" };
            body[max_key] = json!(params.max_tokens);
            // Mistral reports usage on its own and rejects unknown fields.
            if ep.provider_id != "mistral" {
                body["stream_options"] = json!({"include_usage": true});
            }
            let mut req = client.post(format!("{base}/chat/completions")).json(&body);
            if let Some(key) = &ep.api_key {
                req = req.bearer_auth(key);
            }
            req
        }
        Protocol::Anthropic => {
            // Current Anthropic models reject temperature/top_p, so they are not sent.
            let mut body = json!({
                "model": model,
                "max_tokens": params.max_tokens,
                "stream": true,
                "messages": messages,
            });
            if !system.trim().is_empty() {
                body["system"] = json!(system);
            }
            let mut req = client
                .post(format!("{base}/messages"))
                .header("x-api-key", ep.api_key.clone().unwrap_or_default())
                .header("anthropic-version", "2023-06-01");
            if ANTHROPIC_FALLBACK_MODELS.contains(&model) {
                body["fallbacks"] = json!("default");
                req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            req.json(&body)
        }
        Protocol::Gemini => {
            let contents: Vec<Value> = messages
                .iter()
                .map(|m| {
                    let role = if m.role == Role::User { "user" } else { "model" };
                    json!({"role": role, "parts": [{"text": m.content}]})
                })
                .collect();
            let mut body = json!({
                "contents": contents,
                "generationConfig": {
                    "temperature": params.temperature,
                    "topP": params.top_p,
                    "maxOutputTokens": params.max_tokens,
                },
            });
            if !system.trim().is_empty() {
                body["systemInstruction"] = json!({"parts": [{"text": system}]});
            }
            client
                .post(format!("{base}/models/{model}:streamGenerateContent?alt=sse"))
                .header("x-goog-api-key", ep.api_key.clone().unwrap_or_default())
                .json(&body)
        }
        Protocol::Ollama => {
            let mut msgs: Vec<Value> = Vec::new();
            if !system.trim().is_empty() {
                msgs.push(json!({"role": "system", "content": system}));
            }
            msgs.extend(messages.iter().map(|m| json!({"role": m.role, "content": m.content})));
            client.post(format!("{base}/api/chat")).json(&json!({
                "model": model,
                "messages": msgs,
                "stream": true,
                "options": {
                    "temperature": params.temperature,
                    "top_p": params.top_p,
                    "num_predict": params.max_tokens,
                },
            }))
        }
    };

    let mut attempt = 0;
    let mut resp = loop {
        let Some(req) = request.try_clone() else {
            return Err(StreamError::new("The request could not be prepared", Completion::default(), false));
        };
        let sent = tokio::select! {
            _ = cancel.cancelled() => return Err(StreamError::stopped(Completion::default(), false)),
            r = tokio::time::timeout(ep.idle_timeout, req.send()) => r,
        };
        let sent = match sent {
            Ok(r) => r,
            // The server accepted the connection but sent nothing back.
            Err(_) if attempt < MAX_RETRIES => {
                attempt += 1;
                continue;
            }
            Err(_) => {
                let msg = format!("No response for {} s; the server isn't answering", ep.idle_timeout.as_secs());
                return Err(StreamError::new(msg, Completion::default(), false));
            }
        };
        let wait = match sent {
            Ok(r) if r.status().is_success() => break r,
            Ok(r) if is_retryable(r.status()) && attempt < MAX_RETRIES => match retry_after(&r) {
                Some(w) if w > MAX_RETRY_WAIT => return Err(http_error(r).await),
                Some(w) => w,
                None => Duration::from_secs(1 << attempt),
            },
            Ok(r) => return Err(http_error(r).await),
            // Connection refused is not retried: the server isn't there, and waiting won't help.
            Err(e) if e.is_timeout() && attempt < MAX_RETRIES => Duration::from_secs(1 << attempt),
            Err(e) => return Err(StreamError::new(connect_error(ep, e), Completion::default(), false)),
        };
        attempt += 1;
        tokio::select! {
            _ = cancel.cancelled() => return Err(StreamError::stopped(Completion::default(), false)),
            _ = tokio::time::sleep(wait) => {}
        }
    };

    let mut out = Completion::default();
    let mut buf: Vec<u8> = Vec::new();
    let mut handle_line = |line: &[u8], out: &mut Completion| -> Result<(), String> {
        let line = String::from_utf8_lossy(line);
        if let Some(delta) = parse_line(ep.protocol, line.trim(), out)? {
            if out.ttft_ms.is_none() && !delta.is_empty() {
                out.ttft_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
            }
            out.text.push_str(&delta);
            on_delta(&delta);
        }
        Ok(())
    };
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(StreamError::stopped(out, true)),
            r = tokio::time::timeout(ep.idle_timeout, resp.chunk()) => r,
        };
        let chunk = match next {
            Err(_) => {
                let msg = format!("No data for {} s, so the reply was abandoned", ep.idle_timeout.as_secs());
                return Err(StreamError::new(msg, out, true));
            }
            Ok(Err(e)) => return Err(StreamError::new(format!("Connection lost mid-reply: {e}"), out, true)),
            Ok(Ok(None)) => break,
            Ok(Ok(Some(c))) => c,
        };
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            if let Err(e) = handle_line(&line, &mut out) {
                return Err(StreamError::new(e, out, true));
            }
        }
    }
    // A last line without a trailing newline.
    if !buf.is_empty() {
        if let Err(e) = handle_line(&buf, &mut out) {
            return Err(StreamError::new(e, out, true));
        }
    }
    if attempt > 0 {
        let retried = format!("Retried {attempt} time(s) after a rate limit or server error");
        out.note = Some(out.note.map_or(retried.clone(), |n| format!("{n}. {retried}")));
    }
    Ok(out)
}

fn is_retryable(status: StatusCode) -> bool {
    // 529 is Anthropic's "overloaded".
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504 | 529)
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs = resp.headers().get("retry-after")?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(secs))
}

async fn http_error(resp: reqwest::Response) -> Box<StreamError> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    StreamError::new(format!("{status}: {}", api_error_message(&body)), Completion::default(), false)
}

fn connect_error(ep: &Endpoint, e: reqwest::Error) -> String {
    if e.is_connect() {
        format!("Can't reach {} — is the server running?", ep.base_url)
    } else if e.is_timeout() {
        "Request timed out".to_string()
    } else {
        e.to_string()
    }
}

/// Pulls a readable message out of a provider error body.
fn api_error_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let msg = parsed.as_ref().and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .or_else(|| v["message"].as_str())
            .map(str::to_string)
    });
    msg.unwrap_or_else(|| body.chars().take(300).collect())
}

/// Parses one stream line; returns any text it carried.
pub(crate) fn parse_line(protocol: Protocol, line: &str, out: &mut Completion) -> Result<Option<String>, String> {
    if line.is_empty() {
        return Ok(None);
    }
    let payload = match protocol {
        Protocol::Ollama => line,
        _ => match line.strip_prefix("data:") {
            Some(d) => d.trim(),
            None => return Ok(None), // `event:` lines, comments
        },
    };
    if payload == "[DONE]" {
        return Ok(None);
    }
    // Keep-alives and other non-JSON lines carry nothing; skip them rather than fail the reply.
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        return Ok(None);
    };
    let usize_at = |v: &Value| v.as_u64().map(|n| n as usize);

    match protocol {
        Protocol::OpenAi => {
            if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                return Err(err["message"].as_str().or(err.as_str()).unwrap_or("stream error").to_string());
            }
            let usage = v.get("usage").filter(|u| !u.is_null()).or_else(|| v["x_groq"].get("usage"));
            if let Some(u) = usage {
                out.prompt_tokens = usize_at(&u["prompt_tokens"]).or(out.prompt_tokens);
                out.completion_tokens = usize_at(&u["completion_tokens"]).or(out.completion_tokens);
            }
            if v["choices"][0]["finish_reason"] == "length" {
                out.note = Some("Stopped at the max tokens limit".into());
            }
            Ok(v["choices"][0]["delta"]["content"].as_str().map(str::to_string))
        }
        Protocol::Anthropic => match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                out.prompt_tokens = usize_at(&v["message"]["usage"]["input_tokens"]);
                Ok(None)
            }
            "content_block_start" if v["content_block"]["type"] == "fallback" => {
                let b = &v["content_block"];
                out.note = Some(format!(
                    "{} declined; answered by {}",
                    b["from"]["model"].as_str().unwrap_or("model"),
                    b["to"]["model"].as_str().unwrap_or("fallback model"),
                ));
                Ok(None)
            }
            "content_block_delta" if v["delta"]["type"] == "text_delta" => {
                Ok(v["delta"]["text"].as_str().map(str::to_string))
            }
            "message_delta" => {
                out.completion_tokens = usize_at(&v["usage"]["output_tokens"]).or(out.completion_tokens);
                match v["delta"]["stop_reason"].as_str() {
                    Some("refusal") => out.note = Some("The model declined this request (refusal)".into()),
                    Some("max_tokens") => out.note = Some("Stopped at the max tokens limit".into()),
                    _ => {}
                }
                Ok(None)
            }
            "error" => Err(v["error"]["message"].as_str().unwrap_or("stream error").to_string()),
            _ => Ok(None),
        },
        Protocol::Gemini => {
            if let Some(u) = v.get("usageMetadata") {
                out.prompt_tokens = usize_at(&u["promptTokenCount"]).or(out.prompt_tokens);
                out.completion_tokens = usize_at(&u["candidatesTokenCount"]).or(out.completion_tokens);
            }
            if v["candidates"][0]["finishReason"] == "MAX_TOKENS" {
                out.note = Some("Stopped at the max tokens limit".into());
            }
            let text: String = v["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect())
                .unwrap_or_default();
            Ok(Some(text).filter(|t| !t.is_empty()))
        }
        Protocol::Ollama => {
            if let Some(err) = v["error"].as_str() {
                return Err(err.to_string());
            }
            if v["done"] == true {
                out.prompt_tokens = usize_at(&v["prompt_eval_count"]);
                out.completion_tokens = usize_at(&v["eval_count"]);
                if v["done_reason"] == "length" {
                    out.note = Some("Stopped at the max tokens limit".into());
                }
            }
            Ok(v["message"]["content"].as_str().map(str::to_string))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(protocol: Protocol, lines: &[&str]) -> (String, Completion) {
        let mut out = Completion::default();
        let mut text = String::new();
        for l in lines {
            if let Some(d) = parse_line(protocol, l, &mut out).unwrap() {
                text.push_str(&d);
            }
        }
        (text, out)
    }

    #[test]
    fn openai_stream() {
        let (text, c) = feed(
            Protocol::OpenAi,
            &[
                r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
                r#"data: {"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
                "data: [DONE]",
            ],
        );
        assert_eq!(text, "Hello");
        assert_eq!((c.prompt_tokens, c.completion_tokens), (Some(5), Some(2)));
    }

    #[test]
    fn anthropic_stream() {
        let (text, c) = feed(
            Protocol::Anthropic,
            &[
                "event: message_start",
                r#"data: {"type":"message_start","message":{"usage":{"input_tokens":9}}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
            ],
        );
        assert_eq!(text, "Hi");
        assert_eq!((c.prompt_tokens, c.completion_tokens), (Some(9), Some(3)));
        assert!(c.note.is_none());
    }

    #[test]
    fn anthropic_fallback_and_error() {
        let (_, c) = feed(
            Protocol::Anthropic,
            &[
                r#"data: {"type":"content_block_start","content_block":{"type":"fallback","from":{"model":"a"},"to":{"model":"b"}}}"#,
            ],
        );
        assert_eq!(c.note.as_deref(), Some("a declined; answered by b"));
        let mut c = Completion::default();
        let err = parse_line(Protocol::Anthropic, r#"data: {"type":"error","error":{"message":"overloaded"}}"#, &mut c);
        assert_eq!(err.unwrap_err(), "overloaded");
    }

    #[test]
    fn gemini_stream() {
        let (text, c) = feed(
            Protocol::Gemini,
            &[
                r#"data: {"candidates":[{"content":{"parts":[{"text":"A"},{"text":"B"}]}}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":2}}"#,
            ],
        );
        assert_eq!(text, "AB");
        assert_eq!((c.prompt_tokens, c.completion_tokens), (Some(4), Some(2)));
    }

    #[test]
    fn ollama_stream() {
        let (text, c) = feed(
            Protocol::Ollama,
            &[
                r#"{"message":{"role":"assistant","content":"Yo"},"done":false}"#,
                r#"{"message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":7,"eval_count":1}"#,
            ],
        );
        assert_eq!(text, "Yo");
        assert_eq!((c.prompt_tokens, c.completion_tokens), (Some(7), Some(1)));
    }

    use super::test_server::*;

    fn endpoint(base_url: String) -> Endpoint {
        Endpoint {
            provider_id: "custom".into(),
            protocol: Protocol::OpenAi,
            base_url,
            api_key: None,
            idle_timeout: Duration::from_millis(300),
        }
    }

    async fn run(base: String, cancel: &CancellationToken) -> Result<Completion, Box<StreamError>> {
        let client = Client::new();
        let messages = [Message::user("hi")];
        let prompt = Prompt { system: "", messages: &messages, params: &Params::default() };
        stream_chat(&client, &endpoint(base), "m", &prompt, cancel, |_| {}).await
    }

    #[tokio::test]
    async fn keepalive_and_junk_lines_are_ignored() {
        let body =
            format!("{:x}\r\n: ping\n\ndata:\n\ndata: not-json\n\n\r\n", ": ping\n\ndata:\n\ndata: not-json\n\n".len());
        let base = serve(vec![vec![
            Step::Send(OK_HEAD.into()),
            Step::Send(body),
            Step::Send(sse("Hi")),
            Step::Send(END.into()),
        ]])
        .await;
        let out = run(base, &CancellationToken::new()).await.unwrap();
        assert_eq!(out.text, "Hi");
    }

    #[tokio::test]
    async fn rate_limit_is_retried_once_with_clean_output() {
        let limited = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 2\r\n\r\n{}";
        let base = serve(vec![
            vec![Step::Send(limited.into())],
            vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Hi")), Step::Send(END.into())],
        ])
        .await;
        let out = run(base, &CancellationToken::new()).await.unwrap();
        assert_eq!(out.text, "Hi");
        assert!(out.note.unwrap().contains("Retried 1 time"));
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let body = r#"{"error":{"message":"bad model"}}"#;
        let bad = format!("HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        // Only one scripted connection: a retry would hang until the test times out.
        let base = serve(vec![vec![Step::Send(bad)]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert!(err.message.contains("bad model"));
        assert!(!err.started && !err.cancelled);
    }

    #[tokio::test]
    async fn stalled_stream_times_out_and_keeps_partial_text() {
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Hel")), Step::Wait(3000)]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.partial.text, "Hel");
        assert!(err.started && !err.cancelled);
        assert!(err.message.contains("No data"));
    }

    #[tokio::test]
    async fn dropped_connection_keeps_partial_text() {
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Hel"))]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.partial.text, "Hel");
        assert!(err.started && !err.cancelled);
    }

    #[tokio::test]
    async fn stop_keeps_partial_text() {
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Hel")), Step::Wait(3000)]]).await;
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            trigger.cancel();
        });
        let err = run(base, &cancel).await.unwrap_err();
        assert!(err.cancelled && err.started);
        assert_eq!(err.partial.text, "Hel");
    }

    /// Sends one request with a system prompt and returns (request line, headers, body).
    async fn captured(
        protocol: Protocol,
        provider_id: &str,
        model: &str,
        api_key: Option<&str>,
    ) -> (String, Vec<(String, String)>, Value) {
        let (base, requests) = serve_recording(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(END.into())]]).await;
        let ep = Endpoint {
            provider_id: provider_id.into(),
            protocol,
            base_url: base,
            api_key: api_key.map(str::to_string),
            idle_timeout: Duration::from_secs(5),
        };
        let messages = [Message::user("hi"), Message::assistant("hello"), Message::user("again")];
        let prompt = Prompt { system: "SYS", messages: &messages, params: &Params::default() };
        stream_chat(&Client::new(), &ep, model, &prompt, &CancellationToken::new(), |_| {}).await.unwrap();
        let raw = requests.lock().unwrap()[0].clone();
        parse_request(&raw)
    }

    #[tokio::test]
    async fn openai_request_shape() {
        let (line, headers, body) = captured(Protocol::OpenAi, "openai", "gpt-4o", Some("k1")).await;
        assert!(line.starts_with("POST /v1/chat/completions"), "{line}");
        assert_eq!(header(&headers, "authorization"), Some("Bearer k1"));
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "SYS"}));
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn openai_compatible_servers_get_max_tokens_and_no_key() {
        let (_, headers, body) = captured(Protocol::OpenAi, "custom", "local", None).await;
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(header(&headers, "authorization"), None);
        // Mistral rejects unknown fields, so it gets no stream_options.
        let (_, _, mistral) = captured(Protocol::OpenAi, "mistral", "mistral-small-latest", Some("k")).await;
        assert!(mistral.get("stream_options").is_none());
    }

    #[tokio::test]
    async fn anthropic_request_shape() {
        let (line, headers, body) = captured(Protocol::Anthropic, "anthropic", "claude-haiku-5-5", Some("k2")).await;
        assert!(line.starts_with("POST /v1/messages"), "{line}");
        assert_eq!(header(&headers, "x-api-key"), Some("k2"));
        assert_eq!(header(&headers, "anthropic-version"), Some("2023-06-01"));
        assert_eq!(body["system"], "SYS");
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("temperature").is_none() && body.get("top_p").is_none());
        assert_eq!(body["messages"][0], json!({"role": "user", "content": "hi"}));
        assert!(body.get("fallbacks").is_none());
        assert_eq!(header(&headers, "anthropic-beta"), None);
    }

    #[tokio::test]
    async fn newest_anthropic_models_get_server_side_fallback() {
        let (_, headers, body) = captured(Protocol::Anthropic, "anthropic", "claude-opus-5-5", Some("k")).await;
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(header(&headers, "anthropic-beta"), Some("server-side-fallback-2026-07-01"));
    }

    #[tokio::test]
    async fn gemini_request_shape() {
        let (line, headers, body) = captured(Protocol::Gemini, "gemini", "gemini-2.5-flash", Some("k3")).await;
        assert!(line.contains("/models/gemini-2.5-flash:streamGenerateContent?alt=sse"), "{line}");
        assert!(!line.contains("k3"), "the key must not be in the URL");
        assert_eq!(header(&headers, "x-goog-api-key"), Some("k3"));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "SYS");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
    }

    #[tokio::test]
    async fn ollama_request_shape() {
        let (line, _, body) = captured(Protocol::Ollama, "ollama", "llama3", None).await;
        assert!(line.starts_with("POST /v1/api/chat"), "{line}");
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "SYS"}));
        assert_eq!(body["options"]["num_predict"], 4096);
        assert_eq!(body["stream"], true);
    }

    #[tokio::test]
    async fn empty_system_prompt_is_not_sent() {
        let (base, requests) = serve_recording(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(END.into())]]).await;
        let ep = endpoint(base);
        let messages = [Message::user("hi")];
        let prompt = Prompt { system: "  ", messages: &messages, params: &Params::default() };
        stream_chat(&Client::new(), &ep, "m", &prompt, &CancellationToken::new(), |_| {}).await.unwrap();
        let (_, _, body) = parse_request(&requests.lock().unwrap()[0]);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stop_before_reply_is_not_started() {
        let base = serve(vec![vec![Step::Wait(2000)]]).await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = run(base, &cancel).await.unwrap_err();
        assert!(err.cancelled && !err.started);
    }

    #[tokio::test]
    async fn last_line_without_newline_is_read() {
        let data = "data: {\"choices\":[{\"delta\":{\"content\":\"tail\"}}]}";
        let chunk = format!("{:x}\r\n{data}\r\n", data.len());
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(chunk), Step::Send(END.into())]]).await;
        assert_eq!(run(base, &CancellationToken::new()).await.unwrap().text, "tail");
    }

    #[tokio::test]
    async fn provider_error_event_mid_stream_keeps_partial_text() {
        let err_line = "data: {\"error\":{\"message\":\"overloaded\"}}\n\n";
        let chunk = format!("{:x}\r\n{err_line}\r\n", err_line.len());
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Part")), Step::Send(chunk)]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert_eq!(err.message, "overloaded");
        assert_eq!(err.partial.text, "Part");
    }

    #[tokio::test]
    async fn connection_refused_is_reported_without_retry() {
        let started = std::time::Instant::now();
        // Windows takes ~2 s to refuse a loopback connection, so use a realistic idle limit.
        let ep = Endpoint { idle_timeout: Duration::from_secs(30), ..endpoint("http://127.0.0.1:9/v1".into()) };
        let messages = [Message::user("hi")];
        let prompt = Prompt { system: "", messages: &messages, params: &Params::default() };
        let err = stream_chat(&Client::new(), &ep, "m", &prompt, &CancellationToken::new(), |_| {}).await.unwrap_err();
        assert!(err.message.contains("Can't reach"), "{}", err.message);
        assert!(started.elapsed() < Duration::from_secs(5), "should fail fast, not back off");
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        // Three silent connections: the first try plus two retries.
        let base = serve(vec![vec![Step::Wait(3000)], vec![Step::Wait(3000)], vec![Step::Wait(3000)]]).await;
        let started = std::time::Instant::now();
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert!(err.message.contains("No response"), "{}", err.message);
        assert!(!err.started);
        assert!(started.elapsed() < Duration::from_secs(3), "gave up after the idle limit, not the server");
    }

    #[tokio::test]
    async fn retries_give_up_after_two_attempts() {
        let busy = json_response("503 Service Unavailable", r#"{"error":{"message":"busy"}}"#).replacen(
            "\r\n",
            "\r\nRetry-After: 0\r\n",
            1,
        );
        let base =
            serve(vec![vec![Step::Send(busy.clone())], vec![Step::Send(busy.clone())], vec![Step::Send(busy)]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert!(err.message.contains("503") && err.message.contains("busy"), "{}", err.message);
    }

    #[tokio::test]
    async fn long_retry_after_fails_with_the_providers_message() {
        let limited = json_response("429 Too Many Requests", r#"{"error":{"message":"slow down"}}"#).replacen(
            "\r\n",
            "\r\nRetry-After: 3600\r\n",
            1,
        );
        let base = serve(vec![vec![Step::Send(limited)]]).await;
        let err = run(base, &CancellationToken::new()).await.unwrap_err();
        assert!(err.message.contains("slow down"));
    }

    #[test]
    fn null_error_fields_are_not_errors() {
        let mut out = Completion::default();
        let line = r#"data: {"error":null,"choices":[{"delta":{"content":"ok"}}]}"#;
        assert_eq!(parse_line(Protocol::OpenAi, line, &mut out).unwrap().as_deref(), Some("ok"));
        let line = r#"data: {"error":"plain text error"}"#;
        assert_eq!(parse_line(Protocol::OpenAi, line, &mut out).unwrap_err(), "plain text error");
    }

    #[test]
    fn truncation_and_usage_notes() {
        let mut out = Completion::default();
        parse_line(Protocol::OpenAi, r#"data: {"choices":[{"delta":{},"finish_reason":"length"}],"x_groq":{"usage":{"prompt_tokens":3,"completion_tokens":4}}}"#, &mut out).unwrap();
        assert_eq!(out.note.as_deref(), Some("Stopped at the max tokens limit"));
        assert_eq!((out.prompt_tokens, out.completion_tokens), (Some(3), Some(4)));
        let mut out = Completion::default();
        parse_line(
            Protocol::Anthropic,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{"output_tokens":1}}"#,
            &mut out,
        )
        .unwrap();
        assert!(out.note.unwrap().contains("declined"));
        let mut out = Completion::default();
        parse_line(
            Protocol::Gemini,
            r#"data: {"candidates":[{"finishReason":"MAX_TOKENS","content":{"parts":[]}}]}"#,
            &mut out,
        )
        .unwrap();
        assert_eq!(out.note.as_deref(), Some("Stopped at the max tokens limit"));
        let mut out = Completion::default();
        parse_line(Protocol::Ollama, r#"{"message":{"content":""},"done":true,"done_reason":"length"}"#, &mut out)
            .unwrap();
        assert_eq!(out.note.as_deref(), Some("Stopped at the max tokens limit"));
    }

    #[test]
    fn error_body_variants() {
        assert_eq!(api_error_message(r#"{"error":"flat"}"#), "flat");
        assert_eq!(api_error_message(r#"{"message":"top level"}"#), "top level");
        assert_eq!(api_error_message(&"x".repeat(1000)).len(), 300);
    }

    #[test]
    fn retryable_statuses() {
        for code in [429, 500, 502, 503, 504, 529] {
            assert!(is_retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
        for code in [400, 401, 403, 404, 422] {
            assert!(!is_retryable(StatusCode::from_u16(code).unwrap()), "{code}");
        }
    }

    #[test]
    fn error_body_message() {
        assert_eq!(api_error_message(r#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(api_error_message("plain"), "plain");
    }
}

/// A scripted HTTP server for testing streaming without a real provider.
#[cfg(test)]
pub(crate) mod test_server {
    use std::time::Duration;

    pub enum Step {
        Send(String),
        Wait(u64),
    }

    pub fn sse(text: &str) -> String {
        let data = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}}}]}}\n\n");
        format!("{:x}\r\n{data}\r\n", data.len())
    }

    pub const OK_HEAD: &str =
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
    pub const END: &str = "0\r\n\r\n";

    /// Raw requests the server received, in order.
    pub type Requests = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// Serves one scripted response per connection; returns the base URL.
    pub async fn serve(scripts: Vec<Vec<Step>>) -> String {
        serve_recording(scripts).await.0
    }

    /// Like [`serve`], and also records each raw request (headers and body).
    pub async fn serve_recording(scripts: Vec<Vec<Step>>) -> (String, Requests) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Requests::default();
        let seen = requests.clone();
        tokio::spawn(async move {
            for script in scripts {
                let (mut sock, _) = listener.accept().await.unwrap();
                // Read the request headers and body before answering.
                let mut req = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap();
                    req.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&req);
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string())
                            })
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(0);
                        if req.len() >= head_end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                seen.lock().unwrap().push(String::from_utf8_lossy(&req).into_owned());
                for step in script {
                    match step {
                        Step::Send(bytes) => {
                            sock.write_all(bytes.as_bytes()).await.unwrap();
                            sock.flush().await.unwrap();
                        }
                        Step::Wait(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                    }
                }
                // Dropping the socket closes the connection.
            }
        });
        (format!("http://{addr}/v1"), requests)
    }

    /// A complete non-streaming JSON response.
    pub fn json_response(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    /// Splits a raw request into (request line, lower-cased headers, JSON body).
    pub fn parse_request(raw: &str) -> (String, Vec<(String, String)>, serde_json::Value) {
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
        let mut lines = head.lines();
        let line = lines.next().unwrap_or_default().to_string();
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        (line, headers, serde_json::from_str(body).unwrap_or(serde_json::Value::Null))
    }

    pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}
