//! `~/.config/harbour/agent.yaml` (or `agent.json` when there is no YAML file): which providers exist
//! and how to reach them. YAML wins when both exist. JSON may contain `//` line comments (see
//! `docs/agent.example.yaml` / `docs/agent.example.json`). Keys are read from
//! the environment when a provider says `"api_key_env"`; they are never logged or shown.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProviderCfg {
    /// `openai`, `anthropic`, `muse` or `mock`.
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// A literal key. Prefer `api_key_env`; with a literal key keep the file at mode 0600.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// `kind: "muse"` only. No token here: the Muse SDK service keeps its own.
    #[serde(default)]
    pub muse: Option<MuseCfg>,
}

/// Optional settings of a `muse` provider (see docs/MUSE.md). All keys have defaults.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct MuseCfg {
    /// The musegadget service socket. Default `/run/musegadget/musegadget.sock`.
    #[serde(default)]
    pub socket: Option<String>,
    /// Muse side chat for the Agent. Default: none = the main Muse chat (side chats fail on the device so far).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Seconds to wait for Muse's answer, 1..600. Default 60.
    #[serde(default)]
    pub wait_secs: Option<u32>,
    /// Where Muse's `harbour.*` commands reach the launcher. Default `$XDG_RUNTIME_DIR/harbour/muse-cmd.sock`.
    #[serde(default)]
    pub commands_socket: Option<String>,
    /// Push-to-talk voice (hold Sym in the Agent panel). Default true; it also needs `vm_id`.
    #[serde(default)]
    pub voice: Option<bool>,
    /// The leased Muse VM id for `musegadget voice-call --vm-id` (see docs/MUSE.md for how to find it).
    #[serde(default)]
    pub vm_id: Option<String>,
    /// The SDK command. Default `musegadget` (looked up on PATH); an absolute path also works.
    #[serde(default)]
    pub sdk_command: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AgentConfig {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, ProviderCfg>,
}

/// The config file to use: `HARBOUR_AGENT_CONFIG` if set, else `agent.yaml`, else `agent.json` when
/// only that exists. With neither file the (missing) YAML path is returned.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("HARBOUR_AGENT_CONFIG") {
        return PathBuf::from(p);
    }
    let dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config/harbour");
    pick_config(&dir)
}

/// `agent.yaml` first, then `agent.json`, in `dir`.
pub fn pick_config(dir: &std::path::Path) -> PathBuf {
    let (y, j) = (dir.join("agent.yaml"), dir.join("agent.json"));
    if !y.exists() && j.exists() {
        j
    } else {
        y
    }
}

fn is_yaml(path: &std::path::Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"))
}

pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HARBOUR_AGENT_DIR") {
        return PathBuf::from(p);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share/harbour/agent")
}

/// Remove `//` comments that start a token outside of a string (a URL inside a string is safe).
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let (mut in_str, mut esc) = (false, false);
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && it.peek() == Some(&'/') {
            for d in it.by_ref() {
                if d == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl AgentConfig {
    /// Parse JSON (with `//` comments).
    pub fn parse(src: &str) -> Result<Self, String> {
        serde_json::from_str(&strip_comments(src)).map_err(|e| format!("agent.json: {e}"))
    }

    /// Parse YAML (comments are native).
    pub fn parse_yaml(src: &str) -> Result<Self, String> {
        serde_yaml::from_str(src).map_err(|e| format!("agent.yaml: {}", e.to_string().lines().next().unwrap_or("invalid")))
    }

    /// Parse by the file's extension.
    pub fn parse_for(path: &std::path::Path, src: &str) -> Result<Self, String> {
        if is_yaml(path) {
            Self::parse_yaml(src)
        } else {
            Self::parse(src)
        }
    }

    /// The built-in fallback when there is no file: just the mock, so the Agent opens and says
    /// how to configure a provider.
    pub fn mock_only() -> Self {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "mock".to_string(),
            ProviderCfg { kind: "mock".into(), base_url: None, model: Some("mock".into()), api_key_env: None, api_key: None, max_tokens: None, muse: None },
        );
        Self { default: Some("mock".into()), providers }
    }

    /// Provider names in a stable order, the default first.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.providers.keys().cloned().collect();
        if let Some(d) = &self.default {
            if let Some(i) = v.iter().position(|n| n == d) {
                let d = v.remove(i);
                v.insert(0, d);
            }
        }
        v
    }

    pub fn default_name(&self) -> Option<String> {
        self.default.clone().filter(|d| self.providers.contains_key(d)).or_else(|| self.names().into_iter().next())
    }
}

/// What loading the file gave: the config, plus a warning for the UI when the file is too open
/// or could not be read.
pub struct Loaded {
    pub config: AgentConfig,
    pub warning: Option<String>,
    pub exists: bool,
}

/// True when group or others can read or write the file.
pub fn mode_too_open(mode: u32) -> bool {
    mode & 0o077 != 0
}

pub fn load(path: &std::path::Path, mock: bool) -> Loaded {
    let text = std::fs::read_to_string(path);
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("agent config").to_string();
    let (mut config, mut warning, exists) = match text {
        Ok(t) => match AgentConfig::parse_for(path, &t) {
            Ok(c) => (c, None, true),
            Err(e) => (AgentConfig::default(), Some(e), true),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (AgentConfig::default(), None, false),
        Err(e) => (AgentConfig::default(), Some(format!("{fname}: {e}")), true),
    };
    #[cfg(unix)]
    if exists && warning.is_none() {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path) {
            if mode_too_open(m.permissions().mode()) {
                warning = Some(format!("{fname} is readable by others: run chmod 600"));
            }
        }
    }
    if mock || config.providers.is_empty() {
        let m = AgentConfig::mock_only();
        for (k, v) in m.providers {
            config.providers.entry(k).or_insert(v);
        }
        if mock || config.default.is_none() {
            config.default = Some("mock".into());
        }
    }
    Loaded { config, warning, exists }
}

