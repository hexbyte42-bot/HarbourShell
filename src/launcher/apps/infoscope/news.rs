//! News: the feed subscriptions, the cached headlines, and fetching (off the UI thread, curl).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::launcher::apps::browser::{html, layout, load};
use crate::launcher::feed::{self, NewsItem};

const UA: &str = "HarbourInfoscope/0.1 (Cardputer Zero; https://github.com/kurogedelic/HarbourShell)";
const LIMIT: usize = 2_000_000;
/// Headlines kept in the list and in the cache.
pub const KEEP: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    /// The feed's own title once it has been read (the address until then).
    #[serde(default)]
    pub title: String,
    pub url: String,
}

impl Feed {
    pub fn new(url: &str) -> Self {
        Feed { title: String::new(), url: url.to_string() }
    }
    pub fn label(&self) -> &str {
        if self.title.is_empty() { &self.url } else { &self.title }
    }
}

/// The subscriptions (`~/.config/harbour/infoscope.json`). A missing file means the defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscriptions {
    pub feeds: Vec<Feed>,
}

/// A few general news feeds to start with; remove them from Feeds like any other.
pub const DEFAULT_FEEDS: &[(&str, &str)] = &[
    ("NHK News", "https://www.nhk.or.jp/rss/news/cat0.xml"),
    ("BBC News - World", "https://feeds.bbci.co.uk/news/world/rss.xml"),
    ("Hacker News", "https://hnrss.org/frontpage"),
];

impl Default for Subscriptions {
    fn default() -> Self {
        Subscriptions { feeds: DEFAULT_FEEDS.iter().map(|(t, u)| Feed { title: t.to_string(), url: u.to_string() }).collect() }
    }
}

impl Subscriptions {
    /// Add a feed address. Errors for a non-web address or one already there.
    pub fn add(&mut self, url: &str) -> Result<(), String> {
        let url = url.trim();
        let url = if !url.contains("://") && url.contains('.') { format!("https://{url}") } else { url.to_string() };
        if !feed::http_url(&url) {
            return Err("Not a web address".into());
        }
        if self.feeds.iter().any(|f| f.url == url) {
            return Err("Already subscribed".into());
        }
        self.feeds.push(Feed::new(&url));
        Ok(())
    }

    pub fn remove(&mut self, i: usize) -> Option<Feed> {
        (i < self.feeds.len()).then(|| self.feeds.remove(i))
    }
}

/// The last headlines read, so the list shows without a network.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// When they were fetched (seconds since the epoch).
    #[serde(default)]
    pub fetched: Option<i64>,
    #[serde(default)]
    pub items: Vec<NewsItem>,
}

/// Articles read, kept so they open again without a network (the newest [`ARTICLES_KEEP`]).
pub const ARTICLES_KEEP: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Article {
    pub link: String,
    /// When it was read (seconds since the epoch).
    pub fetched: i64,
    pub paragraphs: Vec<String>,
}

/// The saved articles, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Articles {
    #[serde(default)]
    pub list: Vec<Article>,
}

impl Articles {
    pub fn get(&self, link: &str) -> Option<&Article> {
        self.list.iter().find(|a| a.link == link)
    }

    /// Keep `a` as the newest; the oldest go once there are more than [`ARTICLES_KEEP`].
    pub fn put(&mut self, a: Article) {
        self.list.retain(|o| o.link != a.link);
        self.list.push(a);
        let over = self.list.len().saturating_sub(ARTICLES_KEEP);
        self.list.drain(..over);
    }
}

/// Where Infoscope keeps its files; `None` (mock mode, no HOME) keeps everything in memory.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: Option<PathBuf>,
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, p).map_err(|e| e.to_string())
}

