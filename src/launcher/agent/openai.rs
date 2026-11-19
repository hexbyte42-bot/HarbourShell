//! OpenAI-compatible chat completions (OpenAI, Groq, Nous Portal, Z.ai GLM, local servers):
//! `POST {base_url}/chat/completions`, tools as function calling.

use std::sync::Arc;

use serde_json::{json, Value};

use super::http::Transport;
use super::provider::{short_error, wire_messages, Provider};
use super::types::{ChatRequest, Message, ProviderEvent, Role, ToolCall, ToolSpec};

pub struct OpenAiProvider {
    name: String,
    base: String,
    model: String,
    key: Option<String>,
    max_tokens: Option<u32>,
    transport: Arc<dyn Transport>,
}

impl OpenAiProvider {
    pub fn new(name: &str, base: &str, model: &str, key: Option<String>, max_tokens: Option<u32>, transport: Arc<dyn Transport>) -> Self {
        Self { name: name.into(), base: base.trim_end_matches('/').into(), model: model.into(), key, max_tokens, transport }
    }
}

fn tool_json(t: &ToolSpec) -> Value {
    json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}})
}

pub fn encode_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    for m in wire_messages(messages) {
        match m.role {
            Role::User => out.push(json!({"role": "user", "content": m.text})),
            Role::Assistant => {
                let mut v = json!({"role": "assistant", "content": if m.text.is_empty() { Value::Null } else { Value::String(m.text.clone()) }});
                if !m.tool_calls.is_empty() {
                    v["tool_calls"] = Value::Array(
                        m.tool_calls
                            .iter()
                            .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.args.to_string()}}))
                            .collect(),
                    );
                }
                out.push(v);
            }
            Role::Tool => out.push(json!({"role": "tool", "tool_call_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.text})),
            Role::Notice => {}
        }
    }
    out
}

/// The request body. `max_completion_tokens` is what api.openai.com expects; the compatible
/// servers still want `max_tokens`.
pub fn encode_request(base: &str, model: &str, max_tokens: u32, req: &ChatRequest) -> Value {
    let mut body = json!({"model": model, "messages": encode_messages(req.system, req.messages)});
    body[if base.contains("api.openai.com") { "max_completion_tokens" } else { "max_tokens" }] = json!(max_tokens);
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(tool_json).collect());
    }
    if let Some(t) = req.opts.temperature {
        body["temperature"] = json!(t);
    }
    body
}

/// Parse a response body into (text, tool calls).
pub fn decode_response(body: &str) -> Result<(String, Vec<ToolCall>), String> {
    let v: Value = serde_json::from_str(body).map_err(|_| format!("unexpected reply: {}", body.trim().chars().take(100).collect::<String>()))?;
    if let Some(e) = v.get("error") {
        let m = e.get("message").and_then(Value::as_str).or_else(|| e.as_str()).unwrap_or("error");
        return Err(m.to_string());
    }
    let msg = v.pointer("/choices/0/message").ok_or("the reply has no choices")?;
    let text = match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
        _ => String::new(),
    };
    let mut calls = vec![];
    if let Some(Value::Array(tc)) = msg.get("tool_calls") {
        for (i, c) in tc.iter().enumerate() {
            let f = c.get("function").ok_or("tool call without function")?;
            let name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = match f.get("arguments") {
                Some(Value::String(s)) if s.trim().is_empty() => json!({}),
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({"_raw": s})),
                Some(o @ Value::Object(_)) => o.clone(),
                _ => json!({}),
            };
            let id = c.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("call_{i}"));
            calls.push(ToolCall { id, name, args });
        }
    }
    Ok((text, calls))
}

impl Provider for OpenAiProvider {
    fn label(&self) -> String {
        format!("{}/{}", self.name, self.model)
    }

