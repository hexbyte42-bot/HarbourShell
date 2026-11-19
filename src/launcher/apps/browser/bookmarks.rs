//! A named list of web entries: the one structure behind Bookmarks and History, and the shape a
//! later Home "folder of web shortcuts" can reuse (a folder = a `NamedList`).
//!
//! File format (plain text, hand-editable): an optional first line `# <list name>`, then one
//! entry per line, `url<TAB>title`. A line without a tab is a bare URL.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedList {
    pub name: String,
    pub entries: Vec<Entry>,
}

impl NamedList {
    pub fn new(name: &str) -> Self {
        NamedList { name: name.to_string(), entries: Vec::new() }
    }

    /// `default_name` is used when the text has no `# name` header.
    pub fn parse(default_name: &str, text: &str) -> Self {
        let mut list = NamedList::new(default_name);
        for (i, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            if i == 0 {
                if let Some(n) = line.strip_prefix("# ") {
                    list.name = n.trim().to_string();
                    continue;
                }
            }
            let (url, title) = line.split_once('\t').unwrap_or((line, ""));
            list.entries.push(Entry { url: url.to_string(), title: title.to_string() });
        }
        list
    }

    pub fn to_text(&self) -> String {
        let mut s = format!("# {}\n", self.name.replace(['\t', '\n'], " "));
        for e in &self.entries {
            s.push_str(&format!("{}\t{}\n", e.url.replace(['\t', '\n'], "%20"), e.title.replace(['\t', '\n'], " ")));
        }
        s
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, url: &str) -> bool {
        self.entries.iter().any(|e| e.url == url)
    }

    /// Newest first, no duplicates, at most `max`.
    pub fn push_front(&mut self, url: &str, title: &str, max: usize) {
        self.entries.retain(|e| e.url != url);
        self.entries.insert(0, Entry { url: url.to_string(), title: title.to_string() });
        self.entries.truncate(max);
    }

    /// Add (true) or remove (false).
    pub fn toggle(&mut self, url: &str, title: &str) -> bool {
        if let Some(i) = self.entries.iter().position(|e| e.url == url) {
            self.entries.remove(i);
            false
        } else {
            self.entries.insert(0, Entry { url: url.to_string(), title: title.to_string() });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_plain_files() {
        let mut l = NamedList::new("Web");
        l.push_front("https://a.b/", "A\tB", 10);
        l.push_front("https://c.d/", "", 10);
        let back = NamedList::parse("x", &l.to_text());
        assert_eq!(back.name, "Web");
        assert_eq!(back.entries[0].url, "https://c.d/");
        assert_eq!(back.entries[1].title, "A B");
        // an old headerless file: url<TAB>title lines and bare URLs
        let old = NamedList::parse("Bookmarks", "https://a.b/\tA\nhttps://c.d/\n\n");
        assert_eq!((old.name.as_str(), old.len()), ("Bookmarks", 2));
        assert_eq!(old.entries[1].title, "");
    }

    #[test]
    fn push_front_toggle_and_cap() {
        let mut l = NamedList::new("H");
        for i in 0..5 {
            l.push_front(&format!("u{i}"), "", 3);
        }
        assert_eq!(l.len(), 3);
        l.push_front("u3", "again", 3);
        assert_eq!((l.entries[0].url.as_str(), l.entries[0].title.as_str(), l.len()), ("u3", "again", 3));
        assert!(l.toggle("new", "N"));
        assert!(l.contains("new"));
        assert!(!l.toggle("new", "N"));
        assert!(!l.contains("new"));
    }
}
