//! The transport: one blocking HTTPS POST through a `curl` child process (the Browser does the
//! same, so there is no TLS stack to build for the arm64 image).
//!
//! Secrets never appear on a command line: URL, headers and body go to curl as a config file on
//! stdin (`-K -`), so `ps` shows only fixed flags.

use std::io::Write;
use std::process::{Command, Stdio};

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait Transport: Send + Sync {
    fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResponse, String>;
}

/// Quote a value for a curl config file (inside double quotes).
pub fn config_quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{0b}' => o.push_str("\\v"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The curl config for one POST. Header names / values with control characters are rejected so a
/// value can never smuggle in another config line.
pub fn curl_config(url: &str, headers: &[(String, String)], body: &str) -> Result<String, String> {
    let bad = |s: &str| s.chars().any(|c| c.is_control());
    if bad(url) || headers.iter().any(|(k, v)| bad(k) || bad(v)) {
        return Err("control character in the request".into());
    }
    let mut cfg = String::new();
    cfg.push_str(&format!("url = {}\n", config_quote(url)));
    cfg.push_str("request = \"POST\"\n");
    for (k, v) in headers {
        cfg.push_str(&format!("header = {}\n", config_quote(&format!("{k}: {v}"))));
    }
    cfg.push_str(&format!("data = {}\n", config_quote(body)));
    Ok(cfg)
}

pub struct CurlTransport {
    pub timeout_s: u32,
}

impl Default for CurlTransport {
    fn default() -> Self {
        Self { timeout_s: 90 }
    }
}

const STATUS_MARK: &str = "\n__HTTP_STATUS__";

impl Transport for CurlTransport {
    fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResponse, String> {
        let cfg = curl_config(url, headers, body)?;
        let mut child = Command::new("curl")
            .args(["-sS", "-K", "-", "--max-time", &self.timeout_s.to_string(), "--connect-timeout", "15", "--proto", "=http,https"])
            .args(["-w", &format!("{STATUS_MARK}%{{http_code}}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run curl: {e}"))?;
        if let Some(mut si) = child.stdin.take() {
            // Written from a thread: a large body must not deadlock against curl's output.
            std::thread::spawn(move || {
                let _ = si.write_all(cfg.as_bytes());
            });
        }
        let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let Some((body, code)) = text.rsplit_once(STATUS_MARK) else {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.lines().next().unwrap_or("no response").trim();
            return Err(format!("network error: {err}"));
        };
        let status: u16 = code.trim().parse().unwrap_or(0);
        if status == 0 {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.lines().next().unwrap_or("no response").trim();
            return Err(format!("network error: {err}"));
        }
        Ok(HttpResponse { status, body: body.to_string() })
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Returns canned responses in order and records what was sent.
    pub struct FakeTransport {
        pub responses: Mutex<Vec<Result<HttpResponse, String>>>,
        pub sent: Mutex<Vec<(String, Vec<(String, String)>, String)>>,
    }

    impl FakeTransport {
        pub fn new(responses: Vec<Result<HttpResponse, String>>) -> Self {
            Self { responses: Mutex::new(responses), sent: Mutex::new(vec![]) }
        }
        pub fn ok(body: &str) -> Result<HttpResponse, String> {
            Ok(HttpResponse { status: 200, body: body.into() })
        }
    }

    impl Transport for FakeTransport {
        fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResponse, String> {
            self.sent.lock().unwrap().push((url.into(), headers.to_vec(), body.into()));
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                return Err("no canned response".into());
            }
            r.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_values_are_escaped_so_they_stay_on_one_line() {
        assert_eq!(config_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        let cfg = curl_config("https://x/v1", &[("Authorization".into(), "Bearer k".into())], "{\"a\":\"l1\\nl2\"}").unwrap();
        assert_eq!(cfg.lines().count(), 4, "{cfg}");
        assert!(cfg.contains("header = \"Authorization: Bearer k\""));
        assert!(cfg.contains("data = \"{\\\"a\\\":\\\"l1\\\\nl2\\\"}\""));
    }

    /// The real curl path against a throw-away local server: the body and the key header arrive
    /// intact, the status comes back, and the key is not on curl's command line.
    #[test]
    fn curl_posts_json_with_headers_from_stdin() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut got = String::new();
            while !got.contains("\"q\"") {
                let n = c.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            c.write_all(b"HTTP/1.1 418 Teapot\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
            got
        });
        let t = CurlTransport { timeout_s: 10 };
        let r = t
            .post(&format!("http://127.0.0.1:{port}/v1/x"), &[("x-api-key".into(), "SECRET-KEY".into()), ("Content-Type".into(), "application/json".into())], "{\"q\":\"日本\\n\"}")
            .unwrap();
        let got = h.join().unwrap();
        assert_eq!(r.status, 418);
        assert!(r.body.starts_with("{\"ok\""), "{}", r.body);
        assert!(got.to_lowercase().contains("x-api-key: secret-key"), "{got}");
        assert!(got.contains("{\"q\":\"日本\\n\"}"), "{got}");
        assert!(got.starts_with("POST /v1/x"));
    }

    #[test]
    fn control_characters_in_headers_are_refused() {
        assert!(curl_config("https://x", &[("A".into(), "b\nc".into())], "{}").is_err());
    }
}
