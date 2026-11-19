//! What the Interceptor's Device tab shows: categories (device tree, USB, input, network, sound,
//! video, I2C, SPI, storage, power, thermal) as the top level of a [`widgets::TreeList`], each
//! read from sysfs/procfs only when it is opened. USB is the one category the app fills itself
//! (it tracks hot-plug); everything else comes from a [`DevSource`]. No SDL in here.

use std::path::{Path, PathBuf};

use crate::launcher::apps::devtree::{decode_prop, decode_string, DtSource, FsDt, MemDt};
use crate::launcher::widgets::TreeNode;

pub const CAT_USB: &str = "cat:usb";

const CATEGORIES: [(&str, &str); 11] = [
    ("dt", "Device tree"),
    ("usb", "USB"),
    ("input", "Input"),
    ("net", "Network"),
    ("snd", "Sound"),
    ("vid", "Video / Camera"),
    ("i2c", "I2C"),
    ("spi", "SPI"),
    ("blk", "Storage"),
    ("pwr", "Power"),
    ("therm", "Thermal"),
];

/// The tree's contents. Keys: `cat:<id>` for categories, `<id>:<name>` below them
/// (`dt:/soc/serial@7e201000`, `net:eth0`, ...).
pub trait DevSource {
    fn categories(&self) -> Vec<TreeNode> {
        CATEGORIES.iter().map(|(id, label)| TreeNode::new(format!("cat:{id}"), crate::launcher::i18n::tr_s(label)).expandable(true)).collect()
    }
    fn children(&self, key: &str) -> Vec<TreeNode>;
    /// Name / value pairs for the detail panel.
    fn details(&self, key: &str) -> Vec<(String, String)>;
}

/// One entry of a category.
#[derive(Clone, Debug, Default)]
pub struct Rec {
    pub id: String,
    pub label: String,
    pub extra: String,
    pub details: Vec<(String, String)>,
    pub children: Vec<Rec>,
}

fn rec(id: &str, label: &str, extra: &str, details: Vec<(&str, String)>) -> Rec {
    Rec { id: id.into(), label: label.into(), extra: extra.into(), details: details.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), children: Vec::new() }
}

fn node_of(cat: &str, r: &Rec) -> TreeNode {
    TreeNode::new(format!("{cat}:{}", r.id), r.label.clone()).extra(r.extra.clone()).expandable(!r.children.is_empty() || !r.details.is_empty())
}

/// Devices = the device tree plus a scanner for the flat categories.
pub struct Devices {
    dt: Option<Box<dyn DtSource>>,
    scan: Box<dyn Fn(&str) -> Vec<Rec>>,
}

impl Devices {
    /// The real machine.
    pub fn system() -> Self {
        Self::with_roots(PathBuf::from("/sys"), PathBuf::from("/proc"), PathBuf::from("/dev"), FsDt::open().map(|d| Box::new(d) as Box<dyn DtSource>))
    }

    pub fn with_roots(sys: PathBuf, proc_: PathBuf, dev: PathBuf, dt: Option<Box<dyn DtSource>>) -> Self {
        let roots = Roots { sys, proc_, dev };
        Self { dt, scan: Box::new(move |cat| roots.scan(cat)) }
    }

    /// Believable fake data (mock mode and screenshots).
    pub fn sample() -> Self {
        Self { dt: Some(Box::new(MemDt::sample())), scan: Box::new(sample_scan) }
    }

    fn find(&self, cat: &str, id: &str) -> Option<Rec> {
        fn walk(v: &[Rec], id: &str) -> Option<Rec> {
            v.iter().find_map(|r| if r.id == id { Some(r.clone()) } else { walk(&r.children, id) })
        }
        walk(&(self.scan)(cat), id)
    }

    fn dt_node(&self, parent: &str, name: &str) -> TreeNode {
        let path = format!("{parent}/{name}");
        let dt = self.dt.as_ref().unwrap();
        let disabled = dt.prop(&path, "status").map(|b| decode_string(&b)).map_or(false, |s| s == "disabled" || s.starts_with("fail") || s == "reserved");
        // Every node opens: its properties are rows below it, then its child nodes.
        TreeNode::new(format!("dt:{path}"), name).expandable(true).dim(disabled)
    }
}

