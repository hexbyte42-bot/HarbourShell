//! A small, tolerant HTML tokenizer and tree builder. No scripting, no CSS. It never fails:
//! malformed markup produces *some* tree. Text is entity-decoded; `script`, `style` and friends
//! are dropped while parsing so they cost no memory.

pub type NodeId = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text(String),
    Elem { tag: String, attrs: Vec<(String, String)> },
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub parent: NodeId,
    pub children: Vec<NodeId>,
}

#[derive(Debug, Default)]
pub struct Dom {
    /// Node 0 is the synthetic root (an element named "#root").
    pub nodes: Vec<Node>,
    pub truncated: bool,
}

pub const MAX_NODES: usize = 80_000;
const MAX_DEPTH: usize = 120;

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "frame", "basefont"];
const SKIP: &[&str] = &["script", "style", "iframe", "svg", "template", "object", "canvas", "head", "select_hidden", "audio", "video", "map"];
/// Raw-text elements: their content is not markup.
const RAW: &[&str] = &["script", "style", "textarea", "title", "iframe"];

impl Dom {
    pub fn tag(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].kind {
            Kind::Elem { tag, .. } => Some(tag),
            _ => None,
        }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].kind {
            Kind::Elem { attrs, .. } => attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    /// First element with this tag, depth first.
    pub fn find(&self, tag: &str) -> Option<NodeId> {
        (1..self.nodes.len()).find(|&i| self.tag(i) == Some(tag))
    }

    /// All text below a node, whitespace collapsed.
    pub fn text_of(&self, id: NodeId) -> String {
        let mut s = String::new();
        self.collect(id, &mut s);
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn collect(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            Kind::Text(t) => {
                out.push_str(t);
                out.push(' ');
            }
            _ => {
                for &c in &self.nodes[id].children {
                    self.collect(c, out);
                }
            }
        }
    }
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{A0}",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "middot" => "·",
        "times" => "×",
        "divide" => "÷",
        "deg" => "°",
        "plusmn" => "±",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "yen" => "¥",
        "euro" => "€",
        "pound" => "£",
        "cent" => "¢",
        "sect" => "§",
        "para" => "¶",
        "iexcl" => "¡",
        "iquest" => "¿",
        "eacute" => "é",
        "egrave" => "è",
        "agrave" => "à",
        "aacute" => "á",
        "uuml" => "ü",
        "ouml" => "ö",
        "auml" => "ä",
        "szlig" => "ß",
        "ccedil" => "ç",
        "ntilde" => "ñ",
        "zwnj" | "zwj" | "shy" => "",
        "ensp" | "emsp" | "thinsp" => " ",
        _ => return None,
    })
}

