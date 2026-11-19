//! Anthropic's native Messages API: `POST /v1/messages` with `x-api-key`, tools with
//! `input_schema`, `tool_use` / `tool_result` blocks.

use std::sync::Arc;

use serde_json::{json, Value};

use super::http::Transport;
use super::provider::{short_error, wire_messages, Provider};
use super::types::{ChatRequest, Message, ProviderEvent, Role, ToolCall};

pub const VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    name: String,
    base: String,
    model: String,
    key: String,
    max_tokens: Option<u32>,
    transport: Arc<dyn Transport>,
}

impl AnthropicProvider {
    pub fn new(name: &str, base: &str, model: &str, key: String, max_tokens: Option<u32>, transport: Arc<dyn Transport>) -> Self {
        Self { name: name.into(), base: base.trim_end_matches('/').into(), model: model.into(), key, max_tokens, transport }
    }
}

/// Anthropic wants tool results as `tool_result` blocks of a *user* message, right after the
/// assistant message that asked; consecutive Tool messages share one user message.
pub fn encode_messages(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    for m in wire_messages(messages) {
        match m.role {
            Role::User => out.push(json!({"role": "user", "content": [{"type": "text", "text": m.text}]})),
            Role::Assistant => {
                let mut blocks = vec![];
                if !m.text.is_empty() {
                    blocks.push(json!({"type": "text", "text": m.text}));
                }
                for c in &m.tool_calls {
                    blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.args}));
                }
                if !blocks.is_empty() {
                    out.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            Role::Tool => {
                let block = json!({"type": "tool_result", "tool_use_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.text, "is_error": m.is_error});
                let merge = out.last().map_or(false, |l| {
                    l["role"] == "user" && l["content"].as_array().map_or(false, |a| a.iter().all(|b| b["type"] == "tool_result"))
                });
                if merge {
                    out.last_mut().unwrap()["content"].as_array_mut().unwrap().push(block);
                } else {
                    out.push(json!({"role": "user", "content": [block]}));
                }
            }
            Role::Notice => {}
        }
    }
    out
}

pub fn encode_request(model: &str, max_tokens: u32, req: &ChatRequest) -> Value {
    let mut body = json!({"model": model, "max_tokens": max_tokens, "system": req.system, "messages": encode_messages(req.messages)});
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema})).collect());
    }
    if let Some(t) = req.opts.temperature {
        body["temperature"] = json!(t);
    }
    body
}

pub fn decode_response(body: &str) -> Result<(String, Vec<ToolCall>), String> {
    let v: Value = serde_json::from_str(body).map_err(|_| format!("unexpected reply: {}", body.trim().chars().take(100).collect::<String>()))?;
    if v.get("type").and_then(Value::as_str) == Some("error") {
        return Err(v.pointer("/error/message").and_then(Value::as_str).unwrap_or("error").to_string());
    }
    let blocks = v.get("content").and_then(Value::as_array).ok_or("the reply has no content")?;
    let (mut text, mut calls) = (String::new(), vec![]);
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(b.get("text").and_then(Value::as_str).unwrap_or_default()),
            Some("tool_use") => calls.push(ToolCall {
                id: b.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: b.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                args: b.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            _ => {}
        }
    }
    if v.get("stop_reason").and_then(Value::as_str) == Some("max_tokens") && calls.is_empty() {
        text.push_str(" [cut off: max_tokens]");
    }
    Ok((text, calls))
}

impl Provider for AnthropicProvider {
    fn label(&self) -> String {
        format!("{}/{}", self.name, self.model)
    }