impl Store {
    pub fn home() -> Store {
        Store { dir: std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/harbour")) }
    }
    fn subs_path(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("infoscope.json"))
    }
    fn cache_path(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("infoscope-cache.json"))
    }
    pub fn load_subs(&self) -> Subscriptions {
        self.subs_path().and_then(|p| read_json(&p)).unwrap_or_default()
    }
    pub fn save_subs(&self, s: &Subscriptions) -> Result<(), String> {
        match self.subs_path() {
            Some(p) => write_json(&p, s),
            None => Ok(()),
        }
    }
    fn articles_path(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("infoscope-articles.json"))
    }
    pub fn load_articles(&self) -> Articles {
        self.articles_path().and_then(|p| read_json(&p)).unwrap_or_default()
    }
    pub fn save_articles(&self, a: &Articles) -> Result<(), String> {
        match self.articles_path() {
            Some(p) => write_json(&p, a),
            None => Ok(()),
        }
    }
    pub fn load_cache(&self) -> Cache {
        self.cache_path().and_then(|p| read_json(&p)).unwrap_or_default()
    }
    pub fn save_cache(&self, c: &Cache) -> Result<(), String> {
        match self.cache_path() {
            Some(p) => write_json(&p, c),
            None => Ok(()),
        }
    }
}

/// Download one address with curl (call it on a worker thread only).
pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if !feed::http_url(url) {
        return Err("Not a web address".into());
    }
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--compressed", "--max-time", "20",
               "--connect-timeout", "8", "--max-redirs", "5", "--max-filesize", "2000000",
               "--proto", "=http,https", "--proto-redir", "=http,https",
               "--user-agent", UA, "--"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("Network request failed".into());
    }
    if out.stdout.len() > LIMIT {
        return Err("Feed is too large".into());
    }
    Ok(out.stdout)
}

/// What one feed gave: its title (for the Feeds list) and its items, or why it failed.
pub type FeedResult = (String, Result<feed::ParsedFeed, String>);

pub fn read_feed(url: &str) -> FeedResult {
    let r = fetch(url).and_then(|b| {
        let text = String::from_utf8_lossy(&b);
        let f = feed::parse_news(&text);
        if f.items.is_empty() && f.title.is_empty() { Err("Not a news feed".to_string()) } else { Ok(f) }
    });
    (url.to_string(), r)
}

/// The article's own text, taken from its page as the Browser's reader view does
/// (call it on a worker thread only).
pub fn read_article(url: &str) -> Result<Vec<String>, String> {
    let body = fetch(url)?;
    article_text(&load::decode_body(&body, ""))
}

/// The main text of an article page: its paragraphs, or an error when it has none.
pub fn article_text(page: &str) -> Result<Vec<String>, String> {
    layout::reader_text(&html::parse(page)).filter(|p| !p.is_empty()).ok_or_else(|| "No article text found".to_string())
}

/// The text the feed itself carries for an item (its HTML content, else the summary).
pub fn feed_text(it: &NewsItem) -> Vec<String> {
    let mut p = if it.content.is_empty() { Vec::new() } else { layout::text_blocks(&html::parse(&it.content), 0, false) };
    if p.is_empty() && !it.summary.is_empty() {
        p.push(it.summary.clone());
    }
    p
}

