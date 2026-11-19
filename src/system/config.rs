//! Shared settings file used by the official launcher
//! (`~/.config/cardputerzero/config.json`). Harbour reads and writes the same
//! keys (`brightness`, `dark_time`, `volume`, and `app_desktop_*` visibility
//! flags) so that switching back to the official launcher keeps the settings.
//! Unknown keys are preserved.

use serde_json::{Map, Value};
use std::path::PathBuf;

pub struct Config {
    path: PathBuf,
    map: Map<String, Value>,
    /// An unreadable or malformed official config must not be overwritten.
    writable: bool,
}

pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("HARBOUR_CONFIG") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/cardputerzero/config.json")
}

impl Config {
    pub fn load(path: PathBuf) -> Self {
        let parsed = match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Value>(&s) {
                Ok(Value::Object(map)) => Ok(map),
                Ok(_) => Err("configuration is not a JSON object".to_string()),
                Err(e) => Err(format!("invalid configuration JSON: {e}")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
            Err(e) => Err(format!("cannot read configuration: {e}")),
        };
        let writable = parsed.is_ok();
        if let Err(e) = &parsed {
            eprintln!("[config] {e}; refusing to overwrite {}", path.display());
        }
        Self { path, map: parsed.unwrap_or_default(), writable }
    }

    pub fn get_int(&self, key: &str, default: i64) -> i64 {
        self.map.get(key).and_then(Value::as_i64).unwrap_or(default)
    }

    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.map.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.map.insert(key.to_string(), Value::from(v));
    }

    pub fn set_int(&mut self, key: &str, v: i64) {
        self.map.insert(key.to_string(), Value::from(v));
    }

    /// Atomically write the file (temp file + rename).
    pub fn save(&self) -> std::io::Result<()> {
        if !self.writable {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("refusing to overwrite malformed or unreadable configuration: {}", self.path.display()),
            ));
        }
        // Detect corruption that happened after the initial load, too.
        if self.path.exists() {
            let current = std::fs::read_to_string(&self.path)?;
            if !matches!(serde_json::from_str::<Value>(&current), Ok(Value::Object(_))) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "configuration changed to invalid JSON"));
            }
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&Value::Object(self.map.clone()))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&tmp, text + "\n")?;
        std::fs::rename(tmp, &self.path)
    }
}

/// Visibility key used by the official launcher for a `.desktop` file:
/// `app_desktop_<name with non-alphanumerics as _>_<fnv1a-32 hex>`.
pub fn desktop_config_key(filename: &str) -> String {
    let mut key = String::from("app_desktop_");
    for b in filename.bytes() {
        key.push(if b.is_ascii_alphanumeric() { b as char } else { '_' });
    }
    let mut h: u32 = 2166136261;
    for b in filename.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    format!("{key}_{h:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_official_scheme() {
        // fnv1a("a") = 0xe40c292c
        assert_eq!(desktop_config_key("a"), "app_desktop_a_e40c292c");
        let k = desktop_config_key("fune.desktop");
        assert!(k.starts_with("app_desktop_fune_desktop_"));
        assert_eq!(k.len(), "app_desktop_fune_desktop_".len() + 8);
    }

    #[test]
    fn roundtrip_preserves_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("harbour-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"ssh_host":"x","brightness":40}"#).unwrap();
        let mut c = Config::load(path.clone());
        assert_eq!(c.get_int("brightness", 0), 40);
        c.set_int("volume", 55);
        c.save().unwrap();
        let again = Config::load(path.clone());
        assert_eq!(again.get_int("volume", 0), 55);
        assert!(std::fs::read_to_string(&path).unwrap().contains("ssh_host"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn malformed_configuration_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("harbour-invalid-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        for broken in ["{broken", "[]"] {
            std::fs::write(&path, broken).unwrap();
            let mut cfg = Config::load(path.clone());
            cfg.set_int("brightness", 25);
            assert_eq!(cfg.save().unwrap_err().kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn configuration_corrupted_after_load_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("harbour-changed-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{}").unwrap();
        let mut cfg = Config::load(path.clone());
        cfg.set_int("brightness", 25);
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(cfg.save().unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        let _ = std::fs::remove_dir_all(dir);
    }
}
