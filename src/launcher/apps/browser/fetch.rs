//! Getting bytes: `curl` for http(s) (it does TLS, redirects and gzip, so no TLS crate is
//! needed), plain file reads for `file://`, built-in pages for `about:`, and a mock for
//! `HARBOUR_MOCK=1` that serves the fixtures in `tests/fixtures/`. Everything sits behind the
//! [`Fetcher`] trait so the browser can be tested without a network.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const UA: &str = "Mozilla/5.0 (X11; Linux aarch64; Harbour) Lynx-like/1.0";
pub const PAGE_MAX: u64 = 3_000_000;
pub const HTML_MAX: usize = 2_000_000;
pub const IMAGE_MAX: u64 = 600_000;
/// `progress` value meaning "length not known".
pub const UNKNOWN: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Req {
    pub url: String,
    pub post: Option<String>,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Resp {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

pub trait Fetcher: Send + Sync {
    /// Blocking. Sets `cancel` -> returns Err("Cancelled"). `progress` is permille (0..=1000) or [`UNKNOWN`].
    fn fetch(&self, req: &Req, cancel: &AtomicBool, progress: &AtomicU32) -> Result<Resp, String> {
        self.fetch_partial(req, cancel, progress, &mut |_| {})
    }

    /// Like `fetch`, but `partial` is called now and then with everything received so far (so the
    /// page can be shown before the download ends).
    fn fetch_partial(&self, req: &Req, cancel: &AtomicBool, progress: &AtomicU32, partial: &mut dyn FnMut(&[u8])) -> Result<Resp, String>;
}

pub fn curl_path() -> Option<PathBuf> {
    ["/usr/bin/curl", "/usr/local/bin/curl", "/bin/curl", "/opt/homebrew/bin/curl"].iter().map(PathBuf::from).find(|p| p.is_file())
}

pub struct Real;

fn tmp_name(tag: &str) -> PathBuf {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("harbour-browser-{}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), tag))
}

/// The last header block of a `curl -D` file (earlier ones belong to redirects).
pub fn parse_headers(text: &str) -> (u16, String, Option<u64>) {
    let block = text.split("\r\n\r\n").map(str::trim).filter(|b| !b.is_empty()).last().unwrap_or("");
    let mut status = 0;
    let (mut ctype, mut len) = (String::new(), None);
    for (i, line) in block.lines().enumerate() {
        if i == 0 {
            status = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        } else if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-type" => ctype = v.trim().to_string(),
                "content-length" => len = v.trim().parse().ok(),
                _ => {}
            }
        }
    }
    (status, ctype, len)
}

fn curl_error(code: i32, stderr: &str) -> String {
    let msg = match code {
        6 => "Could not resolve host".to_string(),
        7 => "Connection refused".to_string(),
        28 => "Timed out".to_string(),
        35 | 51 | 58 | 60 | 77 => "TLS error".to_string(),
        47 => "Too many redirects".to_string(),
        56 => "Connection reset".to_string(),
        63 => "Too large".to_string(),
        _ => stderr.lines().last().unwrap_or("Network error").trim_start_matches("curl: ").to_string(),
    };
    msg
}

impl Fetcher for Real {
    fn fetch_partial(&self, req: &Req, cancel: &AtomicBool, progress: &AtomicU32, partial: &mut dyn FnMut(&[u8])) -> Result<Resp, String> {
        let u = req.url.as_str();
        if let Some(path) = u.strip_prefix("file://") {
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let ct = if path.ends_with(".html") || path.ends_with(".htm") { "text/html" } else { guess_type(path) };
            return Ok(Resp { url: u.to_string(), status: 200, content_type: ct.to_string(), body: bytes });
        }
        if u.starts_with("about:") {
            return Ok(Resp { url: u.to_string(), status: 200, content_type: "text/html; charset=utf-8".into(), body: Vec::new() });
        }
        if !(u.starts_with("http://") || u.starts_with("https://")) {
            return Err("Unsupported address".into());
        }
        let curl = curl_path().ok_or("curl is not installed")?;
        let (out, hdr) = (tmp_name("body"), tmp_name("hdr"));
        let mut cmd = Command::new(curl);
        cmd.args(["-sS", "-L", "--max-time", "20", "--connect-timeout", "10", "--max-redirs", "8", "--compressed", "--proto", "=http,https", "--proto-redir", "=http,https"])
            .arg("--max-filesize")
            .arg(req.max_bytes.to_string())
            .args(["-A", UA, "-D"])
            .arg(&hdr)
            .arg("-o")
            .arg(&out)
            .args(["-w", "%{url_effective}"]);
        if let Some(body) = &req.post {
            cmd.args(["-H", "Content-Type: application/x-www-form-urlencoded", "--data-binary"]).arg(body);
        }
        cmd.arg("--").arg(u);
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("curl: {e}"))?;
        let started = Instant::now();
        let mut total: Option<u64> = None;
        let (mut last_partial, mut last_len) = (Instant::now(), 0u64);
        let cleanup = || {
            let _ = std::fs::remove_file(&out);
            let _ = std::fs::remove_file(&hdr);
        };
        let exit = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                cleanup();
                return Err("Cancelled".into());
            }
            if let Ok(Some(st)) = child.try_wait() {
                break st;
            }
            let got = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            if total.is_none() {
                if let Ok(h) = std::fs::read_to_string(&hdr) {
                    total = parse_headers(&h).2;
                }
            }
            progress.store(
                match total {
                    Some(t) if t > 0 => (got * 1000 / t).min(1000) as u32,
                    _ => UNKNOWN,
                },
                Ordering::Relaxed,
            );
            if got >= last_len + 6000 && last_partial.elapsed() > Duration::from_millis(600) && got <= req.max_bytes {
                if let Ok(b) = std::fs::read(&out) {
                    partial(&b);
                }
                last_len = got;
                last_partial = Instant::now();
            }
            if got > req.max_bytes {
                // no Content-Length and too much data: keep what we have
                let _ = child.kill();
                let _ = child.wait();
                let body = std::fs::read(&out).unwrap_or_default();
                let (status, ct, _) = parse_headers(&std::fs::read_to_string(&hdr).unwrap_or_default());
                cleanup();
                let mut body = body;
                body.truncate(req.max_bytes as usize);
                return Ok(Resp { url: u.to_string(), status, content_type: ct, body });
            }
            if started.elapsed() > Duration::from_secs(25) {
                let _ = child.kill();
                let _ = child.wait();
                cleanup();
                return Err("Timed out".into());
            }
            std::thread::sleep(Duration::from_millis(40));
        };
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !exit.success() {
            cleanup();
            return Err(curl_error(exit.code().unwrap_or(-1), &stderr));
        }
        let eff = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let (status, ct, _) = parse_headers(&std::fs::read_to_string(&hdr).unwrap_or_default());
        let body = std::fs::read(&out).unwrap_or_default();
        cleanup();
        progress.store(1000, Ordering::Relaxed);
        Ok(Resp { url: if eff.is_empty() { u.to_string() } else { eff }, status, content_type: ct, body })
    }
}