/// Mock mode's article pages (no network): a long English one, a long Japanese one, and a page
/// without an article (so the feed's own text is shown).
pub fn mock_article(url: &str) -> Result<Vec<String>, String> {
    let page = if url.ends_with("/lights") {
        let mut h = String::from("<html><head><title>Harbour lights</title></head><body><nav><a href=/>Home</a> <a href=/world>World</a></nav><article><h1>Harbour lights restored after the autumn storm</h1><figure><img src=lamp.jpg><figcaption>The lamp at dawn</figcaption></figure>");
        for p in [
            "Crews worked through the night to bring the lighthouse and the quay lamps back on, after the storm cut power to the whole of the old harbour on Tuesday evening.",
            "The keeper, who has looked after the light for twenty-two years, said the lamp itself was never in danger. \"It was the cable from the town that went,\" she said. \"The lens just sat there in the dark, waiting.\"",
            "Engineers from the mainland arrived on the first ferry with a new length of cable and a spare transformer. By midnight the quay lamps were on again; the lighthouse followed shortly after two in the morning.",
            "Fishing boats that had sheltered in the bay were able to come in at first light. Several skippers said they had steered by the church tower until the light came back.",
            "The harbour office says a second cable, buried along the sea wall, will be laid before the winter storms, so that one fault can no longer darken the whole harbour.",
            "Visitors to the night market on the old pier on Saturday will find it open as usual.",
        ] {
            h.push_str(&format!("<p>{p}</p>"));
        }
        h.push_str("</article><aside class=related><p>Related: Ferry timetable changes from December and other stories.</p></aside><footer>(c) Sample World</footer></body></html>");
        h
    } else if url.ends_with("/jp1") {
        let mut h = String::from("<article><h1>港の灯台、嵐のあと点灯を再開</h1>");
        for p in [
            "火曜日の夜の嵐で港一帯が停電し、灯台と岸壁の灯りが消えていましたが、夜通しの作業で水曜日の未明までにすべて点灯しました。",
            "灯台を二十二年守ってきた灯台守は「レンズは無事でした。切れたのは町から来る電線です」と話しています。本土から最初のフェリーで技術者が到着し、新しい電線と予備の変圧器に取り替えました。",
            "湾に避難していた漁船は、明け方には港に戻ることができました。灯りが戻るまで、教会の塔を目印に舵を取ったという船長もいます。",
            "港の事務所は、冬の嵐の前に防波堤沿いに二本目の電線を埋め、一か所の故障で港全体が暗くならないようにするとしています。",
        ] {
            h.push_str(&format!("<p>{p}</p>"));
        }
        h.push_str("</article>");
        h
    } else {
        return Err("No article text found".into());
    };
    article_text(&page)
}

