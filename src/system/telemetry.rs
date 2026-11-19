//! Raw numbers for the System Monitor: one cheap snapshot (a handful of small sysfs / procfs
//! reads) per call, plus the parsers for them. The monitor keeps the previous snapshot and
//! turns counters into rates itself, so nothing here blocks or keeps state.

/// Cumulative CPU time in jiffies (`/proc/stat`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

/// `/proc/stat`: the aggregate `cpu` line first, then `cpu0`, `cpu1`, ...
pub fn parse_stat(text: &str) -> Vec<CpuTimes> {
    let mut all = None;
    let mut cores: Vec<(usize, CpuTimes)> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(name) = it.next() else { continue };
        let Some(rest) = name.strip_prefix("cpu") else { continue };
        let f: Vec<u64> = it.take(8).filter_map(|v| v.parse().ok()).collect();
        if f.len() < 4 {
            continue;
        }
        let total: u64 = f.iter().sum();
        let idle = f[3] + f.get(4).copied().unwrap_or(0);
        let t = CpuTimes { busy: total.saturating_sub(idle), total };
        if rest.is_empty() {
            all = Some(t);
        } else if let Ok(n) = rest.parse::<usize>() {
            cores.push((n, t));
        }
    }
    cores.sort_by_key(|c| c.0);
    let mut out = Vec::with_capacity(cores.len() + 1);
    out.push(all.unwrap_or_else(|| {
        let (b, t) = cores.iter().fold((0, 0), |a, c| (a.0 + c.1.busy, a.1 + c.1.total));
        CpuTimes { busy: b, total: t }
    }));
    out.extend(cores.into_iter().map(|c| c.1));
    out
}

/// Busy fraction (0..=1) between two snapshots; `None` when no time has passed.
pub fn cpu_usage(prev: CpuTimes, cur: CpuTimes) -> Option<f32> {
    let total = cur.total.checked_sub(prev.total)?;
    if total == 0 {
        return None;
    }
    let busy = cur.busy.saturating_sub(prev.busy);
    Some((busy as f32 / total as f32).clamp(0.0, 1.0))
}

/// `/proc/loadavg`: the 1, 5 and 15 minute averages.
pub fn parse_loadavg(text: &str) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (o, v) in out.iter_mut().zip(text.split_whitespace()) {
        *o = v.parse().unwrap_or(0.0);
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemInfo {
    pub total_kb: u64,
    pub free_kb: u64,
    pub avail_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl MemInfo {
    /// Memory in use by programs (what `free` calls used): total minus available.
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.avail_kb)
    }
    /// Page cache and buffers.
    pub fn cache_kb(&self) -> u64 {
        (self.buffers_kb + self.cached_kb).min(self.total_kb.saturating_sub(self.used_kb()))
    }
    /// Truly unused memory.
    pub fn free_only_kb(&self) -> u64 {
        self.free_kb.min(self.total_kb)
    }
    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
}

pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    let mut reclaimable = 0;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v: u64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        match k {
            "MemTotal" => m.total_kb = v,
            "MemFree" => m.free_kb = v,
            "MemAvailable" => m.avail_kb = v,
            "Buffers" => m.buffers_kb = v,
            "Cached" => m.cached_kb = v,
            "SReclaimable" => reclaimable = v,
            "SwapTotal" => m.swap_total_kb = v,
            "SwapFree" => m.swap_free_kb = v,
            _ => {}
        }
    }
    m.cached_kb += reclaimable;
    if m.avail_kb == 0 {
        m.avail_kb = m.free_kb + m.buffers_kb + m.cached_kb;
    }
    m
}

/// `get_throttled` bits (Raspberry Pi firmware): the low nibble is "now", bits 16.. "since boot".
pub fn throttle_text(bits: u32) -> String {
    const NOW: [(u32, &str); 4] = [(0, "Under-voltage"), (1, "Freq capped"), (2, "Throttled"), (3, "Temp limit")];
    let now: Vec<&str> = NOW.iter().filter(|(b, _)| bits & (1 << b) != 0).map(|(_, n)| *n).collect();
    if !now.is_empty() {
        return now.join(", ");
    }
    if bits >> 16 & 0xf != 0 {
        "OK (was limited)".into()
    } else {
        "OK".into()
    }
}

