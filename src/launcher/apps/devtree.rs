//! Device tree browsing for the Interceptor's DT page: a lazily expanded tree over
//! `/proc/device-tree` (nodes are directories, properties are files), property
//! decoding, and a small in-memory tree for mock mode and tests. No SDL in here.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

/// Where the tree comes from. Paths are `""` for the root and `"/soc/serial@7e201000"` below it.
pub trait DtSource {
    /// Child node names, sorted.
    fn children(&self, path: &str) -> Vec<String>;
    /// Whether the node has any child (cheaper than listing them where possible).
    fn has_children(&self, path: &str) -> bool {
        !self.children(path).is_empty()
    }
    /// Raw property bytes (long ones may be cut).
    fn prop(&self, path: &str, name: &str) -> Option<Vec<u8>>;
    /// All properties, sorted by name.
    fn props(&self, path: &str) -> Vec<(String, Vec<u8>)>;
}

const PROP_CAP: u64 = 4096;

pub struct FsDt {
    root: PathBuf,
}

impl FsDt {
    /// `/proc/device-tree`, else `/sys/firmware/devicetree/base`.
    pub fn open() -> Option<Self> {
        ["/proc/device-tree", "/sys/firmware/devicetree/base"].iter().map(PathBuf::from).find(|p| p.is_dir()).map(|root| Self { root })
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, path: &str) -> PathBuf {
        let rel = path.trim_start_matches('/');
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }
}

fn read_capped(p: &std::path::Path) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    std::fs::File::open(p).ok()?.take(PROP_CAP).read_to_end(&mut v).ok()?;
    Some(v)
}

impl DtSource for FsDt {
    fn children(&self, path: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.dir(path))
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    fn has_children(&self, path: &str) -> bool {
        std::fs::read_dir(self.dir(path)).map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))).unwrap_or(false)
    }

    fn prop(&self, path: &str, name: &str) -> Option<Vec<u8>> {
        read_capped(&self.dir(path).join(name))
    }

    fn props(&self, path: &str) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(self.dir(path))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .filter_map(|e| Some((e.file_name().to_string_lossy().to_string(), read_capped(&e.path())?)))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

// ----------------------------------------------------------------- in-memory

#[derive(Default)]
struct MemNode {
    props: Vec<(String, Vec<u8>)>,
    children: Vec<String>,
}

/// A tree held in memory: mock mode and tests.
#[derive(Default)]
pub struct MemDt {
    nodes: BTreeMap<String, MemNode>,
}

/// NUL-terminated string list as DT stores it.
pub fn dt_strings(items: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for s in items {
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    }
    v
}

pub fn dt_cells(cells: &[u32]) -> Vec<u8> {
    cells.iter().flat_map(|c| c.to_be_bytes()).collect()
}

impl MemDt {
    /// Add (or extend) a node and register it with its parent.
    pub fn add(&mut self, path: &str, props: Vec<(&str, Vec<u8>)>) {
        if !path.is_empty() {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            if !parent.is_empty() && !self.nodes.contains_key(parent) {
                self.add(parent, vec![]); // intermediate nodes exist too
            }
            let p = self.nodes.entry(parent.to_string()).or_default();
            if !p.children.iter().any(|c| c == name) {
                p.children.push(name.to_string());
                p.children.sort();
            }
        }
        let n = self.nodes.entry(path.to_string()).or_default();
        for (k, v) in props {
            n.props.retain(|(a, _)| a != k);
            n.props.push((k.to_string(), v));
        }
        n.props.sort_by(|a, b| a.0.cmp(&b.0));
    }

