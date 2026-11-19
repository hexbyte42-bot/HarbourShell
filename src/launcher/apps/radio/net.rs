//! Directory lookup for Harbour Radio. No new crates: curl handles HTTPS,
//! serde_json handles Radio Browser and the Apple podcast catalogue.
//! Podcast episodes use a deliberately small RSS 2.0 enclosure reader (the XML helpers live in
//! `launcher::feed`, shared with Infoscope).

use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const UA: &str = "HarbourRadio/0.1 (Cardputer Zero; https://github.com/kurogedelic/HarbourShell)";
const LIMIT: usize = 800_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: u32,
    /// Latitude / longitude from the directory, when it has them (drawn on the map).
    #[serde(default)]
    pub geo: Option<(f32, f32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Podcast {
    pub title: String,
    pub author: String,
    pub feed: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Episode {
    pub title: String,
    pub audio: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub stations: Vec<Station>,
    #[serde(default)]
    pub podcasts: Vec<Podcast>,
}

pub fn http_url(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://"))
        && !s.chars().any(char::is_control)
        && s.len() <= 4096
}

pub fn encode_query(s: &str) -> String {
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

/// Fetch on a background thread only. The caller is responsible for not blocking the UI.
pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if !http_url(url) {
        return Err("Invalid HTTP address".into());
    }
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--max-time", "15",
               "--connect-timeout", "6", "--max-redirs", "5", "--max-filesize", "800000",
               "--proto", "=http,https", "--proto-redir", "=http,https",
               "--user-agent", UA, "--"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("Network request failed ({})", out.status));
    }
    if out.stdout.len() > LIMIT {
        return Err("Feed is too large".into());
    }
    Ok(out.stdout)
}

