//! OSC 1.0 / 1.1 packet decoding (messages, nested bundles), a small encoder
//! (mock traffic and tests) and the per-address statistics the Interceptor's
//! OSC page shows. No I/O in here.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum OscArg {
    Int(i32),
    Float(f32),
    Str(String),
    Blob(Vec<u8>),
    Int64(i64),
    Double(f64),
    /// NTP timetag (`t`); 1 means "immediately".
    Time(u64),
    True,
    False,
    Nil,
    Inf,
    Char(char),
    Rgba(u32),
    Midi([u8; 4]),
    Symbol(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OscMsg {
    pub addr: String,
    pub args: Vec<OscArg>,
}

/// One message out of a packet; `time` is the timetag of the innermost bundle.
#[derive(Clone, Debug, PartialEq)]
pub struct OscItem {
    pub time: Option<u64>,
    pub msg: OscMsg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscError {
    Empty,
    Truncated,
    BadAddress,
    BadString,
    BadTypeTag(char),
    BadBundle,
    TooDeep,
}

const MAX_DEPTH: usize = 8;

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

fn read_str(buf: &[u8], pos: &mut usize) -> Result<String, OscError> {
    let rest = buf.get(*pos..).ok_or(OscError::Truncated)?;
    let nul = rest.iter().position(|b| *b == 0).ok_or(OscError::BadString)?;
    let end = *pos + pad4(nul + 1);
    if end > buf.len() {
        return Err(OscError::Truncated);
    }
    let s = String::from_utf8_lossy(&rest[..nul]).to_string();
    *pos = end;
    Ok(s)
}

fn take<'a>(buf: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], OscError> {
    let end = pos.checked_add(n).ok_or(OscError::Truncated)?;
    let s = buf.get(*pos..end).ok_or(OscError::Truncated)?;
    *pos = end;
    Ok(s)
}

pub fn parse_message(buf: &[u8]) -> Result<OscMsg, OscError> {
    let mut pos = 0;
    let addr = read_str(buf, &mut pos)?;
    if !addr.starts_with('/') {
        return Err(OscError::BadAddress);
    }
    if pos >= buf.len() {
        // OSC 1.0 allowed a missing type tag string.
        return Ok(OscMsg { addr, args: Vec::new() });
    }
    let tags = read_str(buf, &mut pos)?;
    let tags = tags.strip_prefix(',').ok_or(OscError::BadTypeTag(tags.chars().next().unwrap_or('?')))?;
    let mut args = Vec::new();
    for t in tags.chars() {
        let a = match t {
            'i' => OscArg::Int(i32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap())),
            'f' => OscArg::Float(f32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap())),
            's' => OscArg::Str(read_str(buf, &mut pos)?),
            'S' => OscArg::Symbol(read_str(buf, &mut pos)?),
            'b' => {
                let n = u32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap()) as usize;
                let data = take(buf, &mut pos, n)?.to_vec();
                let padded = pad4(n) - n;
                take(buf, &mut pos, padded)?;
                OscArg::Blob(data)
            }
            'h' => OscArg::Int64(i64::from_be_bytes(take(buf, &mut pos, 8)?.try_into().unwrap())),
            'd' => OscArg::Double(f64::from_be_bytes(take(buf, &mut pos, 8)?.try_into().unwrap())),
            't' => OscArg::Time(u64::from_be_bytes(take(buf, &mut pos, 8)?.try_into().unwrap())),
            'c' => OscArg::Char(char::from_u32(u32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap())).unwrap_or('?')),
            'r' => OscArg::Rgba(u32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap())),
            'm' => OscArg::Midi(take(buf, &mut pos, 4)?.try_into().unwrap()),
            'T' => OscArg::True,
            'F' => OscArg::False,
            'N' => OscArg::Nil,
            'I' => OscArg::Inf,
            '[' | ']' => continue, // arrays are flattened
            other => return Err(OscError::BadTypeTag(other)),
        };
        args.push(a);
    }
    Ok(OscMsg { addr, args })
}

fn parse_into(buf: &[u8], depth: usize, time: Option<u64>, out: &mut Vec<OscItem>) -> Result<(), OscError> {
    if depth > MAX_DEPTH {
        return Err(OscError::TooDeep);
    }
    if buf.is_empty() {
        return Err(OscError::Empty);
    }
    if buf.starts_with(b"#bundle\0") {
        if buf.len() < 16 {
            return Err(OscError::Truncated);
        }
        let tt = u64::from_be_bytes(buf[8..16].try_into().unwrap());
        let mut pos = 16;
        while pos < buf.len() {
            let n = i32::from_be_bytes(take(buf, &mut pos, 4)?.try_into().unwrap());
            if n < 0 {
                return Err(OscError::BadBundle);
            }
            let el = take(buf, &mut pos, n as usize)?;
            parse_into(el, depth + 1, Some(tt), out)?;
        }
        Ok(())
    } else {
        out.push(OscItem { time, msg: parse_message(buf)? });
        Ok(())
    }
}