/// The fixed sample shown in mock mode (no network): two small feeds.
pub fn mock_feed(url: &str) -> FeedResult {
    const NOW: i64 = 1_795_046_400; // 2026-11-19 00:00 UTC
    let it = |t: &str, slug: &str, src: &str, min: i64, s: &str| NewsItem {
        title: t.into(), link: format!("https://example.org/{slug}"), source: src.into(), date: Some(NOW + min * 60), summary: s.into(),
        content: if slug == "ferry" { "<p>Two early sailings move by fifteen minutes.</p><p>The 6:40 leaves at 6:55 and the 7:30 at 7:45, from Monday 7 December.</p>".into() } else { String::new() },
    };
    let f = if url.contains("bbc") || url.ends_with("/b") {
        feed::ParsedFeed { title: "Sample World".into(), items: vec![
            it("Harbour lights restored after the autumn storm", "lights", "Sample World", 300,
               "Crews worked through the night to bring the lighthouse and the quay lamps back on."),
            it("Ferry timetable changes from December", "ferry", "Sample World", 120, "Two early sailings move by fifteen minutes."),
            it("Night market opens on the old pier", "market", "Sample World", -900, ""),
            it("Lifeboat crew trains in the winter swell", "lifeboat", "Sample World", -1500, "The crew practised a night rescue off the breakwater."),
        ] }
    } else if url.contains("hnrss") || url.ends_with("/c") {
        feed::ParsedFeed { title: "Sample Tech".into(), items: vec![
            it("A tiny RSS reader for a 320x170 screen", "rss", "Sample Tech", 240, "Feeds, a cache and a browser hand-off in a few hundred lines."),
            it("Show: one-bit icons drawn by hand", "icons", "Sample Tech", -300, ""),
        ] }
    } else {
        feed::ParsedFeed { title: "サンプルニュース".into(), items: vec![
            it("港の灯台、嵐のあと点灯を再開", "jp1", "サンプルニュース", 360, "夜通しの作業で、灯台と岸壁の灯りが戻りました。"),
            it("12月からフェリーの時刻が一部変わります", "jp2", "サンプルニュース", 60, ""),
        ] }
    };
    (url.to_string(), Ok(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("infoscope-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn subscriptions_default_add_remove_and_persist() {
        let store = Store { dir: Some(tmp("subs")) };
        let mut s = store.load_subs();
        assert_eq!(s.feeds.len(), DEFAULT_FEEDS.len());
        assert!(s.add("example.com/feed.xml").is_ok());
        assert_eq!(s.feeds.last().unwrap().url, "https://example.com/feed.xml");
        assert_eq!(s.add("https://example.com/feed.xml"), Err("Already subscribed".into()));
        assert_eq!(s.add("file:///etc/passwd"), Err("Not a web address".into()));
        assert_eq!(s.add("hello"), Err("Not a web address".into()));
        s.remove(0);
        s.remove(0);
        s.remove(0);
        assert!(s.remove(5).is_none());
        store.save_subs(&s).unwrap();
        let back = store.load_subs();
        assert_eq!(back.feeds.len(), 1);
        // Removing every feed is kept (not replaced by the defaults again).
        let mut none = back.clone();
        none.remove(0);
        store.save_subs(&none).unwrap();
        assert!(store.load_subs().feeds.is_empty());
        let _ = std::fs::remove_dir_all(store.dir.unwrap());
    }

    #[test]
    fn cache_round_trips_and_a_broken_file_is_empty() {
        let store = Store { dir: Some(tmp("cache")) };
        assert_eq!(store.load_cache(), Cache::default());
        let c = Cache { fetched: Some(5), items: mock_feed("https://x/a").1.unwrap().items };
        store.save_cache(&c).unwrap();
        assert_eq!(store.load_cache(), c);
        std::fs::write(store.dir.as_ref().unwrap().join("infoscope-cache.json"), b"{not json").unwrap();
        assert_eq!(store.load_cache(), Cache::default());
        let _ = std::fs::remove_dir_all(store.dir.unwrap());
    }

    #[test]
    fn memory_store_never_writes() {
        let store = Store { dir: None };
        assert!(store.save_cache(&Cache::default()).is_ok());
        assert_eq!(store.load_subs(), Subscriptions::default());
    }

    #[test]
    fn fetch_refuses_non_web_addresses() {
        assert!(fetch("file:///etc/hosts").is_err());
        assert!(fetch("https://a b").is_err());
    }

    #[test]
    fn articles_keep_the_newest_and_persist() {
        let store = Store { dir: Some(tmp("articles")) };
        let mut a = store.load_articles();
        for i in 0..ARTICLES_KEEP + 3 {
            a.put(Article { link: format!("https://x/{i}"), fetched: i as i64, paragraphs: vec!["p".into()] });
        }
        a.put(Article { link: "https://x/5".into(), fetched: 99, paragraphs: vec!["again".into()] });
        assert_eq!(a.list.len(), ARTICLES_KEEP);
        assert!(a.get("https://x/0").is_none(), "the oldest went");
        assert_eq!(a.list.last().unwrap().paragraphs, ["again"]);
        store.save_articles(&a).unwrap();
        assert_eq!(store.load_articles(), a);
        let _ = std::fs::remove_dir_all(store.dir.unwrap());
    }

    #[test]
    fn article_text_keeps_paragraphs_and_drops_furniture() {
        let p = mock_article("https://example.org/lights").unwrap();
        assert_eq!(p[0], "Harbour lights restored after the autumn storm");
        assert!(p[1].starts_with("Crews worked"));
        assert!(p.iter().all(|t| !t.contains("Home") && !t.contains("Related") && !t.contains("lamp at dawn") && !t.contains("(c)")), "{p:?}");
        assert!(mock_article("https://example.org/market").is_err());
        // Without an article page, the feed's own text.
        let f = mock_feed("https://x/bbc").1.unwrap();
        let ferry = f.items.iter().find(|i| i.link.ends_with("/ferry")).unwrap();
        assert_eq!(feed_text(ferry).len(), 2);
        let market = f.items.iter().find(|i| i.link.ends_with("/market")).unwrap();
        assert!(feed_text(market).is_empty());
    }
}
