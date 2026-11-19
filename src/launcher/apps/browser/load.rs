//! The worker side of a page load: fetch -> decode -> parse -> layout. Runs in a thread and
//! never touches SDL, so the UI stays responsive on the slow CM0.

use std::sync::atomic::{AtomicBool, AtomicU32};

use super::charset;
use super::fetch::{Fetcher, Req, HTML_MAX};
use super::html;
use super::layout::{self, Metrics, Page};

pub enum Loaded {
    Page(Box<Page>),
    /// The URL itself is a picture: shown full screen.
    Image(String, Vec<u8>),
}

pub const WIDTH: i32 = 304;

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn error_html(url: &str, msg: &str) -> String {
    format!("<title>Error</title><h2>Cannot open page</h2><p>{}</p><p>{}</p><p>Esc / Backspace goes back. Ctrl+R tries again.</p>", escape(url), escape(msg))
}

/// Decode, parse and lay out an HTML string.
pub fn build_page(url: &str, source: &str, m: Metrics) -> Page {
    build_page_with(url, source, m, false)
}

/// `reader`: only the main text if the page has any (else the normal view).
pub fn build_page_with(url: &str, source: &str, m: Metrics, reader: bool) -> Page {
    let src = if source.len() > HTML_MAX {
        let mut end = HTML_MAX;
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        &source[..end]
    } else {
        source
    };
    let dom = html::parse(src);
    let mut p = if reader { layout::layout_reader(&dom, url, m, WIDTH).unwrap_or_else(|| layout::layout(&dom, url, m, WIDTH)) } else { layout::layout(&dom, url, m, WIDTH) };
    p.source = Some(std::sync::Arc::new(src.to_string()));
    if source.len() > HTML_MAX {
        p.truncated = true;
    }
    p
}

/// The charset `decode_body` ends up using, for display.
fn charset_label(body: &[u8], content_type: &str) -> String {
    let cs = charset::charset_from(content_type).or_else(|| charset::sniff_meta(body));
    match cs.as_deref().and_then(charset::canonical) {
        Some("sjis") => "Shift_JIS".into(),
        Some("euc-jp") => "EUC-JP".into(),
        Some("latin1") => "Latin-1".into(),
        Some(_) => "UTF-8".into(),
        None if std::str::from_utf8(body).is_ok() => "UTF-8".into(),
        None => "Latin-1".into(),
    }
}

/// Bytes to text: the charset from the Content-Type, else a `<meta>` in the page, else UTF-8
/// (Latin-1 when that fails).
pub fn decode_body(body: &[u8], content_type: &str) -> String {
    let cs = charset::charset_from(content_type).or_else(|| charset::sniff_meta(body));
    match cs.as_deref().and_then(charset::canonical) {
        Some(_) => charset::decode(body, cs.as_deref()),
        None => match std::str::from_utf8(body) {
            Ok(s) => s.strip_prefix('\u{FEFF}').unwrap_or(s).to_string(),
            Err(_) => charset::decode(body, Some("latin1")),
        },
    }
}

/// Everything a worker needs besides the fetcher.
pub struct Job<'a> {
    pub inline: Option<String>,
    pub m: Metrics,
    pub reader: bool,
    pub cancel: &'a AtomicBool,
    pub progress: &'a AtomicU32,
}

