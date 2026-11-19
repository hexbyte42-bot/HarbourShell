//! Search: file names on the device (an index of the home folder, built once on a worker thread)
//! and the web search address handed to the Browser.

use std::path::{Path, PathBuf};

/// Limits so a large card never stalls the index: entries kept, folders deep, folders read.
const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 8;
const MAX_DIRS: usize = 4_000;
/// File results shown above the web row.
pub const MAX_HITS: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Lower-case name, for matching.
    key: String,
    pub path: PathBuf,
    pub dir: bool,
}

/// Every file and folder under `root` (hidden ones and symlinked folders skipped), breadth first.
pub fn index(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut dirs = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        dirs += 1;
        if dirs > MAX_DIRS {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<_> = rd.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let is_dir = ft.is_dir();
            let path = e.path();
            if is_dir && depth + 1 < MAX_DEPTH {
                queue.push_back((path.clone(), depth + 1));
            }
            out.push(Entry { key: name.to_lowercase(), name, path, dir: is_dir });
            if out.len() >= MAX_ENTRIES {
                return out;
            }
        }
    }
    out
}

/// Names containing every word of `query` (case-insensitive): names starting with the query
/// first, then shallower paths, then shorter names. At most [`MAX_HITS`].
pub fn find<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<&Entry> = entries.iter().filter(|e| words.iter().all(|w| e.key.contains(w))).collect();
    hits.sort_by_key(|e| (!e.key.starts_with(words[0]), e.path.components().count(), e.name.chars().count()));
    hits.truncate(MAX_HITS);
    hits
}

/// The address that searches the web for `query` (the Browser's own search page).
pub fn web_url(query: &str) -> String {
    let mut q = String::new();
    for b in query.trim().bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            q.push(b as char);
        } else if b == b' ' {
            q.push('+');
        } else {
            q.push_str(&format!("%{b:02X}"));
        }
    }
    format!("https://lite.duckduckgo.com/lite/?q={q}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_skips_hidden_and_find_ranks_prefixes_first() {
        let root = std::env::temp_dir().join(format!("infoscope-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Documents/old")).unwrap();
        std::fs::create_dir_all(root.join(".secret")).unwrap();
        std::fs::write(root.join(".secret/harbour notes.txt"), "").unwrap();
        std::fs::write(root.join("Documents/old/my harbour notes.txt"), "").unwrap();
        std::fs::write(root.join("Documents/Harbour plan.md"), "").unwrap();
        std::fs::write(root.join("photo.png"), "").unwrap();
        let idx = index(&root);
        assert!(idx.iter().all(|e| !e.path.to_string_lossy().contains(".secret")));
        assert!(idx.iter().any(|e| e.dir && e.name == "Documents"));
        let hits: Vec<&str> = find(&idx, "HARBOUR").iter().map(|e| e.name.as_str()).collect();
        assert_eq!(hits, ["Harbour plan.md", "my harbour notes.txt"]);
        let hits: Vec<&str> = find(&idx, "notes harbour").iter().map(|e| e.name.as_str()).collect();
        assert_eq!(hits, ["my harbour notes.txt"]);
        assert!(find(&idx, "   ").is_empty());
        assert!(find(&idx, "zzz").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn web_address_is_encoded() {
        assert_eq!(web_url(" 灯台 & sea "), "https://lite.duckduckgo.com/lite/?q=%E7%81%AF%E5%8F%B0+%26+sea");
    }
}