/// Decode a datagram into its messages (bundles are flattened, depth-first).
pub fn parse_packet(buf: &[u8]) -> Result<Vec<OscItem>, OscError> {
    let mut v = Vec::new();
    parse_into(buf, 0, None, &mut v)?;
    Ok(v)
}

// ------------------------------------------------------------------ encoding

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

pub fn encode_msg(addr: &str, args: &[OscArg]) -> Vec<u8> {
    let mut out = Vec::new();
    put_str(&mut out, addr);
    let mut tags = String::from(",");
    for a in args {
        tags.push(match a {
            OscArg::Int(_) => 'i',
            OscArg::Float(_) => 'f',
            OscArg::Str(_) => 's',
            OscArg::Blob(_) => 'b',
            OscArg::Int64(_) => 'h',
            OscArg::Double(_) => 'd',
            OscArg::Time(_) => 't',
            OscArg::True => 'T',
            OscArg::False => 'F',
            OscArg::Nil => 'N',
            OscArg::Inf => 'I',
            OscArg::Char(_) => 'c',
            OscArg::Rgba(_) => 'r',
            OscArg::Midi(_) => 'm',
            OscArg::Symbol(_) => 'S',
        });
    }
    put_str(&mut out, &tags);
    for a in args {
        match a {
            OscArg::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Str(s) | OscArg::Symbol(s) => put_str(&mut out, s),
            OscArg::Blob(b) => {
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
                while out.len() % 4 != 0 {
                    out.push(0);
                }
            }
            OscArg::Int64(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Time(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Char(c) => out.extend_from_slice(&(*c as u32).to_be_bytes()),
            OscArg::Rgba(v) => out.extend_from_slice(&v.to_be_bytes()),
            OscArg::Midi(m) => out.extend_from_slice(m),
            OscArg::True | OscArg::False | OscArg::Nil | OscArg::Inf => {}
        }
    }
    out
}

pub fn encode_bundle(time: u64, elements: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"#bundle\0".to_vec();
    out.extend_from_slice(&time.to_be_bytes());
    for e in elements {
        out.extend_from_slice(&(e.len() as i32).to_be_bytes());
        out.extend_from_slice(e);
    }
    out
}

// ---------------------------------------------------------------- formatting

impl OscArg {
    /// Short text: floats with 3 decimals, blobs as `<blob n>`.
    pub fn compact(&self) -> String {
        match self {
            OscArg::Int(v) => v.to_string(),
            OscArg::Float(v) => format!("{v:.3}"),
            OscArg::Str(s) if s.is_empty() => "\"\"".into(),
            OscArg::Str(s) => s.clone(),
            OscArg::Symbol(s) => format!("'{s}"),
            OscArg::Blob(b) => format!("<blob {}>", b.len()),
            OscArg::Int64(v) => format!("{v}h"),
            OscArg::Double(v) => format!("{v:.3}"),
            OscArg::Time(1) => "@now".into(),
            OscArg::Time(t) => format!("@{}.{:03}", t >> 32, ((t & 0xFFFF_FFFF) * 1000) >> 32),
            OscArg::True => "T".into(),
            OscArg::False => "F".into(),
            OscArg::Nil => "nil".into(),
            OscArg::Inf => "inf".into(),
            OscArg::Char(c) => format!("'{c}'"),
            OscArg::Rgba(v) => format!("#{v:08X}"),
            OscArg::Midi(m) => format!("midi:{:02X}{:02X}{:02X}{:02X}", m[0], m[1], m[2], m[3]),
        }
    }
}

pub fn fmt_args(args: &[OscArg]) -> String {
    args.iter().map(|a| a.compact()).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- statistics

pub const SPARK: usize = 16;

/// Count, last arguments and a per-second history (newest last) for one address.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddrStat {
    pub count: u64,
    pub last: String,
    hist: [u16; SPARK],
    sec: u64,
}

impl AddrStat {
    fn shifted(&self, now_sec: u64) -> [u16; SPARK] {
        let d = now_sec.saturating_sub(self.sec) as usize;
        let mut h = [0u16; SPARK];
        if d < SPARK {
            h[..SPARK - d].copy_from_slice(&self.hist[d..]);
        }
        h
    }

    pub fn record(&mut self, now_ms: u64, args: &str) {
        let sec = now_ms / 1000;
        self.hist = self.shifted(sec);
        self.sec = self.sec.max(sec);
        self.hist[SPARK - 1] = self.hist[SPARK - 1].saturating_add(1);
        self.count += 1;
        if !args.is_empty() || self.count == 1 {
            self.last = args.to_string();
        }
    }

    /// Per-second history ending at the current second.
    pub fn history(&self, now_ms: u64) -> [u16; SPARK] {
        self.shifted(now_ms / 1000)
    }

    /// Messages per second, averaged over the last two complete seconds.
    pub fn rate(&self, now_ms: u64) -> f32 {
        let h = self.history(now_ms);
        (h[SPARK - 2] as f32 + h[SPARK - 3] as f32) / 2.0
    }
}

pub const MAX_ADDRS: usize = 256;

#[derive(Default)]
pub struct OscTable {
    pub map: BTreeMap<String, AddrStat>,
    pub total: AddrStat,
    pub dropped: u64,
}

impl OscTable {
    pub fn record(&mut self, addr: &str, args: &str, now_ms: u64) {
        self.total.record(now_ms, "");
        if let Some(s) = self.map.get_mut(addr) {
            s.record(now_ms, args);
        } else if self.map.len() < MAX_ADDRS {
            self.map.entry(addr.to_string()).or_default().record(now_ms, args);
        }
    }

    pub fn bad(&mut self) {
        self.dropped += 1;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: &str, args: Vec<OscArg>) -> Vec<u8> {
        encode_msg(addr, &args)
    }

    #[test]
    fn simple_message_and_padding() {
        // "/a" ",ii" 1 2 -> hand-written bytes
        let b = [b'/', b'a', 0, 0, b',', b'i', b'i', 0, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFE];
        let m = parse_message(&b).unwrap();
        assert_eq!(m, OscMsg { addr: "/a".into(), args: vec![OscArg::Int(1), OscArg::Int(-2)] });
        // address exactly 4 bytes long needs a full 4 bytes of padding
        let m = parse_message(&msg("/abc", vec![])).unwrap();
        assert_eq!(m.addr, "/abc");
        assert_eq!(msg("/abc", vec![]).len(), 8 + 4);
    }

    #[test]
    fn every_type_tag_round_trips() {
        let args = vec![
            OscArg::Int(-7),
            OscArg::Float(0.5),
            OscArg::Str("hello".into()),
            OscArg::Blob(vec![1, 2, 3, 4, 5]),
            OscArg::Int64(-1 << 40),
            OscArg::Double(2.25),
            OscArg::Time(1),
            OscArg::True,
            OscArg::False,
            OscArg::Nil,
            OscArg::Inf,
            OscArg::Char('x'),
            OscArg::Rgba(0x11223344),
            OscArg::Midi([0, 0x90, 60, 100]),
            OscArg::Symbol("sym".into()),
        ];
        let items = parse_packet(&msg("/all/types", args.clone())).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].msg.args, args);
        assert_eq!(items[0].time, None);
    }

    #[test]
    fn missing_type_tags_are_ok_and_arrays_flatten() {
        let b = [b'/', b'x', 0, 0];
        assert_eq!(parse_message(&b).unwrap().args.len(), 0);
        let mut v = Vec::new();
        put_str(&mut v, "/arr");
        put_str(&mut v, ",[ii]");
        v.extend_from_slice(&1i32.to_be_bytes());
        v.extend_from_slice(&2i32.to_be_bytes());
        assert_eq!(parse_message(&v).unwrap().args, vec![OscArg::Int(1), OscArg::Int(2)]);
    }

    #[test]
    fn bundles_nest_and_carry_timetags() {
        let inner = encode_bundle(5 << 32, &[msg("/in", vec![OscArg::Int(1)])]);
        let outer = encode_bundle(1, &[msg("/a", vec![OscArg::Float(1.0)]), inner, msg("/b", vec![])]);
        let items = parse_packet(&outer).unwrap();
        let got: Vec<(&str, Option<u64>)> = items.iter().map(|i| (i.msg.addr.as_str(), i.time)).collect();
        assert_eq!(got, vec![("/a", Some(1)), ("/in", Some(5 << 32)), ("/b", Some(1))]);
        // too deep
        let mut deep = msg("/x", vec![]);
        for _ in 0..12 {
            deep = encode_bundle(1, &[deep]);
        }
        assert_eq!(parse_packet(&deep), Err(OscError::TooDeep));
    }

    #[test]
    fn truncated_and_garbage_packets_are_errors() {
        assert_eq!(parse_packet(&[]), Err(OscError::Empty));
        let good = msg("/grid/key", vec![OscArg::Int(1), OscArg::Int(2), OscArg::Int(3)]);
        for cut in 1..good.len() {
            if cut % 4 == 0 && cut >= 12 && cut < good.len() {
                // a cut on a 4-byte boundary may leave a shorter but valid tag list; just must not panic
            }
            let _ = parse_packet(&good[..cut]);
        }
        assert_eq!(parse_packet(&good[..good.len() - 1]), Err(OscError::Truncated));
        assert!(parse_packet(b"hello world!").is_err()); // no leading '/'
        assert_eq!(parse_packet(b"/x\0\0,q\0\0").unwrap_err(), OscError::BadTypeTag('q'));
        assert_eq!(parse_packet(b"/x\0\0i\0\0\0").unwrap_err(), OscError::BadTypeTag('i'));
        assert_eq!(parse_packet(b"/nonul"), Err(OscError::BadString));
        // blob length larger than the packet
        let mut b = Vec::new();
        put_str(&mut b, "/b");
        put_str(&mut b, ",b");
        b.extend_from_slice(&1000u32.to_be_bytes());
        assert_eq!(parse_packet(&b), Err(OscError::Truncated));
        // bundle element size lies / negative
        let mut bad = b"#bundle\0".to_vec();
        bad.extend_from_slice(&1u64.to_be_bytes());
        bad.extend_from_slice(&100i32.to_be_bytes());
        bad.extend_from_slice(b"/a\0\0");
        assert_eq!(parse_packet(&bad), Err(OscError::Truncated));
        let mut neg = b"#bundle\0".to_vec();
        neg.extend_from_slice(&1u64.to_be_bytes());
        neg.extend_from_slice(&(-4i32).to_be_bytes());
        assert_eq!(parse_packet(&neg), Err(OscError::BadBundle));
        assert_eq!(parse_packet(b"#bundle\0\0\0"), Err(OscError::Truncated));
        // pseudo random garbage never panics
        let mut x = 12345u32;
        for _ in 0..500 {
            let n = (x % 40) as usize;
            let v: Vec<u8> = (0..n)
                .map(|_| {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    (x >> 24) as u8
                })
                .collect();
            let _ = parse_packet(&v);
        }
    }

    #[test]
    fn compact_formatting() {
        let a = vec![OscArg::Int(3), OscArg::Float(0.5), OscArg::Blob(vec![0; 9]), OscArg::Str("hi".into()), OscArg::True, OscArg::Nil, OscArg::Rgba(0xFF8000FF)];
        assert_eq!(fmt_args(&a), "3 0.500 <blob 9> hi T nil #FF8000FF");
        assert_eq!(OscArg::Time(1).compact(), "@now");
        assert_eq!(OscArg::Midi([0, 0x90, 60, 100]).compact(), "midi:0090 3C64".replace(' ', ""));
    }

    #[test]
    fn address_table_aggregation() {
        let mut t = OscTable::default();
        // 5 messages per second for 4 seconds on /enc, one /tick
        for s in 0..4u64 {
            for k in 0..5u64 {
                t.record("/enc", "2 -1", 10_000 + s * 1000 + k * 100);
            }
        }
        t.record("/tick", "0.250", 13_900);
        t.bad();
        assert_eq!(t.map.len(), 2);
        assert_eq!(t.map["/enc"].count, 20);
        assert_eq!(t.map["/enc"].last, "2 -1");
        assert_eq!(t.total.count, 21);
        assert_eq!(t.dropped, 1);
        // at 14.2s the two previous complete seconds (12, 13) had 5 each
        assert_eq!(t.map["/enc"].rate(14_200), 5.0);
        assert_eq!(t.map["/enc"].history(14_200)[SPARK - 1], 0);
        assert_eq!(t.map["/enc"].history(13_999)[SPARK - 1], 5);
        // long silence: history empties
        assert_eq!(t.map["/enc"].rate(60_000), 0.0);
        assert_eq!(t.map["/enc"].history(60_000), [0; SPARK]);
        t.clear();
        assert!(t.map.is_empty() && t.dropped == 0);
    }

    #[test]
    fn table_is_capped() {
        let mut t = OscTable::default();
        for i in 0..(MAX_ADDRS + 20) {
            t.record(&format!("/a{i}"), "", 0);
        }
        assert_eq!(t.map.len(), MAX_ADDRS);
        assert_eq!(t.total.count, (MAX_ADDRS + 20) as u64);
    }
}