/// `on_page` receives early, partial pages while the download is still running.
pub fn load(f: &dyn Fetcher, req: Req, job: Job, on_page: &mut dyn FnMut(Page)) -> Result<Loaded, String> {
    let Job { inline, m, reader, cancel, progress } = job;
    let url = req.url.clone();
    let resp = match inline {
        Some(h) => return Ok(Loaded::Page(Box::new(build_page_with(&url, &h, m, reader)))),
        None => match f.fetch_partial(&req, cancel, progress, &mut |bytes| {
            let text = decode_body(bytes, "");
            if let Some(end) = text.rfind('>') {
                if text[..end].contains('<') {
                    on_page(build_page_with(&url, &text[..=end], m, reader));
                }
            }
        }) {
            Ok(r) => r,
            Err(e) if e == "Cancelled" => return Err(e),
            Err(e) => return Ok(Loaded::Page(Box::new(build_page(&url, &error_html(&url, &e), m)))),
        },
    };
    let ct = resp.content_type.to_ascii_lowercase();
    if ct.starts_with("image/") {
        return Ok(Loaded::Image(resp.url, resp.body));
    }
    let is_html = ct.is_empty() || ct.contains("html") || ct.contains("xml");
    let is_text = ct.starts_with("text/");
    let source = if is_html {
        decode_body(&resp.body, &resp.content_type)
    } else if is_text {
        format!("<pre>{}</pre>", escape(&decode_body(&resp.body, &resp.content_type)))
    } else {
        error_html(&resp.url, &format!("Cannot show {}", if ct.is_empty() { "this content" } else { &ct }))
    };
    if resp.body.is_empty() && resp.status >= 400 {
        return Ok(Loaded::Page(Box::new(build_page(&resp.url, &error_html(&resp.url, &format!("HTTP {}", resp.status)), m))));
    }
    let mut p = build_page_with(&resp.url, &source, m, reader && is_html);
    p.charset = charset_label(&resp.body, &resp.content_type);
    p.bytes = resp.body.len();
    if is_html && source.to_ascii_lowercase().contains("<frameset") {
        p.frames = append_frames(f, &mut p, &source, &resp.url, m, cancel);
    }
    if p.items.is_empty() && p.title.is_empty() && p.fields.is_empty() && p.refresh.is_none() {
        p = build_page(&resp.url, &format!("<title>Empty</title><p>The page is empty.</p><p>{}</p>", escape(&resp.url)), m);
    }
    Ok(Loaded::Page(Box::new(p)))
}

/// `src` of every `<frame>` (also in nested framesets), resolved against `base`.
pub fn frame_urls(source: &str, base: &str) -> Vec<String> {
    let dom = html::parse(source);
    let base = dom.find("base").and_then(|b| dom.attr(b, "href")).and_then(|h| super::url::resolve(base, h)).unwrap_or_else(|| base.to_string());
    (1..dom.nodes.len()).filter(|&i| dom.tag(i) == Some("frame")).filter_map(|i| dom.attr(i, "src").and_then(|s| super::url::resolve(&base, s))).take(MAX_FRAMES).collect()
}

pub const MAX_FRAMES: usize = 6;

