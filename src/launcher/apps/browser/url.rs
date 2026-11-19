//! URL handling for the Browser: typed input -> URL, resolving links against a base, form
//! query building. Pure functions, no I/O.

pub const SEARCH: &str = "https://lite.duckduckgo.com/lite/?q=";

/// Percent-encode everything except unreserved characters (`-_.~` and alphanumerics).
pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Form encoding: like [`pct`] but a space is `+`.
pub fn form_encode(s: &str) -> String {
    pct(s).replace("%20", "+")
}

/// Encode only what must never appear raw in a URL (space, control characters, non-ASCII, quotes).
fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii() && !ch.is_control() && !" \"<>`{}|\\^".contains(ch) {
            out.push(ch);
        } else {
            let mut b = [0u8; 4];
            for x in ch.encode_utf8(&mut b).bytes() {
                out.push_str(&format!("%{x:02X}"));
            }
        }
    }
    out
}

/// What was typed in the address field -> a URL, or search words -> a search URL.
pub fn normalize(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("about:") || lower.starts_with("file://") {
        return Some(s.to_string());
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let rest = &s[s.find("://")? + 3..];
        if rest.is_empty() || rest.starts_with('/') || rest.contains(char::is_whitespace) {
            return None;
        }
        let clean = sanitize(s);
        // a bare host gets its "/" (https://example.org -> https://example.org/)
        return Some(match parse(&clean) {
            Some(p) if p.path.is_empty() => Parts { path: "/".into(), ..p }.to_url(),
            _ => clean,
        });
    }
    if s.starts_with('/') && !s.contains(char::is_whitespace) {
        return Some(format!("file://{s}"));
    }
    if s.contains("://") {
        return None;
    }
    let host_like = !s.contains(char::is_whitespace) && (s.contains('.') || s.contains(':') || lower == "localhost");
    if host_like {
        let scheme = if lower.starts_with("localhost") { "http://" } else { "https://" };
        return normalize(&format!("{scheme}{s}"));
    }
    let words: Vec<String> = s.split_whitespace().map(form_encode).collect();
    Some(format!("{SEARCH}{}", words.join("+")))
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Parts {
    pub scheme: String,
    /// host[:port]; empty for file:
    pub host: String,
    /// Starts with `/` (or empty).
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

pub fn parse(url: &str) -> Option<Parts> {
    let (scheme, rest) = url.split_once(':')?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let (rest, fragment) = match rest.split_once('#') {
        Some((a, b)) => (a, Some(b.to_string())),
        None => (rest, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((a, b)) => (a, Some(b.to_string())),
        None => (rest, None),
    };
    let (host, path) = if let Some(r) = rest.strip_prefix("//") {
        match r.find('/') {
            Some(i) => (r[..i].to_string(), r[i..].to_string()),
            None => (r.to_string(), String::new()),
        }
    } else {
        (String::new(), rest.to_string())
    };
    Some(Parts { scheme, host: host.to_ascii_lowercase(), path, query, fragment })
}

impl Parts {
    pub fn to_url(&self) -> String {
        let mut s = format!("{}:", self.scheme);
        if !self.host.is_empty() || self.scheme == "file" {
            s.push_str("//");
            s.push_str(&self.host);
        }
        s.push_str(&self.path);
        if let Some(q) = &self.query {
            s.push('?');
            s.push_str(q);
        }
        if let Some(f) = &self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }
}

/// Remove `.` and `..` segments.
fn dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').collect();
    for (i, seg) in segs.iter().enumerate() {
        match *seg {
            "." => {
                if i == segs.len() - 1 {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if i == segs.len() - 1 {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let r = out.join("/");
    if r.is_empty() { "/".into() } else { r }
}

/// Resolve `href` (as found in a page) against `base`.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return Some(base.to_string());
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("mailto:") || lower.starts_with("tel:") || lower.starts_with("data:") {
        return None;
    }
    let b = parse(base)?;
    if let Some(p) = parse(href) {
        // an absolute URL (has a scheme); "host:80" style is not supported
        if href.contains("://") || p.scheme == "about" || p.scheme == "file" {
            return Some(Parts { path: if p.path.is_empty() && !p.host.is_empty() { "/".into() } else { dot_segments(&p.path) }, ..p }.to_url());
        }
    }
    if let Some(f) = href.strip_prefix('#') {
        return Some(Parts { fragment: Some(f.to_string()), ..b }.to_url());
    }
    if let Some(r) = href.strip_prefix("//") {
        return parse(&format!("{}://{}", b.scheme, r)).map(|p| Parts { path: if p.path.is_empty() { "/".into() } else { dot_segments(&p.path) }, ..p }.to_url());
    }
    let (rel, fragment) = match href.split_once('#') {
        Some((a, f)) => (a, Some(f.to_string())),
        None => (href, None),
    };
    let (rel_path, query) = match rel.split_once('?') {
        Some((a, q)) => (a, Some(q.to_string())),
        None => (rel, None),
    };
    let (path, query) = if rel_path.is_empty() {
        (b.path.clone(), query.or(b.query.clone()))
    } else if rel_path.starts_with('/') {
        (dot_segments(rel_path), query)
    } else {
        let dir = match b.path.rfind('/') {
            Some(i) => &b.path[..=i],
            None => "/",
        };
        (dot_segments(&format!("{dir}{rel_path}")), query)
    };
    Some(Parts { path, query, fragment, ..b }.to_url())
}

/// The URL without its `#fragment`.
/// The real target of a search engine's click-through link: DuckDuckGo's result links go through
/// `duckduckgo.com/l/?uddg=<target>`, a page that only redirects with JavaScript (and a `<noscript>`
/// refresh). Following the target directly saves a fetch and works without either.
pub fn unwrap_redirect(url: &str) -> String {
    let Some(p) = parse(url) else { return url.to_string() };
    let host = p.host.trim_start_matches("www.");
    if (host == "duckduckgo.com" || host == "html.duckduckgo.com" || host == "lite.duckduckgo.com") && p.path == "/l/" {
        if let Some(q) = p.query.as_deref() {
            for kv in q.split('&') {
                if let Some(v) = kv.strip_prefix("uddg=") {
                    let t = pct_decode(v);
                    if t.starts_with("http://") || t.starts_with("https://") {
                        return t;
                    }
                }
            }
        }
    }
    url.to_string()
}

/// `%XX` decoding (and `+` as a space) of a query value; invalid escapes stay as they are.
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match h {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn strip_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

pub fn fragment(url: &str) -> Option<&str> {
    url.split_once('#').map(|(_, f)| f)
}

/// `action` + the form's fields -> GET URL (query of the action is replaced, as browsers do).
pub fn form_get_url(base: &str, action: &str, fields: &[(String, String)]) -> Option<String> {
    let target = resolve(base, action)?;
    let mut p = parse(&target)?;
    p.fragment = None;
    p.query = None;
    let q: Vec<String> = fields.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect();
    let mut u = p.to_url();
    if !q.is_empty() {
        u.push('?');
        u.push_str(&q.join("&"));
    }
    Some(u)
}

pub fn form_body(fields: &[(String, String)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&")
}

/// Short text for the status strip: host + path.
pub fn display(url: &str) -> String {
    url.split_once("://").map(|(_, r)| r).unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn duckduckgo_result_links_go_straight_to_the_target() {
        let l = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Frust%2Dlang.org%2Ftools%2Finstall%2F&rut=2e96";
        assert_eq!(super::unwrap_redirect(l), "https://rust-lang.org/tools/install/");
        assert_eq!(super::unwrap_redirect("https://duckduckgo.com/l/?rut=x"), "https://duckduckgo.com/l/?rut=x");
        assert_eq!(super::unwrap_redirect("https://example.org/l/?uddg=https%3A%2F%2Fa.b"), "https://example.org/l/?uddg=https%3A%2F%2Fa.b");
        assert_eq!(super::unwrap_redirect("https://duckduckgo.com/l/?uddg=javascript%3Aalert(1)"), "https://duckduckgo.com/l/?uddg=javascript%3Aalert(1)");
    }

    use super::*;

    #[test]
    fn typed_input() {
        assert_eq!(normalize("example.com").unwrap(), "https://example.com/");
        assert_eq!(normalize("example.com?x=1").unwrap(), "https://example.com/?x=1");
        assert_eq!(normalize("https://example.com/a?b=1&c=2#x").unwrap(), "https://example.com/a?b=1&c=2#x");
        assert_eq!(normalize("localhost:8080").unwrap(), "http://localhost:8080/");
        assert_eq!(normalize("localhost:8080/x").unwrap(), "http://localhost:8080/x");
        assert_eq!(normalize("about:home").unwrap(), "about:home");
        assert_eq!(normalize("/tmp/a.html").unwrap(), "file:///tmp/a.html");
        assert_eq!(normalize("rust sdl2").unwrap(), format!("{SEARCH}rust+sdl2"));
        assert_eq!(normalize("c++ & you").unwrap(), format!("{SEARCH}c%2B%2B+%26+you"));
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("https://"), None);
        assert_eq!(normalize("ftp://x.org"), None);
        assert_eq!(normalize("http://a b"), None);
        assert!(normalize("https://x.org/a\"c").unwrap().contains("%22"));
    }

    #[test]
    fn resolving() {
        let b = "https://example.org/a/b/c.html?x=1";
        assert_eq!(resolve(b, "d.html").unwrap(), "https://example.org/a/b/d.html");
        assert_eq!(resolve(b, "../d.html").unwrap(), "https://example.org/a/d.html");
        assert_eq!(resolve(b, "../../../../d").unwrap(), "https://example.org/d");
        assert_eq!(resolve(b, "/top").unwrap(), "https://example.org/top");
        assert_eq!(resolve(b, "//other.net/p").unwrap(), "https://other.net/p");
        assert_eq!(resolve(b, "?q=2").unwrap(), "https://example.org/a/b/c.html?q=2");
        assert_eq!(resolve(b, "#sec").unwrap(), "https://example.org/a/b/c.html?x=1#sec");
        assert_eq!(resolve(b, "http://Foo.com").unwrap(), "http://foo.com/");
        assert_eq!(resolve(b, "./").unwrap(), "https://example.org/a/b/");
        assert_eq!(resolve(b, "").unwrap(), b);
        assert_eq!(resolve(b, "javascript:void(0)"), None);
        assert_eq!(resolve(b, "mailto:a@b.c"), None);
        assert_eq!(resolve("https://x.org", "a").unwrap(), "https://x.org/a");
        assert_eq!(resolve("about:home", "https://x.org/").unwrap(), "https://x.org/");
        assert_eq!(resolve("file:///tmp/a.html", "b.html").unwrap(), "file:///tmp/b.html");
    }

    #[test]
    fn fragments() {
        assert_eq!(strip_fragment("https://a.b/c#d"), "https://a.b/c");
        assert_eq!(fragment("https://a.b/c#d"), Some("d"));
        assert_eq!(fragment("https://a.b/c"), None);
    }

    #[test]
    fn get_forms() {
        let f = vec![("q".to_string(), "rust lang".to_string()), ("kl".to_string(), "jp-jp".to_string())];
        assert_eq!(form_get_url("https://x.org/a/b", "/lite/?old=1", &f).unwrap(), "https://x.org/lite/?q=rust+lang&kl=jp-jp");
        assert_eq!(form_get_url("https://x.org/a/b", "", &f).unwrap(), "https://x.org/a/b?q=rust+lang&kl=jp-jp");
        assert_eq!(form_body(&[("a".into(), "日&".into())]), "a=%E6%97%A5%26");
    }
}