    fn chat(&self, req: &ChatRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        let body = encode_request(&self.model, self.max_tokens.unwrap_or(req.opts.max_tokens), req).to_string();
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("x-api-key".into(), self.key.clone()),
            ("anthropic-version".into(), VERSION.into()),
        ];
        let url = format!("{}/v1/messages", self.base);
        match self.transport.post(&url, &headers, &body) {
            Err(e) => emit(ProviderEvent::Error(e)),
            Ok(r) if !(200..300).contains(&r.status) => emit(ProviderEvent::Error(short_error(&r.body, r.status))),
            Ok(r) => match decode_response(&r.body) {
                Err(e) => emit(ProviderEvent::Error(e)),
                Ok((text, calls)) => {
                    if !text.is_empty() {
                        emit(ProviderEvent::TextDelta(text));
                    }
                    for c in calls {
                        emit(ProviderEvent::ToolCall(c));
                    }
                    emit(ProviderEvent::Done);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::agent::http::fake::FakeTransport;
    use crate::launcher::agent::provider::collect;
    use crate::launcher::agent::types::{ChatOpts, ToolSpec};

    // Recorded shape of a Messages reply that uses a tool.
    const TOOL_REPLY: &str = r#"{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"text","text":"Opening Files."},{"type":"tool_use","id":"toolu_01","name":"launch_app","input":{"name":"Files"}}],"stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":20}}"#;

    #[test]
    fn decodes_text_and_tool_use_blocks() {
        let (text, calls) = decode_response(TOOL_REPLY).unwrap();
        assert_eq!(text, "Opening Files.");
        assert_eq!(calls, vec![ToolCall { id: "toolu_01".into(), name: "launch_app".into(), args: json!({"name": "Files"}) }]);
        let e = decode_response(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#).unwrap_err();
        assert_eq!(e, "Overloaded");
    }

    #[test]
    fn tool_results_go_into_one_user_message() {
        let calls = vec![
            ToolCall { id: "a".into(), name: "list_apps".into(), args: json!({}) },
            ToolCall { id: "b".into(), name: "system_info".into(), args: json!({}) },
        ];
        let msgs = vec![Message::user("hi"), Message::assistant("x", calls), Message::tool("a", "r1", false), Message::tool("b", "boom", true)];
        let enc = encode_messages(&msgs);
        assert_eq!(enc.len(), 3);
        assert_eq!(enc[1]["content"][1], json!({"type":"tool_use","id":"a","name":"list_apps","input":{}}));
        let last = &enc[2];
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"].as_array().unwrap().len(), 2);
        assert_eq!(last["content"][0]["tool_use_id"], "a");
        assert_eq!(last["content"][1]["is_error"], true);
    }

    #[test]
    fn chat_sends_the_headers_and_the_input_schema() {
        let t = Arc::new(FakeTransport::new(vec![FakeTransport::ok(TOOL_REPLY)]));
        let p = AnthropicProvider::new("anthropic", "https://api.anthropic.com", "claude-sonnet-5-5", "SECRET".into(), None, t.clone());
        let msgs = [Message::user("open files")];
        let opts = ChatOpts::default();
        let tools = [ToolSpec { name: "launch_app", description: "d", schema: json!({"type":"object","properties":{}}) }];
        let (text, calls) = collect(&p, &ChatRequest { system: "sys", messages: &msgs, tools: &tools, opts: &opts }).unwrap();
        assert_eq!(text, "Opening Files.");
        assert_eq!(calls.len(), 1);
        let sent = t.sent.lock().unwrap();
        assert_eq!(sent[0].0, "https://api.anthropic.com/v1/messages");
        assert!(sent[0].1.contains(&("x-api-key".to_string(), "SECRET".to_string())));
        assert!(sent[0].1.contains(&("anthropic-version".to_string(), VERSION.to_string())));
        let body: Value = serde_json::from_str(&sent[0].2).unwrap();
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert!(!sent[0].2.contains("SECRET"));
    }

    #[test]
    fn a_missing_key_is_refused_when_building() {
        let cfg = crate::launcher::agent::config::AgentConfig::parse(r#"{"providers":{"a":{"kind":"anthropic","model":"m","api_key_env":"NOPE"}}}"#).unwrap();
        let r = crate::launcher::agent::provider::build_with(&cfg, "a", None, Arc::new(FakeTransport::new(vec![])), &|_| None);
        assert!(r.err().unwrap().contains("NOPE"));
    }
}