fn guess_type(path: &str) -> &'static str {
    let l = path.to_ascii_lowercase();
    for (ext, t) in [(".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"), (".gif", "image/gif"), (".bmp", "image/bmp"), (".txt", "text/plain")] {
        if l.ends_with(ext) {
            return t;
        }
    }
    "text/plain"
}

// ------------------------------------------------------------------------------- mock

macro_rules! fixture {
    ($n:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $n)).to_vec()
    };
}

/// Serves the fixtures; no network, no processes.
pub struct Mock;

impl Fetcher for Mock {
    fn fetch_partial(&self, req: &Req, cancel: &AtomicBool, progress: &AtomicU32, partial: &mut dyn FnMut(&[u8])) -> Result<Resp, String> {
        if req.url.starts_with("file://") {
            return Real.fetch(req, cancel, progress);
        }
        // a visible, deterministic "connecting" phase for screenshots
        progress.store(UNKNOWN, Ordering::Relaxed);
        for _ in 0..3 {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let u = req.url.as_str();
        let html = "text/html; charset=utf-8";
        let (ct, body): (&str, Vec<u8>) = match crate::launcher::apps::browser::url::strip_fragment(u) {
            "https://example.org/" | "http://example.org/" => (html, fixture!("example.html")),
            "https://example.org/news" => (html, fixture!("news.html")),
            u if u.starts_with("http://frames.example/") || u.starts_with("https://frames.example/") => {
                // a Shift_JIS frameset site: http redirects to https; Shift_JIS, no charset in the HTTP header
                let path = u.splitn(4, '/').nth(3).unwrap_or("");
                let body = match path {
                    "" | "index.htm" => fixture!("sjis_frames/index.htm"),
                    "menu.htm" => fixture!("sjis_frames/menu.htm"),
                    "top.htm" => fixture!("sjis_frames/top.htm"),
                    _ => b"<title>404</title><p>not in the fixtures</p>".to_vec(),
                };
                progress.store(1000, Ordering::Relaxed);
                let final_url = u.replacen("http://", "https://", 1);
                return Ok(Resp { url: final_url, status: 200, content_type: "text/html".into(), body });
            }
            "https://example.org/article" => (html, fixture!("article.html")),
            "https://example.org/logo.png" => ("image/png", fixture!("logo.png")),
            s if s.starts_with("https://lite.duckduckgo.com/lite/") => {
                let q = s.split_once("q=").map(|(_, r)| r.split('&').next().unwrap_or("")).unwrap_or("");
                let page = String::from_utf8_lossy(&fixture!("search.html")).replace("@Q@", &q.replace('+', " "));
                (html, page.into_bytes())
            }
            "https://example.org/echo" if req.post.is_some() => (html, format!("<h1>POST</h1><p>{}</p>", req.post.clone().unwrap()).into_bytes()),
            _ => ("text/html", b"<title>Not found</title><h1>404</h1><p>No such page in the mock web.</p><p><a href=\"https://example.org/\">example.org</a></p>".to_vec()),
        };
        if ct.starts_with("text/html") && body.len() > 400 {
            // show the first half early, as a slow connection would
            let cut = body[..body.len() / 2].iter().rposition(|&b| b == b'>').unwrap_or(0) + 1;
            progress.store(500, Ordering::Relaxed);
            partial(&body[..cut]);
            for _ in 0..8 {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        progress.store(1000, Ordering::Relaxed);
        Ok(Resp { url: u.to_string(), status: if body.starts_with(b"<title>Not found") { 404 } else { 200 }, content_type: ct.to_string(), body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_blocks_use_the_last_one() {
        let h = "HTTP/1.1 301 Moved\r\nLocation: /x\r\nContent-Length: 0\r\n\r\nHTTP/2 200 \r\ncontent-type: text/html; charset=Shift_JIS\r\ncontent-length: 1234\r\n\r\n";
        assert_eq!(parse_headers(h), (200, "text/html; charset=Shift_JIS".into(), Some(1234)));
        assert_eq!(parse_headers(""), (0, String::new(), None));
    }

    #[test]
    fn mock_serves_fixtures_and_search() {
        let (c, p) = (AtomicBool::new(false), AtomicU32::new(0));
        let r = Mock.fetch(&Req { url: "https://example.org/".into(), post: None, max_bytes: 1 }, &c, &p).unwrap();
        assert!(String::from_utf8_lossy(&r.body).contains("Example Domain"));
        let r = Mock.fetch(&Req { url: "https://lite.duckduckgo.com/lite/?q=rust+lang".into(), post: None, max_bytes: 1 }, &c, &p).unwrap();
        assert!(String::from_utf8_lossy(&r.body).contains("value=\"rust lang\""));
        let r = Mock.fetch(&Req { url: "https://nowhere.test/".into(), post: None, max_bytes: 1 }, &c, &p).unwrap();
        assert_eq!(r.status, 404);
        c.store(true, Ordering::Relaxed);
        assert_eq!(Mock.fetch(&Req { url: "https://example.org/".into(), post: None, max_bytes: 1 }, &c, &p).unwrap_err(), "Cancelled");
    }

    #[test]
    fn real_reads_local_files() {
        let (c, p) = (AtomicBool::new(false), AtomicU32::new(0));
        let path = format!("{}/tests/fixtures/example.html", env!("CARGO_MANIFEST_DIR"));
        let r = Real.fetch(&Req { url: format!("file://{path}"), post: None, max_bytes: 1 }, &c, &p).unwrap();
        assert_eq!(r.content_type, "text/html");
        assert!(Real.fetch(&Req { url: "file:///nonexistent/x".into(), post: None, max_bytes: 1 }, &c, &p).is_err());
        assert!(Real.fetch(&Req { url: "gopher://x".into(), post: None, max_bytes: 1 }, &c, &p).is_err());
    }
}
