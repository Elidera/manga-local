//! OpenAI-compatible streaming client for llama-server (`/v1/chat/completions`).
//! Streaming lets us show text as it is generated and lets a superseded request
//! be cancelled simply by dropping the future (llama-server stops generating
//! when the connection closes).

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;
use manga_core::prompt::Message;

use crate::config::ModelPreset;

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base: String,
}

impl LlmClient {
    pub fn new(base: &str, timeout_s: u64) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_s.max(5)))
            .connect_timeout(Duration::from_secs(3))
            .no_proxy() // never route localhost traffic through a system proxy
            .build()
            .expect("reqwest client");
        Self { http, base: base.trim_end_matches('/').to_string() }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn health(&self) -> Result<bool> {
        let r = self.http.get(format!("{}/health", self.base)).timeout(Duration::from_secs(2)).send().await?;
        Ok(r.status().is_success())
    }

    /// Stream a chat completion. `on_delta` receives the text accumulated so far.
    pub async fn chat(&self, messages: &[Message], p: &ModelPreset, on_delta: impl FnMut(&str)) -> Result<String> {
        self.chat_within(messages, p, None, on_delta).await
    }

    /// `chat` with its own time limit (a long page batch on a processor needs
    /// more than the usual one).
    pub async fn chat_within(&self, messages: &[Message], p: &ModelPreset, timeout: Option<Duration>, on_delta: impl FnMut(&str)) -> Result<String> {
        self.chat_full(messages, p, timeout, None, on_delta).await
    }

    /// `schema`: the answer must be JSON of this shape (llama-server turns it
    /// into a grammar the model cannot step out of).
    pub async fn chat_full(&self, messages: &[Message], p: &ModelPreset, timeout: Option<Duration>, schema: Option<&Value>, on_delta: impl FnMut(&str)) -> Result<String> {
        // An unloaded model starts loading; wait for it like for the first start.
        crate::llm::server::touch();
        if crate::llm::server::asleep() {
            for _ in 0..240 {
                if !crate::llm::server::asleep() && self.health().await.unwrap_or(false) {
                    break;
                }
                // Asked again: a use that came right as the model was unloading still wakes it.
                crate::llm::server::touch();
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        let r = self.stream_chat(messages, p, timeout, schema, on_delta).await;
        // A long answer counts as use till its end.
        crate::llm::server::touch();
        r
    }

    async fn stream_chat(&self, messages: &[Message], p: &ModelPreset, timeout: Option<Duration>, schema: Option<&Value>, mut on_delta: impl FnMut(&str)) -> Result<String> {
        let msgs: Vec<Value> = messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect();
        let mut body = json!({
            "messages": msgs,
            "stream": true,
            "temperature": p.temperature,
            "top_p": p.top_p,
            "top_k": p.top_k,
            "repeat_penalty": p.repeat_penalty,
            "max_tokens": p.max_tokens,
            "cache_prompt": true,
        });
        if p.no_think {
            body["chat_template_kwargs"] = json!({"enable_thinking": false});
        }
        if let Some(schema) = schema {
            body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "page", "schema": schema}});
        }
        let url = format!("{}/v1/chat/completions", self.base);
        let post = || {
            // Our server's password; an external server without one ignores it.
            let r = self.http.post(&url).bearer_auth(crate::llm::server::api_key()).json(&body);
            match timeout {
                Some(t) => r.timeout(t),
                None => r,
            }
        };
        let resp = match post().send().await {
            Ok(r) => r,
            // llama-server may close a kept-alive connection right as we reuse it
            // ("error sending request" in logs): one quick retry on a fresh one.
            Err(e) if e.is_request() || e.is_connect() => {
                tracing::debug!("llama-server: повтор запроса после «{e}»");
                tokio::time::sleep(Duration::from_millis(60)).await;
                post().send().await?
            }
            Err(e) => return Err(e.into()),
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("llama-server {status}: {}", text.chars().take(300).collect::<String>()));
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk?);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim();
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(out);
                }
                let v: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("llama-server: {err}"));
                }
                if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                    if !delta.is_empty() {
                        out.push_str(delta);
                        on_delta(&out);
                    }
                }
            }
        }
        Ok(out)
    }
}