    /// A small Raspberry-Pi-flavoured sample.
    pub fn sample() -> Self {
        let mut t = MemDt::default();
        t.add("", vec![("compatible", dt_strings(&["raspberrypi,model-zero-2-w", "brcm,bcm2837"])), ("model", dt_strings(&["Raspberry Pi Compute Module 0"])), ("#address-cells", dt_cells(&[1])), ("#size-cells", dt_cells(&[1])), ("interrupt-parent", dt_cells(&[1]))]);
        t.add("/aliases", vec![("serial0", dt_strings(&["/soc/serial@7e201000"])), ("ethernet0", dt_strings(&["/soc/usb@7e980000/usb1@1/ethernet@1"]))]);
        t.add("/chosen", vec![("bootargs", dt_strings(&["coherent_pool=1M snd_bcm2835.enable_headphones=0"])), ("stdout-path", dt_strings(&["serial0:115200n8"]))]);
        t.add("/memory@0", vec![("device_type", dt_strings(&["memory"])), ("reg", dt_cells(&[0, 0x1fc00000]))]);
        t.add("/cpus", vec![("#address-cells", dt_cells(&[1])), ("#size-cells", dt_cells(&[0]))]);
        for i in 0..4u32 {
            t.add(&format!("/cpus/cpu@{i}"), vec![("compatible", dt_strings(&["arm,cortex-a53"])), ("device_type", dt_strings(&["cpu"])), ("reg", dt_cells(&[i])), ("enable-method", dt_strings(&["brcm,bcm2836-smp"])), ("clock-frequency", dt_cells(&[1_000_000_000]))]);
        }
        t.add("/soc", vec![("compatible", dt_strings(&["simple-bus"])), ("ranges", dt_cells(&[0x7e000000, 0x3f000000, 0x1000000, 0x40000000, 0x40000000, 0x1000]))]);
        t.add("/soc/gpio@7e200000", vec![("compatible", dt_strings(&["brcm,bcm2835-gpio"])), ("reg", dt_cells(&[0x7e200000, 0xb4])), ("gpio-controller", vec![]), ("#gpio-cells", dt_cells(&[2])), ("status", dt_strings(&["okay"]))]);
        t.add("/soc/gpio@7e200000/gpio-ir", vec![("brcm,pins", dt_cells(&[13])), ("brcm,function", dt_cells(&[0]))]);
        t.add("/soc/serial@7e201000", vec![("compatible", dt_strings(&["arm,pl011", "arm,primecell"])), ("reg", dt_cells(&[0x7e201000, 0x200])), ("interrupts", dt_cells(&[0, 121, 4])), ("status", dt_strings(&["okay"]))]);
        t.add("/soc/spi@7e204000", vec![("compatible", dt_strings(&["brcm,bcm2835-spi"])), ("reg", dt_cells(&[0x7e204000, 0x200])), ("status", dt_strings(&["disabled"]))]);
        t.add("/soc/i2c@7e804000", vec![("compatible", dt_strings(&["brcm,bcm2835-i2c"])), ("reg", dt_cells(&[0x7e804000, 0x1000])), ("clock-frequency", dt_cells(&[100000])), ("status", dt_strings(&["okay"]))]);
        t.add("/soc/mmc@7e300000", vec![("compatible", dt_strings(&["brcm,bcm2835-sdhci"])), ("reg", dt_cells(&[0x7e300000, 0x100])), ("bus-width", dt_cells(&[4])), ("status", dt_strings(&["okay"]))]);
        t.add("/soc/usb@7e980000", vec![("compatible", dt_strings(&["brcm,bcm2835-usb"])), ("reg", dt_cells(&[0x7e980000, 0x10000])), ("dr_mode", dt_strings(&["host"])), ("status", dt_strings(&["okay"]))]);
        t.add("/ir-receiver@d", vec![("compatible", dt_strings(&["gpio-ir-receiver"])), ("gpios", dt_cells(&[1, 13, 1])), ("linux,rc-map-name", dt_strings(&["rc-rc6-mce"])), ("status", dt_strings(&["okay"]))]);
        t.add("/bluetooth", vec![("compatible", dt_strings(&["brcm,bcm43438-bt"])), ("local-bd-address", vec![0xf0, 0x40, 0xaf, 0x90, 0x3f, 0x83]), ("max-speed", dt_cells(&[3000000]))]);
        t.add("/thermal-zones/cpu-thermal", vec![("polling-delay", dt_cells(&[1000])), ("thermal-sensors", dt_cells(&[7]))]);
        t
    }
}