    fn chat(&self, req: &ChatRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        let body = encode_request(&self.base, &self.model, self.max_tokens.unwrap_or(req.opts.max_tokens), req).to_string();
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(k) = &self.key {
            headers.push(("Authorization".into(), format!("Bearer {k}")));
        }
        let url = format!("{}/chat/completions", self.base);
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
    use crate::launcher::agent::types::ChatOpts;

    // Recorded shape of a chat.completions reply with a tool call (Groq / OpenAI).
    const TOOL_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_9","type":"function","function":{"name":"launch_app","arguments":"{\"name\":\"Files\"}"}}]}}],"usage":{"total_tokens":42}}"#;
    const TEXT_REPLY: &str = r#"{"choices":[{"message":{"role":"assistant","content":"こんにちは"}}]}"#;

    fn tool() -> ToolSpec {
        ToolSpec { name: "launch_app", description: "d", schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}) }
    }

    #[test]
    fn decodes_a_tool_call_reply() {
        let (text, calls) = decode_response(TOOL_REPLY).unwrap();
        assert_eq!(text, "");
        assert_eq!(calls, vec![ToolCall { id: "call_9".into(), name: "launch_app".into(), args: json!({"name": "Files"}) }]);
    }

    #[test]
    fn decodes_text_and_errors() {
        assert_eq!(decode_response(TEXT_REPLY).unwrap().0, "こんにちは");
        let e = decode_response(r#"{"error":{"message":"bad key","type":"auth"}}"#).unwrap_err();
        assert_eq!(e, "bad key");
        assert!(decode_response("<html>").is_err());
    }

    #[test]
    fn encodes_the_tool_round_trip() {
        let msgs = vec![
            Message::user("open files"),
            Message::assistant("", vec![ToolCall { id: "call_9".into(), name: "launch_app".into(), args: json!({"name": "Files"}) }]),
            Message::tool("call_9", "ok", false),
            Message::notice("hidden"),
        ];
        let opts = ChatOpts::default();
        let tools = [tool()];
        let req = ChatRequest { system: "sys", messages: &msgs, tools: &tools, opts: &opts };
        let b = encode_request("https://api.groq.com/openai/v1", "m", 512, &req);
        assert_eq!(b["max_tokens"], 512);
        assert!(b.get("max_completion_tokens").is_none());
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m.len(), 4, "system + user + assistant + tool, the notice is dropped");
        assert_eq!(m[0], json!({"role":"system","content":"sys"}));
        assert_eq!(m[2]["content"], Value::Null);
        assert_eq!(m[2]["tool_calls"][0]["function"]["arguments"], "{\"name\":\"Files\"}");
        assert_eq!(m[3], json!({"role":"tool","tool_call_id":"call_9","content":"ok"}));
        assert_eq!(b["tools"][0]["function"]["parameters"]["required"][0], "name");
        let b = encode_request("https://api.openai.com/v1", "m", 512, &req);
        assert_eq!(b["max_completion_tokens"], 512);
    }

    #[test]
    fn chat_posts_to_the_right_url_with_a_bearer_key() {
        let t = Arc::new(FakeTransport::new(vec![FakeTransport::ok(TOOL_REPLY)]));
        let p = OpenAiProvider::new("groq", "https://api.groq.com/openai/v1/", "m", Some("KEY".into()), None, t.clone());
        let msgs = [Message::user("hi")];
        let opts = ChatOpts::default();
        let req = ChatRequest { system: "s", messages: &msgs, tools: &[], opts: &opts };
        let (text, calls) = collect(&p, &req).unwrap();
        assert!(text.is_empty());
        assert_eq!(calls[0].name, "launch_app");
        let sent = t.sent.lock().unwrap();
        assert_eq!(sent[0].0, "https://api.groq.com/openai/v1/chat/completions");
        assert!(sent[0].1.contains(&("Authorization".to_string(), "Bearer KEY".to_string())));
        assert!(!sent[0].2.contains("KEY"), "the key is only in a header");
    }

    #[test]
    fn http_errors_become_plain_words() {
        let t = Arc::new(FakeTransport::new(vec![Ok(crate::launcher::agent::http::HttpResponse { status: 401, body: r#"{"error":{"message":"Invalid API Key"}}"#.into() })]));
        let p = OpenAiProvider::new("x", "https://h/v1", "m", None, None, t);
        let msgs = [Message::user("hi")];
        let opts = ChatOpts::default();
        let e = collect(&p, &ChatRequest { system: "s", messages: &msgs, tools: &[], opts: &opts }).unwrap_err();
        assert!(e.contains("401") && e.contains("Invalid API Key") && e.contains("API key"), "{e}");
    }
}
