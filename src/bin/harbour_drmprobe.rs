//! Read-only DRM report for the LCD, plus an optional timing benchmark.
//!   harbour_drmprobe [/dev/dri/cardN]          report only
//!   harbour_drmprobe bench [fbdev] [/dev/dri/cardN]    time full / partial pushes (draws test patterns; stop harbour first)
#[cfg(target_os = "linux")]
fn main() {
    use harbour_shell::platform::display::Display;
    use harbour_shell::platform::drm::*;
    use std::time::Instant;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.first().map(|a| a == "bench").unwrap_or(false);
    let path = args.iter().find(|a| a.starts_with("/dev/")).cloned().unwrap_or_else(|| find_panel().unwrap_or("/dev/dri/card1".into()));
    println!("device {path}");
    let dev = match Device::open(&path) {
        Ok(d) => d,
        Err(e) => return println!("open: {e}"),
    };
    println!("DUMB_BUFFER cap {:?}, TIMESTAMP_MONOTONIC {:?}", dev.cap(1), dev.cap(6));
    println!("set_master: {:?}", dev.set_master());
    match dev.connectors() {
        Ok(cs) => {
            for c in cs {
                println!("connector {} type {} ({}) connection {} crtc {}", c.connector_id, c.connector_type, c.connector_type_id, c.connection, c.crtc_id);
                for m in &c.modes {
                    println!("  mode {}x{} @{} clock {} flags {:#x} type {:#x}", m.hdisplay, m.vdisplay, m.vrefresh, m.clock, m.flags, m.typ);
                }
                if c.crtc_id != 0 {
                    if let Ok(cr) = dev.get_crtc(c.crtc_id) {
                        println!("  crtc fb {} mode_valid {} {}x{}", cr.fb_id, cr.mode_valid, cr.mode.hdisplay, cr.mode.vdisplay);
                    }
                }
            }
        }
        Err(e) => println!("connectors: {e}"),
    }
    let _ = dev.set_client_cap(2, 1); // universal planes
    match dev.planes() {
        Ok(ps) => {
            for (id, crtcs, fb, f) in ps {
                println!("plane {id} possible_crtcs {crtcs:#x} fb {fb} formats {:?}", f.iter().map(|&x| fourcc_name(x)).collect::<Vec<_>>());
            }
        }
        Err(e) => println!("planes: {e}"),
    }
    drop(dev);
    if !bench {
        return;
    }
    let mut d: Box<dyn Display> = if args.iter().any(|a| a == "fbdev") {
        match harbour_shell::platform::fb::Framebuffer::open("/dev/fb0") {
            Ok(d) => Box::new(d),
            Err(e) => return println!("fbdev open: {e}"),
        }
    } else {
        match DrmDisplay::open(Some(&path), "/dev/fb0") {
            Ok(d) => {
                println!("opened {}", d.describe());
                Box::new(d)
            }
            Err(e) => return println!("DrmDisplay::open: {e}"),
        }
    };
    let (w, h) = (320usize, 170usize);
    let mut frame = vec![0u8; w * h * 2];
    let fill = |f: &mut Vec<u8>, v: u16| f.chunks_exact_mut(2).for_each(|p| p.copy_from_slice(&v.to_le_bytes()));
    let t = |label: &str, n: usize, f: &mut dyn FnMut(usize)| {
        let s = Instant::now();
        for i in 0..n {
            f(i);
        }
        println!("{label}: {:.2} ms/frame over {n}", s.elapsed().as_secs_f64() * 1000.0 / n as f64);
    };
    fill(&mut frame, 0x001f);
    let _ = d.present(&frame); // acquire
    t("full (alternate colours)", 40, &mut |i| {
        fill(&mut frame, if i % 2 == 0 { 0xf800 } else { 0x07e0 });
        d.present(&frame).unwrap();
    });
    // partial: a 16x16 box moving; mouse-cursor sized damage
    t("partial 16x16 (cursor, 2 boxes damaged)", 100, &mut |i| {
        fill(&mut frame, 0x07e0);
        let x = (i * 3) % (w - 16);
        for y in 40..56 {
            for xx in x..x + 16 {
                frame[(y * w + xx) * 2] = 0xff;
                frame[(y * w + xx) * 2 + 1] = 0xff;
            }
        }
        d.present(&frame).unwrap();
    });
    t("partial full-width 20 rows", 50, &mut |i| {
        fill(&mut frame, 0x07e0);
        for y in 0..20 {
            for xx in 0..w {
                frame[(y * w + xx) * 2] = (i & 0xff) as u8;
            }
        }
        d.present(&frame).unwrap();
    });
    fill(&mut frame, 0);
    let _ = d.present(&frame);
}
#[cfg(not(target_os = "linux"))]
fn main() {}