pub fn parse_stations(bytes: &[u8]) -> Result<Vec<Station>, String> {
    let rows: Vec<Value> = serde_json::from_slice(bytes).map_err(|e| format!("Stations JSON: {e}"))?;
    let mut stations = Vec::new();
    for v in rows {
        let name = v["name"].as_str().unwrap_or("").trim();
        let id = v["stationuuid"].as_str().unwrap_or("");
        let url = v["url_resolved"].as_str().filter(|s| http_url(s))
            .or_else(|| v["url"].as_str().filter(|s| http_url(s)));
        if name.is_empty() || id.is_empty() || url.is_none() {
            continue;
        }
        if v["lastcheckok"] == false || v["lastcheckok"] == 0 {
            continue;
        }
        if stations.iter().any(|s: &Station| s.id == id) {
            continue;
        }
        stations.push(Station {
            id: id.to_string(),
            name: name.to_string(),
            url: url.unwrap().to_string(),
            tags: v["tags"].as_str().unwrap_or("").to_string(),
            codec: v["codec"].as_str().unwrap_or("").to_string(),
            bitrate: v["bitrate"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32,
            geo: match (v["geo_lat"].as_f64(), v["geo_long"].as_f64()) {
                (Some(la), Some(lo)) if la.abs() <= 90.0 && lo.abs() <= 180.0 => Some((la as f32, lo as f32)),
                _ => None,
            },
        });
    }
    Ok(stations)
}

/// Radio Browser is a community service with multiple mirrors. Use a bounded
/// response and try a second mirror if the primary cannot be reached.
pub fn stations(genre: Option<&str>, name: Option<&str>) -> Result<Vec<Station>, String> {
    let path = if let Some(q) = name {
        format!("/json/stations/search?name={}&hidebroken=true&order=clickcount&reverse=true&limit=60", encode_query(q))
    } else {
        format!("/json/stations/bytag/{}?hidebroken=true&order=clickcount&reverse=true&limit=60", encode_query(genre.unwrap_or("ambient")))
    };
    let mut last = "Radio directory unavailable".to_string();
    for server in ["de1.api.radio-browser.info", "nl1.api.radio-browser.info"] {
        match fetch(&format!("https://{server}{path}")).and_then(|bytes| parse_stations(&bytes)) {
            Ok(stations) => return Ok(stations),
            Err(err) => last = err,
        }
    }
    Err(last)
}

pub fn parse_podcasts(bytes: &[u8]) -> Result<Vec<Podcast>, String> {
    let root: Value = serde_json::from_slice(bytes).map_err(|e| format!("Podcasts JSON: {e}"))?;
    let mut out = Vec::new();
    if let Some(rows) = root["results"].as_array() {
        for row in rows {
            let title = row["collectionName"].as_str().unwrap_or("").trim();
            let feed = row["feedUrl"].as_str().unwrap_or("");
            if title.is_empty() || !http_url(feed) || out.iter().any(|p: &Podcast| p.feed == feed) {
                continue;
            }
            out.push(Podcast {
                title: title.to_string(),
                author: row["artistName"].as_str().unwrap_or("").to_string(),
                feed: feed.to_string(),
            });
        }
    }
    Ok(out)
}

/// Apple's iTunes Search API is a catalogue, not an account sync API.
pub fn podcasts(term: &str) -> Result<Vec<Podcast>, String> {
    let url = format!("https://itunes.apple.com/search?media=podcast&entity=podcast&limit=30&term={}", encode_query(term));
    parse_podcasts(&fetch(&url)?)
}

use crate::launcher::feed::{item_blocks, tag, xml_text};

fn enclosure_url(item: &str) -> Option<String> {
    let start = item.find("<enclosure")?;
    let end = item[start..].find('>')? + start;
    let attrs = &item[start + "<enclosure".len()..end];
    // Support either quote style, without matching "data-url" or similar.
    for (i, _) in attrs.match_indices("url=") {
        if i > 0 && !attrs.as_bytes()[i - 1].is_ascii_whitespace() {
            continue;
        }
        let value = &attrs[i + 4..];
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let rest = &value[1..];
        let finish = rest.find(quote)?;
        let url = xml_text(&rest[..finish]);
        if http_url(&url) {
            return Some(url);
        }
    }
    None
}

/// RSS 2.0 audio enclosures only. Atom / feeds without enclosures are
/// explicitly not supported; never guess audio URLs from links.
pub fn parse_episodes(xml: &str) -> Vec<Episode> {
    let mut out = Vec::new();
    for item in item_blocks(xml) {
        if let (Some(title), Some(audio)) = (tag(item, "title"), enclosure_url(item)) {
            if !title.trim().is_empty() {
                out.push(Episode { title, audio });
            }
        }
        if out.len() >= 60 { break; }
    }
    out
}

pub fn episodes(feed: &str) -> Result<Vec<Episode>, String> {
    let bytes = fetch(feed)?;
    let text = String::from_utf8(bytes).map_err(|_| "Podcast feed is not UTF-8".to_string())?;
    Ok(parse_episodes(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_filters_unplayable_rows_and_dupes() {
        let input = br#"[{"stationuuid":"a","name":"  Jazz FM  ","url_resolved":"https://radio.test/live","bitrate":128,"lastcheckok":1},
          {"stationuuid":"a","name":"duplicate","url":"http://radio.test/live"},
          {"stationuuid":"b","name":"broken","url":"https://radio.test/no","lastcheckok":0},
          {"stationuuid":"c","name":"file","url":"file:///tmp/a"}]"#;
        let v = parse_stations(input).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Jazz FM");
        assert_eq!(v[0].bitrate, 128);
    }

    #[test]
    fn podcast_catalogue_and_rss_enclosures() {
        let list = br#"{"results":[{"collectionName":"Test","artistName":"Maker","feedUrl":"https://example.org/rss"}]}"#;
        assert_eq!(parse_podcasts(list).unwrap()[0].title, "Test");
        let feed = r#"<rss><channel>
          <item><title><![CDATA[Synths & Sounds]]></title><enclosure length="12" url="https://a.test/one.mp3?x=1&amp;y=2" type="audio/mpeg"/></item>
          <item><title>Two &amp; Three</title><enclosure url='https://a.test/two.mp3'/></item>
          <item><title>No audio</title></item>
        </channel></rss>"#;
        let got = parse_episodes(feed);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "Synths & Sounds");
        assert_eq!(got[0].audio, "https://a.test/one.mp3?x=1&y=2");
        assert_eq!(got[1].title, "Two & Three");
    }

    #[test]
    fn query_encoding_and_urls() {
        assert_eq!(encode_query("日本 jazz & pop"), "%E6%97%A5%E6%9C%AC%20jazz%20%26%20pop");
        assert!(!http_url("file:///tmp/foo"));
        assert!(!http_url("https://x\nhack"));
    }
}
