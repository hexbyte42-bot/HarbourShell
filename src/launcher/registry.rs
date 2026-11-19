//! App registry: discovers `.desktop` entries in the APPLaunch applications
//! directory (plus the preinstalled manifest) and orders them the way the
//! official launcher does (product apps first, then by modification time).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::desktop_entry::{self, DesktopEntry};

pub const DEFAULT_APP_DIR: &str = "/usr/share/APPLaunch/applications";

#[derive(Clone, Debug)]
pub struct AppEntry {
    /// `.desktop` file name (stable identity, also the visibility-key source).
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub exec: String,
    pub terminal: bool,
    pub sysplause: bool,
    pub mtime: Option<SystemTime>,
}

/// Product app order used by the official launcher (labels, normalised).
const BUILTIN_ORDER: &[&str] = &[
    "settings", "store", "cli", "python", "zclaw", "ssh", "ippanel", "files", "camera", "rec",
    "music", "piano", "compass", "irremote", "irchat", "calculator", "snake", "tank",
    "keyboardguide", "factorytest", "gps", "lora", "nfc", "capcc1101subgchat",
];

fn normalize(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn builtin_rank(label: &str) -> Option<usize> {
    let n = normalize(label);
    if n == "subgchat" {
        return Some(23);
    }
    BUILTIN_ORDER.iter().position(|b| *b == n)
}

fn resolve_icon(base: &Path, icon: &str) -> Option<PathBuf> {
    if icon.is_empty() {
        return None;
    }
    let p = Path::new(icon);
    let full = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
    full.is_file().then_some(full)
}

fn from_entry(id: String, e: DesktopEntry, base: &Path, mtime: Option<SystemTime>) -> AppEntry {
    AppEntry {
        id,
        name: e.name,
        icon: resolve_icon(base, &e.icon),
        exec: e.exec,
        terminal: e.terminal,
        sysplause: e.sysplause,
        mtime,
    }
}

/// `filename<TAB>Name<TAB>Icon<TAB>Exec<TAB>Terminal<TAB>Sysplause` lines.
pub fn parse_preinstalled(contents: &str, base: &Path) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for line in contents.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 || !desktop_entry::filename_valid(f[0]) || f[1].is_empty() || f[3].is_empty() {
            continue;
        }
        let b = |i: usize, d: bool| {
            f.get(i)
                .map(|v| matches!(*v, "true" | "True" | "1"))
                .unwrap_or(d)
        };
        out.push(from_entry(
            f[0].to_string(),
            DesktopEntry {
                name: f[1].to_string(),
                icon: f[2].to_string(),
                exec: f[3].to_string(),
                terminal: b(4, false),
                sysplause: b(5, true),
                name_ja: String::new(),
                aliases: Vec::new(),
            },
            base,
            None,
        ));
    }
    out
}

/// Scan `app_dir`. Unreadable or invalid files are skipped.
pub fn scan(app_dir: &Path) -> Vec<AppEntry> {
    let base = app_dir.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut apps: Vec<AppEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(app_dir) {
        for ent in rd.flatten() {
            let Some(name) = ent.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !desktop_entry::filename_valid(&name) {
                continue;
            }
            let path = ent.path();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(entry) = desktop_entry::parse(&text) else {
                continue;
            };
            let mtime = ent.metadata().and_then(|m| m.modified()).ok();
            apps.push(from_entry(name, entry, &base, mtime));
        }
    }
    if let Ok(text) = std::fs::read_to_string(base.join("preinstalled-desktop-apps.tsv")) {
        for e in parse_preinstalled(&text, &base) {
            if !apps.iter().any(|a| a.id == e.id) {
                apps.push(e);
            }
        }
    }
    sort_official(&mut apps);
    apps
}

/// Official order: product apps by rank, then everything else by mtime (oldest first).
pub fn sort_official(apps: &mut [AppEntry]) {
    apps.sort_by(|a, b| {
        let ra = builtin_rank(&a.name);
        let rb = builtin_rank(&b.name);
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => match (a.mtime, b.mtime) {
                (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.name.cmp(&b.name)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            },
        }
    });
}

/// Apply a user-chosen order (list of `.desktop` ids). Unlisted apps keep their
/// relative position after the listed ones.
pub fn apply_order(apps: &mut Vec<AppEntry>, order: &[String]) {
    let mut sorted: Vec<AppEntry> = Vec::with_capacity(apps.len());
    for id in order {
        if let Some(i) = apps.iter().position(|a| &a.id == id) {
            sorted.push(apps.remove(i));
        }
    }
    sorted.append(apps);
    *apps = sorted;
}

pub fn load_order(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn save_order(path: &Path, ids: &[String]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, ids.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, secs: Option<u64>) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            icon: None,
            exec: "/bin/x".into(),
            terminal: false,
            sysplause: true,
            mtime: secs.map(|s| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s)),
        }
    }

    #[test]
    fn product_apps_first_then_mtime() {
        let mut v = vec![
            app("z.desktop", "Zeta", Some(10)),
            app("calc.desktop", "Calculator", Some(500)),
            app("a.desktop", "Alpha", Some(5)),
            app("f.desktop", "Files", Some(900)),
        ];
        sort_official(&mut v);
        let names: Vec<_> = v.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Files", "Calculator", "Alpha", "Zeta"]);
    }

    #[test]
    fn rank_normalises_labels() {
        assert_eq!(builtin_rank("Keyboard Guide"), Some(18));
        assert_eq!(builtin_rank("IR-Remote"), Some(13));
        assert_eq!(builtin_rank("Fune"), None);
    }

    #[test]
    fn custom_order_wins_and_unlisted_follow() {
        let mut v = vec![app("a.desktop", "A", None), app("b.desktop", "B", None), app("c.desktop", "C", None)];
        apply_order(&mut v, &["c.desktop".into(), "missing.desktop".into(), "a.desktop".into()]);
        let ids: Vec<_> = v.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["c.desktop", "a.desktop", "b.desktop"]);
    }

    #[test]
    fn preinstalled_manifest() {
        let t = "# header\nzclaw.desktop\tZClaw\tshare/images/claw_100.png\t/usr/share/APPLaunch/bin/ZClaw\tfalse\ttrue\n";
        let v = parse_preinstalled(t, Path::new("/nonexistent"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "ZClaw");
        assert!(!v[0].terminal && v[0].sysplause);
    }
}