/// Decode `&amp;`, `&#65;`, `&#x41;`; unknown references stay as they are.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '#')).map(|e| e + 1).unwrap_or(rest.len());
        let name = &rest[1..end];
        let has_semi = rest[end..].starts_with(';');
        let rep: Option<String> = if let Some(num) = name.strip_prefix('#') {
            let v = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) { u32::from_str_radix(h, 16).ok() } else { num.parse::<u32>().ok() };
            v.and_then(char::from_u32).map(|c| if c == '\0' { '\u{FFFD}' } else { c }).map(|c| c.to_string())
        } else if name.len() <= 8 && !name.is_empty() {
            named_entity(name).map(String::from)
        } else {
            None
        };
        match rep {
            Some(r) if has_semi || name.starts_with('#') || matches!(name, "amp" | "lt" | "gt" | "nbsp" | "quot") => {
                out.push_str(&r);
                rest = &rest[end + if has_semi { 1 } else { 0 }..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

/// Parse attributes from the inside of a start tag (after the tag name).
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        while i < b.len() && (is_ws(b[i]) || b[i] == b'/') {
            i += 1;
        }
        let ns = i;
        while i < b.len() && !is_ws(b[i]) && b[i] != b'=' && b[i] != b'/' && b[i] != b'>' {
            i += 1;
        }
        if ns == i {
            i += 1;
            continue;
        }
        let name = s[ns..i].to_ascii_lowercase();
        while i < b.len() && is_ws(b[i]) {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && is_ws(b[i]) {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = decode_entities(&s[vs..i]);
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !is_ws(b[i]) && b[i] != b'>' {
                    i += 1;
                }
                val = decode_entities(&s[vs..i]);
            }
        }
        if !out.iter().any(|(k, _): &(String, String)| *k == name) {
            out.push((name, val));
        }
    }
    out
}

/// Which open element does a new `tag` implicitly close?
fn closes(open: &str, new: &str) -> bool {
    match (open, new) {
        ("p", t) => matches!(t, "p" | "div" | "ul" | "ol" | "dl" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "blockquote" | "hr" | "form" | "li" | "dt" | "dd" | "tr" | "td" | "th" | "fieldset" | "address" | "section" | "article" | "header" | "footer" | "nav" | "aside" | "main"),
        ("li", "li") | ("dt", "dt" | "dd") | ("dd", "dt" | "dd") => true,
        ("option", "option" | "optgroup") => true,
        ("tr", "tr") | ("td" | "th", "td" | "th" | "tr") => true,
        ("thead" | "tbody" | "tfoot", "tbody" | "tfoot" | "thead") => true,
        ("a", "a") => true,
        _ => false,
    }
}

/// Elements a close tag must not look past when searching for its opener.
fn is_barrier(open: &str, closing: &str) -> bool {
    match closing {
        "table" => false,
        "tr" | "td" | "th" | "tbody" | "thead" | "tfoot" => open == "table",
        _ => matches!(open, "table" | "td" | "th" | "button" | "html" | "body"),
    }
}

pub fn parse(src: &str) -> Dom {
    let mut dom = Dom { nodes: vec![Node { kind: Kind::Elem { tag: "#root".into(), attrs: vec![] }, parent: 0, children: vec![] }], truncated: false };
    let mut stack: Vec<NodeId> = vec![0];
    let b = src.as_bytes();
    let mut i = 0;
    let mut skip_depth: Option<String> = None;

    macro_rules! push_node {
        ($kind:expr) => {{
            let parent = *stack.last().unwrap();
            let id = dom.nodes.len();
            dom.nodes.push(Node { kind: $kind, parent, children: vec![] });
            dom.nodes[parent].children.push(id);
            id
        }};
    }

    while i < b.len() {
        if dom.nodes.len() >= MAX_NODES {
            dom.truncated = true;
            break;
        }
        if b[i] != b'<' {
            let end = src[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            if skip_depth.is_none() {
                let t = decode_entities(&src[i..end]);
                if !t.is_empty() {
                    push_node!(Kind::Text(t));
                }
            }
            i = end;
            continue;
        }
        // comment / doctype / processing
        if src[i..].starts_with("<!--") {
            i = src[i + 4..].find("-->").map(|e| i + 4 + e + 3).unwrap_or(b.len());
            continue;
        }
        if i + 1 < b.len() && (b[i + 1] == b'!' || b[i + 1] == b'?') {
            i = src[i..].find('>').map(|e| i + e + 1).unwrap_or(b.len());
            continue;
        }
        let closing = i + 1 < b.len() && b[i + 1] == b'/';
        let ns = i + 1 + closing as usize;
        let mut ne = ns;
        while ne < b.len() && (b[ne].is_ascii_alphanumeric() || b[ne] == b'-' || b[ne] == b':') {
            ne += 1;
        }
        if ne == ns {
            // a lone '<' is text
            if skip_depth.is_none() {
                push_node!(Kind::Text("<".into()));
            }
            i += 1;
            continue;
        }
        let tag = src[ns..ne].to_ascii_lowercase();
        // find the end of the tag, honouring quotes
        let mut j = ne;
        let mut q = 0u8;
        while j < b.len() {
            match (q, b[j]) {
                (0, b'"') | (0, b'\'') => q = b[j],
                (x, c) if x != 0 && c == x => q = 0,
                (0, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        let inner = &src[ne..j.min(b.len())];
        i = (j + 1).min(b.len());
        if let Some(s) = &skip_depth {
            if closing && *s == tag {
                skip_depth = None;
            }
            continue;
        }
        if closing {
            // pop to the nearest matching open element (not across barriers)
            let mut k = stack.len();
            let mut found = None;
            while k > 1 {
                k -= 1;
                let t = dom.tag(stack[k]).unwrap_or("");
                if t == tag {
                    found = Some(k);
                    break;
                }
                if t != tag && is_barrier(t, &tag) {
                    break;
                }
            }
            if let Some(k) = found {
                stack.truncate(k);
            }
            continue;
        }
        // raw text elements swallow until their close tag
        if RAW.contains(&tag.as_str()) && !inner.trim_end().ends_with('/') {
            let lower_rest: String = src[i..].chars().take(0).collect();
            let _ = lower_rest;
            let close = format!("</{tag}");
            let hay = &src[i..];
            let lc = hay.to_ascii_lowercase();
            let end = lc.find(&close).unwrap_or(hay.len());
            if tag == "title" || tag == "textarea" {
                let id = push_node!(Kind::Elem { tag: tag.clone(), attrs: parse_attrs(inner) });
                let t = decode_entities(&hay[..end]);
                dom.nodes.push(Node { kind: Kind::Text(t), parent: id, children: vec![] });
                let tid = dom.nodes.len() - 1;
                dom.nodes[id].children.push(tid);
            }
            let after = hay[end..].find('>').map(|e| end + e + 1).unwrap_or(hay.len());
            i += after;
            continue;
        }
        if SKIP.contains(&tag.as_str()) {
            if !VOID.contains(&tag.as_str()) && tag != "head" && tag != "map" {
                skip_depth = Some(tag);
            }
            continue;
        }
        // implied end tags
        while stack.len() > 1 {
            let top = dom.tag(*stack.last().unwrap()).unwrap_or("").to_string();
            if closes(&top, &tag) {
                stack.pop();
            } else {
                break;
            }
        }
        if stack.len() >= MAX_DEPTH {
            stack.truncate(MAX_DEPTH / 2);
        }
        let attrs = parse_attrs(inner);
        let id = push_node!(Kind::Elem { tag: tag.clone(), attrs });
        let self_closed = inner.trim_end().ends_with('/');
        if !VOID.contains(&tag.as_str()) && !self_closed {
            stack.push(id);
        }
    }
    dom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(d: &Dom, id: NodeId) -> String {
        match &d.nodes[id].kind {
            Kind::Text(t) => t.clone(),
            Kind::Elem { tag, .. } => {
                let kids: Vec<String> = d.nodes[id].children.iter().map(|&c| tags(d, c)).collect();
                if tag == "#root" { kids.join("") } else { format!("<{tag}>{}</{tag}>", kids.join("")) }
            }
        }
    }

    #[test]
    fn nesting_and_void() {
        let d = parse("<div><p>a<br>b<img src=x></p></div>");
        assert_eq!(tags(&d, 0), "<div><p>a<br></br>b<img></img></p></div>");
    }

    #[test]
    fn implied_ends() {
        let d = parse("<ul><li>a<li>b</ul><p>x<p>y");
        assert_eq!(tags(&d, 0), "<ul><li>a</li><li>b</li></ul><p>x</p><p>y</p>");
        let d = parse("<table><tr><td>1<td>2<tr><td>3</table>");
        assert_eq!(tags(&d, 0), "<table><tr><td>1</td><td>2</td></tr><tr><td>3</td></tr></table>");
    }

    #[test]
    fn table_close_tags_close_open_cells() {
        let d = parse("<table><tr><td>a<td>b</tr></table><h3>after</h3>");
        assert_eq!(tags(&d, 0), "<table><tr><td>a</td><td>b</td></tr></table><h3>after</h3>");
        let d = parse("<table><tr><td><b>x</table><p>y");
        assert_eq!(tags(&d, 0), "<table><tr><td><b>x</b></td></tr></table><p>y</p>");
    }

    #[test]
    fn malformed_does_not_panic() {
        for s in ["<<<>>>", "<a href=\"x", "</b></i>text", "<div", "&", "&#xZZ;&#99999999999;", "<p <b>>x", "<!-- unterminated", "<script>alert(1)", "", "<a><a><a>", "<table><td></tr></div>x"] {
            let _ = parse(s);
        }
        let d = parse("a < b and c > d");
        assert!(d.text_of(0).contains("a < b"));
    }

    #[test]
    fn mismatched_close_is_ignored() {
        let d = parse("<b>x</i>y</b>z");
        assert_eq!(tags(&d, 0), "<b>xy</b>z");
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &copy; &unknown; &amp"), "a & b <c> AB © &unknown; &");
        assert_eq!(decode_entities("fish&chips"), "fish&chips");
        assert_eq!(decode_entities("&#12354;"), "あ");
    }

    #[test]
    fn attributes() {
        let d = parse("<a HREF='x?a=1&amp;b=2' class=big disabled>t</a>");
        let a = d.find("a").unwrap();
        assert_eq!(d.attr(a, "href"), Some("x?a=1&b=2"));
        assert_eq!(d.attr(a, "class"), Some("big"));
        assert_eq!(d.attr(a, "disabled"), Some(""));
    }

    #[test]
    fn scripts_and_styles_are_dropped() {
        let d = parse("<p>a</p><script>if (a<b) {x='</p>'}</script><style>p{}</style><p>b</p>");
        assert_eq!(d.text_of(0), "a b");
    }

    #[test]
    fn title_and_comments() {
        let d = parse("<html><head><title>T &amp; U</title></head><!-- c --><body>x</body></html>");
        let t = d.find("title").unwrap();
        assert_eq!(d.text_of(t), "T & U");
        assert_eq!(d.text_of(d.find("body").unwrap()), "x");
    }

    #[test]
    fn huge_input_is_capped() {
        let s = "<p>x</p>".repeat(MAX_NODES);
        let d = parse(&s);
        assert!(d.truncated && d.nodes.len() <= MAX_NODES + 2);
    }
}