/// The content of a `get_throttled` file: `0x50005` or a plain number.
pub fn parse_throttled(text: &str) -> Option<u32> {
    let t = text.trim();
    match t.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => t.parse().ok(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerInfo {
    pub percent: Option<u8>,
    pub voltage_mv: Option<u32>,
    /// Positive while charging (bq27220).
    pub current_ma: Option<i32>,
    pub status: String,
    pub temp_c: Option<f32>,
    pub time_to_empty_min: Option<u32>,
    pub time_to_full_min: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetCounters {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub up: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    /// Aggregate first, then one entry per core.
    pub cpu: Vec<CpuTimes>,
    pub freq_mhz: Option<u32>,
    pub freq_max_mhz: Option<u32>,
    pub load: [f32; 3],
    pub mem: MemInfo,
    pub temp_c: Option<f32>,
    pub throttled: Option<u32>,
    pub power: PowerInfo,
    pub net: Vec<NetCounters>,
    pub disk_total_mb: u64,
    pub disk_free_mb: u64,
    pub uptime_s: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  12448 0 7070 175306 5846 0 89 0 0 0\ncpu0 2692 0 1565 44204 1654 0 24 0 0 0\ncpu1 2964 0 1968 44011 1212 0 25 0 0 0\nintr 1 2 3\n";

    #[test]
    fn stat_has_aggregate_then_cores() {
        let v = parse_stat(STAT);
        assert_eq!(v.len(), 3);
        // busy = total - idle - iowait
        assert_eq!(v[0].total, 12448 + 7070 + 175306 + 5846 + 89);
        assert_eq!(v[0].busy, 12448 + 7070 + 89);
        assert_eq!(v[1].busy, 2692 + 1565 + 24);
    }

    #[test]
    fn stat_without_aggregate_sums_cores() {
        let v = parse_stat("cpu0 10 0 10 80 0 0 0 0\ncpu1 20 0 0 80 0 0 0 0\n");
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], CpuTimes { busy: 40, total: 200 });
    }

    #[test]
    fn usage_is_a_delta_not_a_lifetime_average() {
        let a = CpuTimes { busy: 1000, total: 10_000 };
        let b = CpuTimes { busy: 1050, total: 10_100 };
        assert!((cpu_usage(a, b).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(cpu_usage(a, a), None, "no time passed");
        assert_eq!(cpu_usage(b, a), None, "counter went backwards");
        let c = CpuTimes { busy: 900, total: 10_100 }; // busy regressed: clamp, not underflow
        assert_eq!(cpu_usage(a, c), Some(0.0));
    }

    #[test]
    fn loadavg_and_meminfo() {
        assert_eq!(parse_loadavg("0.15 0.39 0.30 1/279 4445"), [0.15, 0.39, 0.30]);
        assert_eq!(parse_loadavg(""), [0.0; 3]);
        let m = parse_meminfo("MemTotal: 360000 kB\nMemFree: 40000 kB\nMemAvailable: 143000 kB\nBuffers: 10000 kB\nCached: 80000 kB\nSReclaimable: 5000 kB\nSwapTotal: 102400 kB\nSwapFree: 100000 kB\n");
        assert_eq!(m.used_kb(), 217_000);
        assert_eq!(m.cache_kb(), 95_000);
        assert_eq!(m.free_only_kb(), 40_000);
        assert_eq!(m.swap_used_kb(), 2_400);
        // old kernels without MemAvailable
        let o = parse_meminfo("MemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 5 kB\nCached: 20 kB\n");
        assert_eq!(o.avail_kb, 35);
    }

    #[test]
    fn throttle_flags() {
        assert_eq!(parse_throttled("0x50005\n"), Some(0x50005));
        assert_eq!(parse_throttled("5"), Some(5));
        assert_eq!(parse_throttled("zzz"), None);
        assert_eq!(throttle_text(0), "OK");
        assert_eq!(throttle_text(0x50000), "OK (was limited)");
        assert_eq!(throttle_text(0x5), "Under-voltage, Throttled");
    }
}