impl DtSource for MemDt {
    fn children(&self, path: &str) -> Vec<String> {
        self.nodes.get(path).map(|n| n.children.clone()).unwrap_or_default()
    }
    fn prop(&self, path: &str, name: &str) -> Option<Vec<u8>> {
        self.nodes.get(path)?.props.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
    fn props(&self, path: &str) -> Vec<(String, Vec<u8>)> {
        self.nodes.get(path).map(|n| n.props.clone()).unwrap_or_default()
    }
}

// ------------------------------------------------------------------- decoding

fn printable(b: u8) -> bool {
    (0x20..0x7f).contains(&b)
}

/// First string of a property (up to the first NUL).
pub fn decode_string(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// A NUL-terminated list of non-empty printable strings.
fn string_list(b: &[u8]) -> Option<Vec<String>> {
    if b.len() < 2 || *b.last()? != 0 || b[0] == 0 {
        return None;
    }
    let body = &b[..b.len() - 1];
    let mut out = Vec::new();
    for part in body.split(|c| *c == 0) {
        if part.is_empty() || !part.iter().all(|c| printable(*c)) {
            return None;
        }
        out.push(String::from_utf8_lossy(part).to_string());
    }
    Some(out)
}

fn cell(v: u32) -> String {
    if v < 10 { v.to_string() } else { format!("0x{v:x}") }
}

const STRING_PROPS: [&str; 14] = ["compatible", "status", "model", "name", "device_type", "label", "serial-number", "bootargs", "stdout-path", "dr_mode", "enable-method", "linux,rc-map-name", "firmware-name", "regulator-name"];

/// Human text for a property: strings quoted, u32 cells as `<..>`, anything else as hex bytes.
pub fn decode_prop(name: &str, b: &[u8]) -> String {
    if b.is_empty() {
        return "<empty>".into();
    }
    if STRING_PROPS.contains(&name) || !name.starts_with('#') {
        if let Some(l) = string_list(b) {
            return l.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
        }
    }
    if b.len() % 4 == 0 {
        let cells: Vec<String> = b.chunks_exact(4).map(|c| cell(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))).collect();
        let shown = cells.iter().take(8).cloned().collect::<Vec<_>>().join(" ");
        return if cells.len() > 8 { format!("<{shown} ..({})>", cells.len()) } else { format!("<{shown}>") };
    }
    let shown = b.iter().take(12).map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
    if b.len() > 12 { format!("[{shown} ..({})]", b.len()) } else { format!("[{shown}]") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_are_decoded() {
        assert_eq!(decode_prop("compatible", &dt_strings(&["a,b", "c"])), "\"a,b\", \"c\"");
        assert_eq!(decode_prop("status", b"okay\0"), "\"okay\"");
        assert_eq!(decode_prop("reg", &dt_cells(&[0x7e201000, 0x200])), "<0x7e201000 0x200>");
        assert_eq!(decode_prop("#size-cells", &dt_cells(&[1])), "<1>");
        assert_eq!(decode_prop("gpio-controller", &[]), "<empty>");
        assert_eq!(decode_prop("local-bd-address", &[0xf0, 0x40, 0xaf, 0x90, 0x3f, 0x83]), "[f0 40 af 90 3f 83]");
        // 16 cells: cut after 8, with the count
        assert_eq!(decode_prop("ranges", &dt_cells(&[1; 16])), "<1 1 1 1 1 1 1 1 ..(16)>");
        // 20 raw bytes that are not cells or strings
        let long: Vec<u8> = (0..21u8).map(|i| i + 0x80).collect();
        assert!(decode_prop("blob", &long).ends_with("..(21)]"));
        // a name-less string and the first string of a status value
        assert_eq!(decode_prop("whatever", b"hello\0"), "\"hello\"");
        assert_eq!(decode_string(b"disabled\0junk"), "disabled");
    }

    #[test]
    fn the_sample_tree_has_sorted_properties_and_children() {
        let t = MemDt::sample();
        assert_eq!(t.children(""), vec!["aliases", "bluetooth", "chosen", "cpus", "ir-receiver@d", "memory@0", "soc", "thermal-zones"]);
        assert!(t.has_children("/soc") && !t.has_children("/chosen"));
        let p = t.props("");
        assert_eq!(p.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["#address-cells", "#size-cells", "compatible", "interrupt-parent", "model"]);
        assert_eq!(decode_prop("compatible", &p[2].1), "\"raspberrypi,model-zero-2-w\", \"brcm,bcm2837\"");
    }

    #[test]
    fn a_real_directory_is_read_lazily() {
        let dir = std::env::temp_dir().join(format!("dt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("soc/serial@1")).unwrap();
        std::fs::write(dir.join("model"), b"Test\0").unwrap();
        let fs = FsDt::at(dir.clone());
        assert_eq!(fs.children(""), vec!["soc"]);
        assert!(fs.has_children("") && fs.has_children("/soc") && !fs.has_children("/soc/serial@1"));
        assert_eq!(fs.props(""), vec![("model".to_string(), b"Test\0".to_vec())]);
        assert_eq!(fs.prop("", "model"), Some(b"Test\0".to_vec()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