impl DevSource for Devices {
    fn children(&self, key: &str) -> Vec<TreeNode> {
        if key == "cat:dt" {
            return match &self.dt {
                Some(dt) => dt.children("").iter().map(|n| self.dt_node("", n)).collect(),
                None => Vec::new(),
            };
        }
        if let Some(path) = key.strip_prefix("dt:") {
            return match &self.dt {
                Some(dt) => dt.children(path).iter().map(|n| self.dt_node(path, n)).collect(),
                None => Vec::new(),
            };
        }
        if let Some(cat) = key.strip_prefix("cat:") {
            return (self.scan)(cat).iter().map(|r| node_of(cat, r)).collect();
        }
        if let Some((cat, id)) = key.split_once(':') {
            return self.find(cat, id).map(|r| r.children.iter().map(|c| node_of(cat, c)).collect()).unwrap_or_default();
        }
        Vec::new()
    }

    fn details(&self, key: &str) -> Vec<(String, String)> {
        if key == "cat:dt" {
            return match &self.dt {
                Some(dt) => dt.props("").into_iter().map(|(k, v)| { let d = decode_prop(&k, &v); (k, d) }).collect(),
                None => vec![("device tree".into(), "not available".into())],
            };
        }
        if let Some(path) = key.strip_prefix("dt:") {
            return self.dt.as_ref().map(|dt| dt.props(path).into_iter().map(|(k, v)| { let d = decode_prop(&k, &v); (k, d) }).collect()).unwrap_or_default();
        }
        if let Some(cat) = key.strip_prefix("cat:") {
            let n = (self.scan)(cat).len();
            return vec![("items".into(), n.to_string())];
        }
        match key.split_once(':').and_then(|(cat, id)| self.find(cat, id)) {
            Some(r) => r.details,
            None => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------- sysfs

struct Roots {
    sys: PathBuf,
    proc_: PathBuf,
    dev: PathBuf,
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `(label, value)` for every file in `files` that exists and is not empty.
fn files(dir: &Path, files: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    files.iter().filter_map(|(label, f)| read(&dir.join(f)).map(|v| (*label, v))).collect()
}

fn driver_of(dir: &Path) -> Option<String> {
    std::fs::read_link(dir.join("device/driver")).or_else(|_| std::fs::read_link(dir.join("driver"))).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

pub fn human_size(bytes: u64) -> String {
    let f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if f < 1048576.0 {
        format!("{:.0} KB", f / 1024.0)
    } else if f < 1073741824.0 {
        format!("{:.0} MB", f / 1048576.0)
    } else if f < 1099511627776.0 {
        format!("{:.1} GB", f / 1073741824.0)
    } else {
        format!("{:.1} TB", f / 1099511627776.0)
    }
}

/// Thermal zones report millidegrees.
pub fn fmt_milli_c(m: i64) -> String {
    format!("{:.1} C", m as f64 / 1000.0)
}

/// `/proc/asound/cards` -> (number, id, short description, long name).
pub fn parse_asound_cards(text: &str) -> Vec<(String, String, String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        let Some((num, rest)) = t.split_once(char::is_whitespace) else { continue };
        if num.parse::<u32>().is_err() || !rest.trim_start().starts_with('[') {
            continue;
        }
        let rest = rest.trim_start();
        let Some(close) = rest.find(']') else { continue };
        let id = rest[1..close].trim().to_string();
        let desc = rest[close + 1..].trim_start_matches(':').trim().to_string();
        let long = lines.get(i + 1).map(|s| s.trim().to_string()).unwrap_or_default();
        out.push((num.to_string(), id, desc, long));
    }
    out
}

impl Roots {
    fn scan(&self, cat: &str) -> Vec<Rec> {
        match cat {
            "input" => self.input(),
            "net" => self.net(),
            "snd" => self.sound(),
            "vid" => self.video(),
            "i2c" => self.bus("bus/i2c/devices"),
            "spi" => self.bus("bus/spi/devices"),
            "blk" => self.block(),
            "pwr" => self.power(),
            "therm" => self.thermal(),
            _ => Vec::new(),
        }
    }

    fn input(&self) -> Vec<Rec> {
        let base = self.sys.join("class/input");
        names(&base)
            .into_iter()
            .filter(|n| n.starts_with("input"))
            .map(|n| {
                let d = base.join(&n);
                let name = read(&d.join("name")).unwrap_or_else(|| n.clone());
                let mut det = vec![("node", n.clone())];
                det.extend(files(&d, &[("name", "name"), ("phys", "phys"), ("uniq", "uniq"), ("bus", "id/bustype"), ("vendor", "id/vendor"), ("product", "id/product"), ("version", "id/version")]));
                let handlers: Vec<String> = names(&d).into_iter().filter(|x| x.starts_with("event") || x.starts_with("mouse") || x.starts_with("js")).collect();
                if !handlers.is_empty() {
                    det.push(("handlers", handlers.join(" ")));
                }
                rec(&n, &name, "", det)
            })
            .collect()
    }

    fn net(&self) -> Vec<Rec> {
        let base = self.sys.join("class/net");
        names(&base)
            .into_iter()
            .map(|n| {
                let d = base.join(&n);
                let state = read(&d.join("operstate")).unwrap_or_default();
                let mut det = files(&d, &[("state", "operstate"), ("address", "address"), ("mtu", "mtu"), ("carrier", "carrier")]);
                if let Some(s) = read(&d.join("speed")).and_then(|s| s.parse::<i32>().ok()).filter(|s| *s > 0) {
                    det.push(("speed", format!("{s} Mb/s")));
                }
                if let Some(drv) = driver_of(&d) {
                    det.push(("driver", drv));
                }
                rec(&n, &n, &state, det)
            })
            .collect()
    }

    fn sound(&self) -> Vec<Rec> {
        let text = std::fs::read_to_string(self.proc_.join("asound/cards")).unwrap_or_default();
        parse_asound_cards(&text).into_iter().map(|(num, id, desc, long)| rec(&num, &format!("{num}: {id}"), "", vec![("card", num.clone()), ("id", id.clone()), ("driver", desc), ("name", long)])).collect()
    }

    fn video(&self) -> Vec<Rec> {
        let base = self.sys.join("class/video4linux");
        let mut v: Vec<Rec> = names(&base)
            .into_iter()
            .filter(|n| n.starts_with("video"))
            .map(|n| {
                let d = base.join(&n);
                let name = read(&d.join("name")).unwrap_or_default();
                let node = self.dev.join(&n);
                let mut det = vec![("node", format!("/dev/{n}")), ("present", if node.exists() { "yes" } else { "no" }.to_string())];
                det.extend(files(&d, &[("name", "name"), ("index", "index")]));
                if let Some(drv) = driver_of(&d) {
                    det.push(("driver", drv));
                }
                rec(&n, &format!("{n}: {name}"), "", det)
            })
            .collect();
        if v.is_empty() {
            // No sysfs class (or a container): fall back to the device nodes.
            v = names(&self.dev).into_iter().filter(|n| n.starts_with("video")).map(|n| rec(&n, &n, "", vec![("node", format!("/dev/{n}"))])).collect();
        }
        v
    }

    fn bus(&self, rel: &str) -> Vec<Rec> {
        let base = self.sys.join(rel);
        names(&base)
            .into_iter()
            .map(|n| {
                let d = base.join(&n);
                let name = read(&d.join("name")).unwrap_or_default();
                let mut det = vec![("device", n.clone())];
                det.extend(files(&d, &[("name", "name"), ("modalias", "modalias")]));
                if let Some(drv) = driver_of(&d) {
                    det.push(("driver", drv));
                }
                rec(&n, &if name.is_empty() { n.clone() } else { format!("{n} {name}") }, "", det)
            })
            .collect()
    }

    fn block(&self) -> Vec<Rec> {
        let base = self.sys.join("block");
        let size_of = |d: &Path| read(&d.join("size")).and_then(|s| s.parse::<u64>().ok()).map(|s| human_size(s * 512));
        names(&base)
            .into_iter()
            .filter(|n| !n.starts_with("loop") && !n.starts_with("ram"))
            .map(|n| {
                let d = base.join(&n);
                let size = size_of(&d).unwrap_or_default();
                let mut det = vec![("device", n.clone()), ("size", size.clone())];
                det.extend(files(&d, &[("removable", "removable"), ("read-only", "ro"), ("model", "device/model"), ("vendor", "device/vendor")]));
                let mut r = rec(&n, &n, &size, det);
                r.children = names(&d)
                    .into_iter()
                    .filter(|p| p.starts_with(&n) && d.join(p).join("partition").exists())
                    .map(|p| {
                        let pd = d.join(&p);
                        let ps = size_of(&pd).unwrap_or_default();
                        rec(&p, &p, &ps, vec![("partition", read(&pd.join("partition")).unwrap_or_default()), ("size", ps.clone())])
                    })
                    .collect();
                r
            })
            .collect()
    }

    fn power(&self) -> Vec<Rec> {
        let base = self.sys.join("class/power_supply");
        names(&base)
            .into_iter()
            .map(|n| {
                let d = base.join(&n);
                let mut det = files(&d, &[("type", "type"), ("status", "status"), ("capacity", "capacity"), ("health", "health"), ("technology", "technology"), ("online", "online")]);
                if let Some(uv) = read(&d.join("voltage_now")).and_then(|s| s.parse::<f64>().ok()) {
                    det.push(("voltage", format!("{:.2} V", uv / 1e6)));
                }
                if let Some(ua) = read(&d.join("current_now")).and_then(|s| s.parse::<f64>().ok()) {
                    det.push(("current", format!("{:.0} mA", ua / 1000.0)));
                }
                let extra = read(&d.join("capacity")).map(|c| format!("{c}%")).unwrap_or_default();
                rec(&n, &n, &extra, det)
            })
            .collect()
    }

    fn thermal(&self) -> Vec<Rec> {
        let base = self.sys.join("class/thermal");
        names(&base)
            .into_iter()
            .filter(|n| n.starts_with("thermal_zone"))
            .map(|n| {
                let d = base.join(&n);
                let ty = read(&d.join("type")).unwrap_or_else(|| n.clone());
                let temp = read(&d.join("temp")).and_then(|s| s.parse::<i64>().ok()).map(fmt_milli_c).unwrap_or_default();
                let mut det = vec![("zone", n.clone()), ("type", ty.clone())];
                if !temp.is_empty() {
                    det.push(("temp", temp.clone()));
                }
                det.extend(files(&d, &[("policy", "policy")]));
                rec(&n, &ty, &temp, det)
            })
            .collect()
    }
}

// ----------------------------------------------------------------------- mock

fn sample_scan(cat: &str) -> Vec<Rec> {
    let s = |v: &str| v.to_string();
    match cat {
        "input" => vec![
            rec("input0", "cardputer-zero-keyboard", "", vec![("name", s("cardputer-zero-keyboard")), ("bus", s("0018")), ("handlers", s("kbd event0"))]),
            rec("input2", "gpio_ir_recv", "", vec![("name", s("gpio_ir_recv")), ("phys", s("gpio-rc/input0")), ("handlers", s("kbd event2"))]),
            rec("input4", "StickMouse", "", vec![("name", s("StickMouse")), ("bus", s("0005")), ("handlers", s("mouse0 event4"))]),
        ],
        "net" => vec![
            rec("eth0", "eth0", "up", vec![("state", s("up")), ("address", s("00:e0:4c:36:b1:2f")), ("speed", s("100 Mb/s")), ("mtu", s("1500")), ("driver", s("r8152"))]),
            rec("lo", "lo", "unknown", vec![("state", s("unknown")), ("mtu", s("65536"))]),
            rec("wlan0", "wlan0", "down", vec![("state", s("down")), ("address", s("d8:3a:dd:aa:10:42")), ("driver", s("brcmfmac"))]),
        ],
        "snd" => vec![
            rec("0", "0: vc4hdmi", "", vec![("card", s("0")), ("id", s("vc4hdmi")), ("driver", s("vc4-hdmi - vc4-hdmi"))]),
            rec("1", "1: ES8389Audio", "", vec![("card", s("1")), ("id", s("ES8389Audio")), ("driver", s("cardputerzero-a - ES8389-Audio"))]),
        ],
        "vid" => vec![rec("video0", "video0: UVC Camera", "", vec![("node", s("/dev/video0")), ("present", s("yes")), ("name", s("UVC Camera (046d:0825)")), ("driver", s("uvcvideo"))])],
        "i2c" => vec![
            rec("1-0034", "1-0034 bq27220", "", vec![("device", s("1-0034")), ("name", s("bq27220")), ("driver", s("bq27xxx-battery"))]),
            rec("1-0068", "1-0068 bmi270", "", vec![("device", s("1-0068")), ("name", s("bmi270")), ("driver", s("bmi270_i2c"))]),
        ],
        "spi" => vec![rec("spi0.0", "spi0.0 st7789", "", vec![("device", s("spi0.0")), ("name", s("st7789")), ("driver", s("fb_st7789v"))])],
        "blk" => {
            let mut d = rec("mmcblk0", "mmcblk0", "29.7 GB", vec![("device", s("mmcblk0")), ("size", s("29.7 GB")), ("removable", s("0"))]);
            d.children = vec![rec("mmcblk0p1", "mmcblk0p1", "512 MB", vec![("partition", s("1")), ("size", s("512 MB"))]), rec("mmcblk0p2", "mmcblk0p2", "29.2 GB", vec![("partition", s("2")), ("size", s("29.2 GB"))])];
            vec![d]
        }
        "pwr" => vec![rec("bq27220-0", "bq27220-0", "76%", vec![("type", s("Battery")), ("status", s("Discharging")), ("capacity", s("76")), ("voltage", s("3.98 V")), ("current", s("-182 mA"))])],
        "therm" => vec![rec("thermal_zone0", "cpu-thermal", "41.2 C", vec![("zone", s("thermal_zone0")), ("type", s("cpu-thermal")), ("temp", s("41.2 C"))])],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn fixture() -> (PathBuf, Devices) {
        let root = std::env::temp_dir().join(format!("dev-test-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        let _ = fs::remove_dir_all(&root);
        let (sys, proc_, dev) = (root.join("sys"), root.join("proc"), root.join("dev"));
        put(&sys, "class/input/input2/name", "gpio_ir_recv\n");
        put(&sys, "class/input/input2/id/vendor", "0001\n");
        fs::create_dir_all(sys.join("class/input/input2/event2")).unwrap();
        put(&sys, "class/net/eth0/operstate", "up\n");
        put(&sys, "class/net/eth0/speed", "100\n");
        put(&sys, "class/net/eth0/address", "00:e0:4c:36:b1:2f\n");
        put(&sys, "class/net/wlan0/operstate", "down\n");
        put(&sys, "class/net/wlan0/speed", "-1\n");
        put(&proc_, "asound/cards", " 0 [vc4hdmi        ]: vc4-hdmi - vc4-hdmi\n                      vc4-hdmi\n 1 [ES8389Audio    ]: cardputerzero-a - ES8389-Audio\n                      ES8389-Audio\n");
        put(&sys, "class/video4linux/video0/name", "UVC Camera\n");
        put(&dev, "video0", "");
        put(&sys, "bus/i2c/devices/1-0034/name", "bq27220\n");
        put(&sys, "bus/spi/devices/spi0.0/modalias", "spi:st7789\n");
        put(&sys, "block/loop0/size", "0\n");
        put(&sys, "block/mmcblk0/size", "62333952\n");
        put(&sys, "block/mmcblk0/mmcblk0p1/partition", "1\n");
        put(&sys, "block/mmcblk0/mmcblk0p1/size", "1048576\n");
        put(&sys, "class/power_supply/bat/capacity", "76\n");
        put(&sys, "class/power_supply/bat/voltage_now", "3980000\n");
        put(&sys, "class/thermal/thermal_zone0/type", "cpu-thermal\n");
        put(&sys, "class/thermal/thermal_zone0/temp", "41234\n");
        put(&root.join("dt"), "model", "Test\0");
        put(&root.join("dt"), "soc/serial@1/status", "disabled\0");
        let dt: Box<dyn DtSource> = Box::new(FsDt::at(root.join("dt")));
        (root.clone(), Devices::with_roots(sys, proc_, dev, Some(dt)))
    }

    fn labels(d: &Devices, key: &str) -> Vec<String> {
        d.children(key).into_iter().map(|n| n.label).collect()
    }

    #[test]
    fn categories_cover_everything_in_order() {
        let d = Devices::sample();
        let c: Vec<String> = d.categories().into_iter().map(|n| n.label).collect();
        assert_eq!(c, vec!["Device tree", "USB", "Input", "Network", "Sound", "Video / Camera", "I2C", "SPI", "Storage", "Power", "Thermal"]);
        assert!(d.categories().iter().all(|n| n.expandable && n.key.starts_with("cat:")));
    }

    #[test]
    fn flat_categories_are_read_from_sysfs() {
        let (root, d) = fixture();
        assert_eq!(labels(&d, "cat:input"), vec!["gpio_ir_recv"]);
        let det = d.details("input:input2");
        assert!(det.contains(&("vendor".into(), "0001".into())) && det.contains(&("handlers".into(), "event2".into())));
        assert_eq!(labels(&d, "cat:net"), vec!["eth0", "wlan0"]);
        let eth = d.children("cat:net");
        assert_eq!(eth[0].extra, "up");
        assert!(d.details("net:eth0").contains(&("speed".into(), "100 Mb/s".into())));
        assert!(!d.details("net:wlan0").iter().any(|(k, _)| k == "speed")); // -1 = unknown
        assert_eq!(labels(&d, "cat:snd"), vec!["0: vc4hdmi", "1: ES8389Audio"]);
        assert!(d.details("snd:1").contains(&("name".into(), "ES8389-Audio".into())));
        assert_eq!(labels(&d, "cat:vid"), vec!["video0: UVC Camera"]);
        assert!(d.details("vid:video0").contains(&("present".into(), "yes".into())));
        assert_eq!(labels(&d, "cat:i2c"), vec!["1-0034 bq27220"]);
        assert_eq!(labels(&d, "cat:spi"), vec!["spi0.0"]);
        // storage: loop devices are hidden, partitions are children
        let blk = d.children("cat:blk");
        assert_eq!(blk.len(), 1);
        assert_eq!(blk[0].extra, "29.7 GB");
        assert!(blk[0].expandable);
        assert_eq!(labels(&d, "blk:mmcblk0"), vec!["mmcblk0p1"]);
        assert_eq!(d.children("blk:mmcblk0")[0].extra, "512 MB");
        assert_eq!(d.children("cat:pwr")[0].extra, "76%");
        assert!(d.details("pwr:bat").contains(&("voltage".into(), "3.98 V".into())));
        assert_eq!(d.children("cat:therm")[0].extra, "41.2 C");
        assert_eq!(d.details("cat:net"), vec![("items".to_string(), "2".to_string())]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn device_tree_children_and_properties_are_lazy_and_decoded() {
        let (root, d) = fixture();
        assert_eq!(labels(&d, "cat:dt"), vec!["soc"]);
        let soc = &d.children("cat:dt")[0];
        assert!(soc.expandable);
        assert_eq!(soc.key, "dt:/soc");
        let serial = &d.children("dt:/soc")[0];
        assert_eq!(serial.key, "dt:/soc/serial@1");
        assert!(serial.dim && serial.expandable); // opens to its properties
        assert_eq!(d.details("dt:/soc/serial@1"), vec![("status".to_string(), "\"disabled\"".to_string())]);
        assert_eq!(d.details("cat:dt"), vec![("model".to_string(), "\"Test\"".to_string())]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_things_give_empty_categories() {
        let d = Devices::with_roots(PathBuf::from("/nonexistent/sys"), PathBuf::from("/nonexistent/proc"), PathBuf::from("/nonexistent/dev"), None);
        for (id, _) in CATEGORIES {
            assert!(d.children(&format!("cat:{id}")).is_empty());
        }
        assert_eq!(d.details("cat:dt")[0].1, "not available");
    }

    #[test]
    fn helpers_format_and_parse() {
        assert_eq!(human_size(62333952 * 512), "29.7 GB");
        assert_eq!(human_size(1048576 * 512), "512 MB");
        assert_eq!(fmt_milli_c(41234), "41.2 C");
        let c = parse_asound_cards(" 0 [vc4hdmi        ]: vc4-hdmi - vc4-hdmi\n                      vc4-hdmi\n");
        assert_eq!(c, vec![("0".to_string(), "vc4hdmi".to_string(), "vc4-hdmi - vc4-hdmi".to_string(), "vc4-hdmi".to_string())]);
        assert!(parse_asound_cards("--- no soundcards ---\n").is_empty());
    }

    #[test]
    fn the_sample_has_something_in_every_category() {
        let d = Devices::sample();
        for (id, _) in CATEGORIES {
            if id != "usb" {
                assert!(!d.children(&format!("cat:{id}")).is_empty(), "{id}");
            }
        }
        assert!(!d.details("net:eth0").is_empty());
    }
}