/// A frameset has no content of its own: fetch each frame and show them one after another
/// (the menu frame first, as a link list), each laid out against its own URL.
fn append_frames(f: &dyn Fetcher, p: &mut Page, source: &str, base: &str, m: Metrics, cancel: &AtomicBool) -> usize {
    let mut n = 0;
    for u in frame_urls(source, base) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return n;
        }
        n += 1;
        let req = Req { url: u.clone(), post: None, max_bytes: super::fetch::PAGE_MAX / 3 };
        let sub = match f.fetch(&req, cancel, &AtomicU32::new(0)) {
            Ok(r) if r.content_type.to_ascii_lowercase().starts_with("image/") => build_page(&u, &format!("<img src=\"{}\">", escape(&u)), m),
            Ok(r) => build_page(&r.url, &decode_body(&r.body, &r.content_type), m),
            Err(e) => build_page(&u, &error_html(&u, &e), m),
        };
        // a frame's own title does not replace the frameset's
        p.append(sub, 8);
    }
    p.items.sort_by_key(|i| i.y);
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::apps::browser::fetch::Mock;
    use std::sync::atomic::Ordering;

    const M: Metrics = Metrics::fixed(6, 12);

    fn job<'a>(c: &'a AtomicBool, p: &'a AtomicU32) -> Job<'a> {
        Job { inline: None, m: M, reader: false, cancel: c, progress: p }
    }

    #[test]
    fn a_partial_page_arrives_before_the_final_one() {
        let mut partials = Vec::new();
        let r = load(&Mock, Req { url: "https://example.org/".into(), post: None, max_bytes: 1 }, job(&AtomicBool::new(false), &AtomicU32::new(0)), &mut |p| partials.push(p)).unwrap();
        let Loaded::Page(full) = r else { panic!() };
        assert_eq!(partials.len(), 1);
        assert_eq!(partials[0].title, "Example Domain");
        assert!(partials[0].height > 0 && partials[0].height < full.height);
    }

    fn run(url: &str) -> Loaded {
        load(&Mock, Req { url: url.into(), post: None, max_bytes: 1 }, job(&AtomicBool::new(false), &AtomicU32::new(0)), &mut |_| {}).unwrap()
    }

    #[test]
    fn pages_and_images() {
        match run("https://example.org/") {
            Loaded::Page(p) => {
                assert_eq!(p.title, "Example Domain");
                assert!(p.links.len() >= 2 && !p.forms.is_empty());
            }
            _ => panic!(),
        }
        assert!(matches!(run("https://example.org/logo.png"), Loaded::Image(..)));
        match run("https://example.org/news") {
            Loaded::Page(p) => assert!(p.title.contains("ニュース")),
            _ => panic!(),
        }
    }

    #[test]
    fn errors_become_pages_and_cancel_stays_an_error() {
        let c = AtomicBool::new(true);
        assert!(load(&Mock, Req { url: "https://example.org/".into(), post: None, max_bytes: 1 }, job(&c, &AtomicU32::new(0)), &mut |_| {}).is_err());
        c.store(false, Ordering::Relaxed);
        let p = build_page("x", &error_html("x", "Boom <b>"), M);
        assert!(p.items.iter().any(|i| matches!(&i.kind, layout::ItemKind::Text { text, .. } if text.contains("Boom <b>"))));
    }

    #[test]
    fn framesets_are_fetched_and_stacked() {
        let Loaded::Page(p) = run("http://frames.example/") else { panic!() };
        assert_eq!(p.title, "はーばーのホームページ");
        assert_eq!(p.url, "https://frames.example/", "the http -> https redirect is followed");
        let text: String = p.items.iter().filter_map(|i| if let layout::ItemKind::Text { text, .. } = &i.kind { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join(" ");
        for want in ["トップ", "お知らせ", "ギャラリー", "はーばーのホームページ", "サイトについて", "Sample Harbour", "架空の人物"] {
            assert!(text.contains(want), "{want} missing: {text}");
        }
        assert!(p.links.iter().any(|l| l == "https://frames.example/news/news.htm"));
        assert!(p.links.iter().any(|l| l == "https://frames.example/about/about.htm"));
        // the menu comes first
        let y = |w: &str| p.items.iter().find(|i| matches!(&i.kind, layout::ItemKind::Text { text, .. } if text.contains(w))).map(|i| i.y).unwrap();
        assert!(y("トップ") < y("Sample Harbour"));
        assert!(p.items.windows(2).all(|w| w[0].y <= w[1].y));
    }

    #[test]
    fn frame_urls_resolve() {
        let u = frame_urls("<frameset cols=1,2><frame src=menu.htm><frameset rows=1,2><frame src='/a/b.htm'><frame></frameset></frameset>", "https://x.org/d/i.htm");
        assert_eq!(u, ["https://x.org/d/menu.htm", "https://x.org/a/b.htm"]);
    }

    #[test]
    fn body_charsets() {
        assert_eq!(decode_body(b"<meta charset=shift_jis>\x82\xA0", ""), "<meta charset=shift_jis>あ");
        assert_eq!(decode_body(b"caf\xE9", ""), "café");
        assert_eq!(decode_body("日本".as_bytes(), "text/html; charset=utf-8"), "日本");
    }

    #[test]
    fn huge_documents_are_cut() {
        let big = "<p>word </p>".repeat(400_000);
        let p = build_page("u", &big, M);
        assert!(p.truncated);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `BROWSER_BENCH=<file.html> cargo test --release --lib bench_page -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_page() {
        let Ok(path) = std::env::var("BROWSER_BENCH") else { return };
        let bytes = std::fs::read(path).unwrap();
        let t = std::time::Instant::now();
        let src = decode_body(&bytes, "text/html; charset=utf-8");
        let t1 = t.elapsed();
        let p = build_page("https://x.org/", &src, Metrics::fixed(6, 12));
        println!("{} bytes: decode {:?}, parse+layout {:?}, {} items, height {} px, {} links", bytes.len(), t1, t.elapsed() - t1, p.items.len(), p.height, p.links.len());
    }
}
