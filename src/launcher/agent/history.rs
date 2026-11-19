//! Chats saved as JSON under `~/.local/share/harbour/agent/` (mode 0600), the last few kept,
//! plus a small `state.json` for the choices the menu makes (auto-run, provider).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{Message, Role};

/// Messages sent to the model per request.
pub const SEND_WINDOW: usize = 40;
/// Chat files kept on disk.
pub const KEEP_CHATS: usize = 8;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Chat {
    pub id: String,
    pub provider: String,
    pub messages: Vec<Message>,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct State {
    /// The Agent can be switched off in Settings (default on).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// What the last provider call said, one short plain line (never a key or a URL).
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub auto_run: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

pub fn new_id(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y%m%d-%H%M%S").to_string()
}

pub fn chat_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("chat-{id}.json"))
}

/// Save (notices are kept: they are part of what the screen showed). Old chats beyond
/// [`KEEP_CHATS`] are removed.
pub fn save(dir: &Path, chat: &Chat) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string(chat).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    write_private(&chat_path(dir, &chat.id), &text)?;
    let mut files = chat_files(dir);
    while files.len() > KEEP_CHATS {
        let _ = std::fs::remove_file(files.remove(0));
    }
    Ok(())
}

/// Chat files, oldest first (the id is a timestamp).
pub fn chat_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with("chat-") && n.ends_with(".json")));
    v.sort();
    v
}

pub fn load_latest(dir: &Path) -> Option<Chat> {
    let p = chat_files(dir).pop()?;
    let c: Chat = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    Some(c)
}

pub fn delete_all(dir: &Path) {
    for p in chat_files(dir) {
        let _ = std::fs::remove_file(p);
    }
}

impl Default for State {
    fn default() -> Self {
        Self { enabled: true, last_status: None, auto_run: false, provider: None, model: None }
    }
}

pub fn load_state(dir: &Path) -> State {
    std::fs::read_to_string(dir.join("state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_state(dir: &Path, st: &State) {
    if std::fs::create_dir_all(dir).is_ok() {
        if let Ok(t) = serde_json::to_string(st) {
            let _ = write_private(&dir.join("state.json"), &t);
        }
    }
}

/// A chat that ends in the middle of a tool round (assistant asked, no result yet) cannot be
/// continued: cut it back to the last complete state.
pub fn repair(messages: &mut Vec<Message>) {
    while let Some(l) = messages.last() {
        let dangling = l.role == Role::Assistant && !l.tool_calls.is_empty();
        let orphan_tool = l.role == Role::Tool;
        if dangling || orphan_tool {
            messages.pop();
        } else {
            break;
        }
    }
    // A tool result at the end of an incomplete set is handled by the loop above; an assistant
    // message with calls followed by only some results is trimmed the same way.
    while let Some(l) = messages.last() {
        if l.role == Role::Tool || (l.role == Role::Assistant && !l.tool_calls.is_empty()) {
            messages.pop();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::agent::types::{truncate_for_send, ToolCall};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agent-hist-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn truncation_keeps_the_last_window_and_starts_on_a_user_message() {
        let mut msgs = vec![];
        for i in 0..30 {
            msgs.push(Message::user(&format!("q{i}")));
            msgs.push(Message::assistant(&format!("a{i}"), vec![]));
        }
        let t = truncate_for_send(&msgs, SEND_WINDOW);
        assert_eq!(t.len(), 40);
        assert_eq!(t[0].text, "q10");
        // A window that would start on a tool result moves forward to the next user message.
        let mut m2 = vec![Message::user("old"), Message::assistant("", vec![ToolCall { id: "1".into(), name: "x".into(), args: serde_json::json!({}) }]), Message::tool("1", "r", false), Message::assistant("done", vec![]), Message::user("new"), Message::assistant("ok", vec![])];
        let t = truncate_for_send(&m2, 4);
        assert_eq!(t[0].text, "new");
        m2.push(Message::notice("n"));
        assert!(truncate_for_send(&m2, 100).iter().all(|m| m.role != Role::Notice));
    }

    #[test]
    fn chats_round_trip_and_only_the_last_are_kept() {
        let d = tmp("rt");
        for i in 0..(KEEP_CHATS + 3) {
            let c = Chat { id: format!("20260101-0000{i:02}"), provider: "mock".into(), messages: vec![Message::user(&format!("hi {i}"))] };
            save(&d, &c).unwrap();
        }
        assert_eq!(chat_files(&d).len(), KEEP_CHATS);
        let last = load_latest(&d).unwrap();
        assert_eq!(last.messages[0].text, format!("hi {}", KEEP_CHATS + 2));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(chat_files(&d)[0].clone()).unwrap().permissions().mode();
            assert_eq!(m & 0o077, 0, "chats are private");
        }
        delete_all(&d);
        assert!(load_latest(&d).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn state_round_trips_and_defaults_to_confirming() {
        let d = tmp("st");
        assert!(!load_state(&d).auto_run);
        assert!(load_state(&d).enabled, "enabled by default");
        save_state(&d, &State { auto_run: true, provider: Some("groq".into()), ..State::default() });
        assert!(load_state(&d).auto_run);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn repair_drops_an_unfinished_tool_round() {
        let mut m = vec![Message::user("a"), Message::assistant("x", vec![]), Message::user("b"), Message::assistant("", vec![ToolCall { id: "1".into(), name: "x".into(), args: serde_json::json!({}) }])];
        repair(&mut m);
        assert_eq!(m.len(), 3);
        assert_eq!(m.last().unwrap().role, Role::User);
    }
}