/// The key for a provider: `api_key_env` first, then the literal `api_key`. `Ok(None)` = the
/// provider needs none (a local server). `env` is a parameter so tests need no real variables.
pub fn resolve_key(p: &ProviderCfg, env: &dyn Fn(&str) -> Option<String>) -> Result<Option<String>, String> {
    let key = if let Some(name) = p.api_key_env.as_deref().filter(|n| !n.is_empty()) {
        match env(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            Some(k) => Some(k),
            None => match p.api_key.as_deref().filter(|k| !k.is_empty()) {
                Some(k) => Some(k.trim().to_string()),
                None => return Err(format!("environment variable {name} is not set")),
            },
        }
    } else {
        p.api_key.as_deref().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
    };
    if let Some(k) = &key {
        if k.chars().any(|c| c.is_control() || c == '"') {
            return Err("the API key contains characters that are not allowed".into());
        }
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      // default provider
      "default": "groq",
      "providers": {
        "groq": {"kind": "openai", "base_url": "https://api.groq.com/openai/v1", "model": "m", "api_key_env": "GROQ_API_KEY"}, // trailing
        "anthropic": {"kind": "anthropic", "model": "claude-sonnet-5-5", "api_key": "sk-x"}
      }
    }"#;

    #[test]
    fn comments_are_stripped_but_urls_in_strings_survive() {
        let c = AgentConfig::parse(SAMPLE).unwrap();
        assert_eq!(c.providers["groq"].base_url.as_deref(), Some("https://api.groq.com/openai/v1"));
        assert_eq!(c.default_name().as_deref(), Some("groq"));
        assert_eq!(c.names(), ["groq", "anthropic"]);
    }

    #[test]
    fn keys_come_from_the_environment_or_the_file() {
        let c = AgentConfig::parse(SAMPLE).unwrap();
        let env = |n: &str| if n == "GROQ_API_KEY" { Some(" gsk_1 \n".to_string()) } else { None };
        assert_eq!(resolve_key(&c.providers["groq"], &env).unwrap().as_deref(), Some("gsk_1"));
        assert_eq!(resolve_key(&c.providers["anthropic"], &env).unwrap().as_deref(), Some("sk-x"));
        let none = |_: &str| None;
        let err = resolve_key(&c.providers["groq"], &none).unwrap_err();
        assert!(err.contains("GROQ_API_KEY"), "{err}");
        assert!(!err.contains("gsk"));
        let local = ProviderCfg { kind: "openai".into(), base_url: None, model: None, api_key_env: None, api_key: None, max_tokens: None, muse: None };
        assert_eq!(resolve_key(&local, &none).unwrap(), None);
    }

    #[test]
    fn a_key_with_a_newline_is_refused() {
        let mut p = AgentConfig::parse(SAMPLE).unwrap().providers["anthropic"].clone();
        p.api_key = Some("a\nheader = \"x\"".into());
        assert!(resolve_key(&p, &|_| None).is_err());
    }

    #[test]
    fn yaml_parses_like_json_and_wins_over_it() {
        let y = "default: groq\nproviders:\n  groq:\n    kind: openai   # comment\n    base_url: https://api.groq.com/openai/v1\n    model: m\n    api_key_env: GROQ_API_KEY\n";
        let c = AgentConfig::parse_yaml(y).unwrap();
        assert_eq!(c.providers["groq"].base_url.as_deref(), Some("https://api.groq.com/openai/v1"));
        assert_eq!(c.providers["groq"].api_key_env.as_deref(), Some("GROQ_API_KEY"));
        assert!(AgentConfig::parse_yaml("providers: [").unwrap_err().starts_with("agent.yaml:"));
        let d = std::env::temp_dir().join(format!("agent-yaml-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(pick_config(&d).ends_with("agent.yaml"), "neither file: the yaml path");
        std::fs::write(d.join("agent.json"), SAMPLE).unwrap();
        assert!(pick_config(&d).ends_with("agent.json"), "only json exists");
        std::fs::write(d.join("agent.yaml"), y).unwrap();
        assert!(pick_config(&d).ends_with("agent.yaml"), "yaml first");
        let l = load(&d.join("agent.yaml"), false);
        assert!(l.exists && l.config.providers.contains_key("groq"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mode_check_flags_group_and_other_access() {
        assert!(!mode_too_open(0o600));
        assert!(mode_too_open(0o644));
        assert!(mode_too_open(0o640));
    }

    #[test]
    fn a_missing_file_falls_back_to_the_mock() {
        let l = load(std::path::Path::new("/nonexistent/agent.json"), false);
        assert!(!l.exists);
        assert_eq!(l.config.default_name().as_deref(), Some("mock"));
    }

    #[test]
    fn a_broken_file_warns_and_still_gives_the_mock() {
        let d = std::env::temp_dir().join(format!("agent-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("agent.json");
        std::fs::write(&p, "{ nope").unwrap();
        let l = load(&p, false);
        assert!(l.warning.is_some());
        assert!(l.config.providers.contains_key("mock"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
