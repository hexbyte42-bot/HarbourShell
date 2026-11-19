//! The provider layer: one trait, chosen by name from the config.

use std::sync::Arc;

use super::anthropic::AnthropicProvider;
use super::config::{resolve_key, AgentConfig};
use super::http::{CurlTransport, Transport};
use super::openai::OpenAiProvider;
use super::types::{ChatRequest, Message, ProviderEvent, Role, ToolCall};

pub trait Provider: Send + Sync {
    /// Shown in the status row, e.g. `groq/llama-3.3-70b`.
    fn label(&self) -> String;
    /// Ask once. Events are reported through `emit`: `TextDelta` and `ToolCall` as they are known,
    /// then exactly one `Done` or `Error`. (Wire formats answer whole; a streaming one only has to
    /// call `emit` earlier.) Blocking: run it on a worker thread.
    fn chat(&self, req: &ChatRequest, emit: &mut dyn FnMut(ProviderEvent));
}

/// Build the provider called `name`. `kind`s: `openai`, `anthropic`, `muse`, `mock`.
pub fn build(cfg: &AgentConfig, name: &str, model_override: Option<&str>) -> Result<Arc<dyn Provider>, String> {
    build_with(cfg, name, model_override, Arc::new(CurlTransport::default()), &|n| std::env::var(n).ok())
}

pub fn build_with(
    cfg: &AgentConfig,
    name: &str,
    model_override: Option<&str>,
    transport: Arc<dyn Transport>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Arc<dyn Provider>, String> {
    let p = cfg.providers.get(name).ok_or_else(|| format!("no provider called \"{name}\""))?;
    let model = model_override.map(str::to_string).or_else(|| p.model.clone());
    match p.kind.as_str() {
        "mock" => Ok(Arc::new(MockProvider)),
        "openai" => {
            let base = p.base_url.clone().ok_or_else(|| format!("{name}: base_url is missing"))?;
            let model = model.ok_or_else(|| format!("{name}: model is missing"))?;
            let key = resolve_key(p, env).map_err(|e| format!("{name}: {e}"))?;
            Ok(Arc::new(OpenAiProvider::new(name, &base, &model, key, p.max_tokens, transport)))
        }
        "anthropic" => {
            let base = p.base_url.clone().unwrap_or_else(|| "https://api.anthropic.com".into());
            let model = model.ok_or_else(|| format!("{name}: model is missing"))?;
            let key = resolve_key(p, env).map_err(|e| format!("{name}: {e}"))?.ok_or_else(|| format!("{name}: an API key is required"))?;
            Ok(Arc::new(AnthropicProvider::new(name, &base, &model, key, p.max_tokens, transport)))
        }
        "muse" => super::muse::provider(name, p),
        other => Err(format!("{name}: unknown kind \"{other}\"")),
    }
}

/// Canned answers for headless runs and tests: a tool call for the first request about an app,
/// a plain answer after a tool result.
pub struct MockProvider;

impl Provider for MockProvider {
    fn label(&self) -> String {
        "mock".into()
    }

    fn chat(&self, req: &ChatRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        let last = req.messages.last();
        if last.map(|m| m.role) == Some(Role::Tool) {
            let l = last.unwrap();
            emit(ProviderEvent::TextDelta(if l.is_error {
                "That did not work. (mock)".into()
            } else {
                "Done. This is the mock provider: no model was asked.".into()
            }));
            emit(ProviderEvent::Done);
            return;
        }
        let q = last.map(|m| m.text.to_lowercase()).unwrap_or_default();
        if q.contains("slow") {
            // For screenshots of the busy state.
            std::thread::sleep(std::time::Duration::from_millis(4000));
        }
        if q.contains("error") {
            emit(ProviderEvent::Error("network error: Could not resolve host: api.example.com".into()));
            return;
        }
        let call = |name: &str, args: serde_json::Value| ToolCall { id: "mock_1".into(), name: name.into(), args };
        if q.contains("open") || q.contains("launch") {
            emit(ProviderEvent::TextDelta("Sure, opening Files.".into()));
            emit(ProviderEvent::ToolCall(call("launch_app", serde_json::json!({"name": "Files"}))));
        } else if q.contains("apps") {
            emit(ProviderEvent::TextDelta("Let me look.".into()));
            emit(ProviderEvent::ToolCall(call("list_apps", serde_json::json!({}))));
        } else {
            emit(ProviderEvent::TextDelta("Hello! I am the mock agent. Try \"open files\" or \"list apps\".".into()));
        }
        emit(ProviderEvent::Done);
    }
}

/// Collect a provider's events into (text, tool calls, error): handy for tests and the worker.
pub fn collect(p: &dyn Provider, req: &ChatRequest) -> Result<(String, Vec<ToolCall>), String> {
    let (mut text, mut calls, mut err) = (String::new(), vec![], None);
    p.chat(req, &mut |e| match e {
        ProviderEvent::TextDelta(t) => text.push_str(&t),
        ProviderEvent::ToolCall(c) => calls.push(c),
        ProviderEvent::Done => {}
        ProviderEvent::Error(e) => err = Some(e),
    });
    match err {
        Some(e) => Err(e),
        None => Ok((text, calls)),
    }
}

/// Used by the encoders: the messages a provider may see (no notices).
pub fn wire_messages(messages: &[Message]) -> Vec<&Message> {
    messages.iter().filter(|m| m.role != Role::Notice).collect()
}

/// Cap an error body to one short plain line for the status row.
pub fn short_error(body: &str, status: u16) -> String {
    let v: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let msg = v
        .as_ref()
        .and_then(|v| v.pointer("/error/message").or_else(|| v.pointer("/message")).or_else(|| v.pointer("/error")))
        .and_then(|m| m.as_str().map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(120).collect());
    let hint = match status {
        401 | 403 => " (check the API key)",
        429 => " (rate limited, try again later)",
        _ => "",
    };
    format!("HTTP {status}: {}{hint}", msg.chars().take(160).collect::<String>())
}
