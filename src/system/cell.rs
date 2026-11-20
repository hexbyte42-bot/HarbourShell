//! Client for `harbour-celld`, the cellular daemon (QDC507 4G module):
//! NDJSON JSON-RPC over `/run/cell.sock`. One request per connection.
//!
//! Used by the Linux backend (status bar) and by the Cellular app (SMS).
//! Every call is blocking; call from a worker thread when the UI must not
//! stall (notably `sms.send`, which waits for the network).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{json, Value};

use super::CellStatus;

pub const SOCK: &str = "/run/cell.sock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmsMessage {
    pub id: i64,
    /// "in" | "out"
    pub direction: String,
    pub address: String,
    pub body: String,
    /// Unix timestamp (network SCTS for incoming, local time for outgoing).
    pub ts: i64,
    pub status: String,
}

/// One RPC round trip. `read_timeout` must cover the daemon's work:
/// `sms.send` waits for the modem, so it gets a long leash.
pub fn request(method: &str, params: &Value, read_timeout: Duration) -> Result<Value, String> {
    let mut stream = UnixStream::connect(SOCK).map_err(|e| format!("celld: {e}"))?;
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(|e| format!("celld: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("celld: {e}"))?;
    let req = json!({"id": 1, "method": method, "params": params});
    writeln!(stream, "{req}").map_err(|e| format!("celld: {e}"))?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| format!("celld: {e}"))?;
    let resp: Value = serde_json::from_str(line.trim()).map_err(|e| format!("celld: bad reply ({e})"))?;
    if let Some(err) = resp.get("error") {
        return Err(format!("celld: {}", err.as_str().unwrap_or("error")));
    }
    resp.get("result")
        .cloned()
        .ok_or_else(|| "celld: no result".to_string())
}

/// Signal bars 0..=4 from RSRP (dBm), falling back to the CSQ-derived dBm.
pub fn bars(rsrp: Option<i32>, csq_dbm: Option<i32>) -> Option<u8> {
    let (v, th) = match (rsrp, csq_dbm) {
        (Some(r), _) => (r, [-80, -90, -100, -110]),
        (None, Some(c)) => (c, [-70, -85, -100, -110]),
        (None, None) => return None,
    };
    Some(th.iter().filter(|&&t| v >= t).count() as u8)
}

/// Friendly name for the MCC+MNC operator codes the daemon reports.
pub fn operator_name(code: &str) -> String {
    match code {
        "46000" | "46002" | "46007" => "China Mobile".into(),
        "46001" | "46006" | "46009" => "China Unicom".into(),
        "46003" | "46005" | "46011" => "China Telecom".into(),
        "46015" => "China Broadnet".into(),
        other => other.to_string(),
    }
}

/// Merged daemon status + network info. Err when the daemon is unreachable.
pub fn status() -> Result<CellStatus, String> {
    let st = request("status", &json!({}), Duration::from_secs(10))?;
    let net = request("net.info", &json!({}), Duration::from_secs(10)).unwrap_or(Value::Null);
    let rsrp = st.get("rsrp").and_then(|v| v.as_i64()).map(|v| v as i32);
    let csq = st.get("csq").and_then(|v| v.as_i64()).map(|v| v as i32);
    let registered = st.get("registered").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(CellStatus {
        present: true,
        registered,
        operator: st.get("operator").and_then(|v| v.as_str()).map(operator_name),
        tech: st.get("tech").and_then(|v| v.as_str()).map(str::to_string),
        band: st.get("band").and_then(|v| v.as_str()).map(str::to_string),
        bars: if registered { bars(rsrp, csq) } else { Some(0) },
        rsrp,
        data_on: net.get("pdp_active").and_then(|v| v.as_bool()).unwrap_or(false),
        ip: net
            .get("ipv4")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(str::to_string),
        ipv6: net
            .get("ipv6")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// (rx, tx) byte counters of a network interface from sysfs; the Cellular
/// app samples these to show throughput and total usage.
pub fn iface_stats(iface: &str) -> Option<(u64, u64)> {
    let base = format!("/sys/class/net/{iface}/statistics");
    let read = |name: &str| {
        std::fs::read_to_string(format!("{base}/{name}"))
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

pub fn set_data(on: bool) -> Result<(), String> {
    request("net.set", &json!({"on": on}), Duration::from_secs(30))?;
    Ok(())
}

pub fn sms_list(limit: i64, offset: i64) -> Result<Vec<SmsMessage>, String> {
    let v = request("sms.list", &json!({"limit": limit, "offset": offset}), Duration::from_secs(10))?;
    let arr = v
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or("celld: no messages")?;
    Ok(arr
        .iter()
        .map(|m| SmsMessage {
            id: m.get("id").and_then(|x| x.as_i64()).unwrap_or(0),
            direction: m.get("direction").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            address: m.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            body: m.get("body").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            ts: m.get("ts").and_then(|x| x.as_i64()).unwrap_or(0),
            status: m.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })
        .collect())
}

pub fn sms_send(to: &str, text: &str) -> Result<i64, String> {
    let v = request("sms.send", &json!({"to": to, "text": text}), Duration::from_secs(110))?;
    Ok(v.get("id").and_then(|x| x.as_i64()).unwrap_or(0))
}

pub fn sms_delete(id: i64) -> Result<(), String> {
    request("sms.delete", &json!({"id": id}), Duration::from_secs(10))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_follow_rsrp_thresholds() {
        assert_eq!(bars(Some(-75), None), Some(4));
        assert_eq!(bars(Some(-85), None), Some(3));
        assert_eq!(bars(Some(-97), None), Some(2));
        assert_eq!(bars(Some(-108), None), Some(1));
        assert_eq!(bars(Some(-125), None), Some(0));
        assert_eq!(bars(None, Some(-65)), Some(4)); // CSQ fallback
        assert_eq!(bars(None, None), None);
    }

    #[test]
    fn operator_codes_map_to_names() {
        assert_eq!(operator_name("46001"), "China Unicom");
        assert_eq!(operator_name("46000"), "China Mobile");
        assert_eq!(operator_name("44010"), "44010");
    }
}
