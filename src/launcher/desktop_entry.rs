//! `.desktop` parsing with the same rules as the official APPLaunch
//! (M5Stack, MIT): `Name` and `Exec` are required, `Terminal`/`Sysplause` are
//! booleans (`Sysplause` defaults to true), `Hidden`/`NoDisplay` hide an entry,
//! and `Type` must be `Application` when present.

pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_ICON_BYTES: usize = 512;
pub const MAX_EXEC_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub icon: String,
    pub exec: String,
    pub terminal: bool,
    pub sysplause: bool,
    /// `Name[ja]` (or `Name[ja_JP]`): the Japanese name. Empty when the entry has none.
    pub name_ja: String,
    /// `X-Harbour-Aliases`: other names the app is known by (`;`-separated, freedesktop list style).
    pub aliases: Vec<String>,
}

fn parse_bool(v: &str) -> bool {
    matches!(v, "true" | "True" | "1")
}

fn has_control(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20 || b == 0x7f)
}

/// File names must end in `.desktop` and contain no separators or control bytes.
pub fn filename_valid(name: &str) -> bool {
    const SUFFIX: &str = ".desktop";
    name.len() > SUFFIX.len()
        && name.ends_with(SUFFIX)
        && !name.bytes().any(|b| b < 0x20 || b == 0x7f || b == b'/' || b == b'\\')
}

pub fn parse(contents: &str) -> Option<DesktopEntry> {
    let mut e = DesktopEntry {
        name: String::new(),
        icon: String::new(),
        exec: String::new(),
        terminal: false,
        sysplause: true,
        name_ja: String::new(),
        aliases: Vec::new(),
    };
    let mut ja_jp = String::new();
    let mut in_entry = false;
    let mut hidden = false;
    let mut valid_type = true;
    for raw in contents.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim_matches([' ', '\t']), v.trim_matches([' ', '\t']));
        match k {
            "Name" => e.name = v.to_string(),
            "Name[ja]" => e.name_ja = v.to_string(),
            "Name[ja_JP]" => ja_jp = v.to_string(),
            "X-Harbour-Aliases" => {
                e.aliases = v.split(';').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect()
            }
            "Icon" => e.icon = v.to_string(),
            "Exec" => e.exec = v.to_string(),
            "Terminal" => e.terminal = parse_bool(v),
            "Sysplause" => e.sysplause = parse_bool(v),
            "Hidden" | "NoDisplay" => hidden = hidden || parse_bool(v),
            "Type" => valid_type = v == "Application",
            _ => {}
        }
    }
    if e.name_ja.is_empty() {
        e.name_ja = ja_jp;
    }
    if e.name_ja.len() > MAX_NAME_BYTES || has_control(&e.name_ja) {
        e.name_ja.clear();
    }
    if hidden
        || !valid_type
        || e.name.is_empty()
        || e.exec.is_empty()
        || e.name.len() > MAX_NAME_BYTES
        || e.icon.len() > MAX_ICON_BYTES
        || e.exec.len() > MAX_EXEC_BYTES
        || has_control(&e.name)
        || has_control(&e.icon)
        || e.exec.contains('\0')
    {
        return None;
    }
    Some(e)
}

/// Commands without a path that may be run from `Exec` (matches the official list).
const BARE_ALLOWED: &[&str] = &["bash", "sh", "python3", "vim", "vi", "nano"];

/// Characters that would give `Exec` shell semantics. Harbour never runs `Exec`
/// through a shell, so these are rejected outright.
const SHELL_META: &[char] = &[
    ';', '|', '&', '<', '>', '$', '`', '\\', '"', '\'', '(', ')', '{', '}', '[', ']', '*', '?',
    '!', '~', '#', '\n', '\r',
];

/// Split an `Exec` value into argv. Returns None when it is unsafe.
pub fn exec_argv(exec: &str) -> Option<Vec<String>> {
    if exec.contains(SHELL_META) || exec.bytes().any(|b| b < 0x20 && b != b'\t') {
        return None;
    }
    let argv: Vec<String> = exec.split_whitespace().map(str::to_string).collect();
    let first = argv.first()?;
    if first.contains('/') {
        Some(argv)
    } else if BARE_ALLOWED.contains(&first.as_str()) {
        Some(argv)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_official_fune_entry() {
        let e = parse(
            "[Desktop Entry]\nName=Fune\nExec=/usr/share/APPLaunch/bin/fune\nTerminal=false\nIcon=share/images/fune.png\nType=Application\n",
        )
        .unwrap();
        assert_eq!(e.name, "Fune");
        assert_eq!(e.exec, "/usr/share/APPLaunch/bin/fune");
        assert!(!e.terminal);
        assert!(e.sysplause, "Sysplause defaults to true");
    }

    #[test]
    fn hidden_and_wrong_type_are_rejected() {
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nNoDisplay=true\n").is_none());
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nHidden=1\n").is_none());
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nType=Link\n").is_none());
        assert!(parse("[Desktop Entry]\nName=A\n").is_none());
        assert!(parse("[Other]\nName=A\nExec=/bin/a\n").is_none());
    }

    #[test]
    fn japanese_name_and_aliases() {
        let e = parse("[Desktop Entry]\nName=Paint\nName[ja]=ペイント\nExec=harbour:paint\nX-Harbour-Aliases=draw; sketch;\n").unwrap();
        assert_eq!((e.name.as_str(), e.name_ja.as_str()), ("Paint", "ペイント"));
        assert_eq!(e.aliases, ["draw", "sketch"]);
        let e = parse("[Desktop Entry]\nName=A\nName[ja_JP]=エー\nExec=/bin/a\n").unwrap();
        assert_eq!(e.name_ja, "エー");
        let e = parse("[Desktop Entry]\nName=A\nExec=/bin/a\n").unwrap();
        assert!(e.name_ja.is_empty() && e.aliases.is_empty());
    }

    #[test]
    fn terminal_and_sysplause() {
        let e = parse("[Desktop Entry]\nName=V\nExec=vim\nTerminal=true\nSysplause=false\n").unwrap();
        assert!(e.terminal && !e.sysplause);
    }

    #[test]
    fn comments_and_crlf() {
        let e = parse("# c\r\n[Desktop Entry]\r\n; x\r\nName = Calc \r\nExec = /bin/calc\r\n").unwrap();
        assert_eq!(e.name, "Calc");
        assert_eq!(e.exec, "/bin/calc");
    }

    #[test]
    fn rejects_control_characters_and_oversize() {
        assert!(parse("[Desktop Entry]\nName=A\u{1}B\nExec=/bin/a\n").is_none());
        let long = "x".repeat(MAX_EXEC_BYTES + 1);
        assert!(parse(&format!("[Desktop Entry]\nName=A\nExec=/bin/{long}\n")).is_none());
    }

    #[test]
    fn filename_rules() {
        assert!(filename_valid("fune.desktop"));
        assert!(!filename_valid(".desktop"));
        assert!(!filename_valid("vim.desktop.temple"));
        assert!(!filename_valid("a/b.desktop"));
    }

    #[test]
    fn exec_safety() {
        assert_eq!(
            exec_argv("/usr/bin/foo --bar 1").unwrap(),
            vec!["/usr/bin/foo", "--bar", "1"]
        );
        assert!(exec_argv("vim").is_some());
        assert!(exec_argv("rm -rf x").is_none(), "bare command not allow-listed");
        assert!(exec_argv("/bin/a; rm x").is_none());
        assert!(exec_argv("/bin/a $(id)").is_none());
        assert!(exec_argv("/bin/a > /tmp/x").is_none());
        assert!(exec_argv("").is_none());
    }
}
