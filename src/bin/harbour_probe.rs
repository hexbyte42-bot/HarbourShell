//! Prints what the real Linux backend sees (read-only; never changes settings).
//! Run on the device: `harbour_probe [--scan]`.

use harbour_shell::system::linux::LinuxBackend;
use harbour_shell::system::{Backend, ExtPort};

fn main() {
    if let Some(dir) = std::env::args().skip_while(|a| a != "--apps").nth(1) {
        // Show how every .desktop entry in `dir` is interpreted.
        let apps = harbour_shell::launcher::registry::scan(std::path::Path::new(&dir));
        for a in &apps {
            let ok = harbour_shell::launcher::desktop_entry::exec_argv(&a.exec).is_some();
            println!("{:<22} exec_ok={:<5} terminal={:<5} icon={} exec={}", a.name, ok, a.terminal, a.icon.is_some(), a.exec);
        }
        println!("{} apps", apps.len());
        return;
    }
    let b = LinuxBackend::new();
    let scan = std::env::args().any(|a| a == "--scan");
    println!("battery     : {:?}", b.battery());
    println!("brightness  : {:?}", b.brightness());
    println!("volume      : {:?}", b.volume());
    println!("wifi        : {:?}", b.wifi_status());
    println!("ethernet ip : {:?}", b.eth_ip());
    println!("bt powered  : {}", b.bt_powered());
    println!("grove 5v    : {:?}", b.ext_port(ExtPort::Grove5V));
    println!("ext 5v      : {:?}", b.ext_port(ExtPort::Ext5V));
    println!("clock       : {} / tz {} / ntp {:?}", b.clock_text(), b.timezone(), b.ntp_synced());
    println!("sysinfo     : {:?}", b.sysinfo());
    if scan {
        println!("wifi scan   : {:?}", b.wifi_scan());
        println!("updates     : {:?}", b.upgradable_count());
    }
}
