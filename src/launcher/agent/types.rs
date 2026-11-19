//! Provider-independent conversation types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    /// The result of one tool call (`Message::tool_call_id` says which).
    Tool,
    /// Shown in the transcript only (errors, "new chat"); never sent to a provider.
    Notice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(default)]
    pub text: String,
    /// Assistant messages: the tools it wants run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Tool messages: the call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Message {
    pub fn user(text: &str) -> Self {
        Self { role: Role::User, text: text.into(), tool_calls: vec![], tool_call_id: None, is_error: false }
    }
    pub fn assistant(text: &str, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: Role::Assistant, text: text.into(), tool_calls, tool_call_id: None, is_error: false }
    }
    pub fn tool(id: &str, text: &str, is_error: bool) -> Self {
        Self { role: Role::Tool, text: text.into(), tool_calls: vec![], tool_call_id: Some(id.into()), is_error }
    }
    pub fn notice(text: &str) -> Self {
        Self { role: Role::Notice, text: text.into(), tool_calls: vec![], tool_call_id: None, is_error: false }
    }
}

/// A tool as the model sees it. `schema` is a JSON Schema object for the arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

#[derive(Clone, Debug)]
pub struct ChatOpts {
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl Default for ChatOpts {
    fn default() -> Self {
        Self { max_tokens: 1024, temperature: None }
    }
}

/// Everything one provider call needs.
pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
    pub opts: &'a ChatOpts,
}

/// What a provider reports while (or after) it answers.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEvent {
    TextDelta(String),
    ToolCall(ToolCall),
    Done,
    Error(String),
}

/// Keep the last `n` messages for sending, never starting on a tool result or an assistant
/// message that is cut off from its user turn: the window starts at a user message.
pub fn truncate_for_send(messages: &[Message], n: usize) -> Vec<Message> {
    let msgs: Vec<&Message> = messages.iter().filter(|m| m.role != Role::Notice).collect();
    if msgs.len() <= n {
        return msgs.into_iter().cloned().collect();
    }
    let mut start = msgs.len() - n;
    while start < msgs.len() && msgs[start].role != Role::User {
        start += 1;
    }
    if start == msgs.len() {
        // No user message in the window: fall back to the last user message overall.
        start = msgs.iter().rposition(|m| m.role == Role::User).unwrap_or(0);
    }
    msgs[start..].iter().map(|m| (*m).clone()).collect()
}
