//! Builds the boot splash for the Cardputer Zero's `fb_load`: assets/images/splash.png (the
//! "Harbour" wordmark, black on transparent) in the theme's colours, with the version and
//! "for M5Stack Cardputer Zero" at the bottom. written as a 16-bit (RGB565) BMP, transposed (170 x 320): that is what `fb_load` accepts and rotates to the 320 x 170 LCD.
//!
//!   boot_splash [theme.yaml] [out.bmp]      (defaults: assets/themes/theme-black.yaml, dist/boot/splash.bmp)
use harbour_shell::font::{load_display_font, FontSystem};
use harbour_shell::theme::{Theme, ThemeManager};
use sdl2::image::LoadSurface;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::surface::Surface;

fn main() {
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    let sdl = sdl2::init().unwrap();
    let video = sdl.video().unwrap();
    let _img = sdl2::image::init(sdl2::image::InitFlag::PNG).unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let theme_path = args.first().cloned().unwrap_or_else(|| "assets/themes/theme-black.yaml".into());
    let out = args.get(1).cloned().unwrap_or_else(|| "dist/boot/splash.bmp".into());
    let tm = ThemeManager::new(Theme::load_from_yaml(&theme_path).expect("theme"));
    let (bg, fg, muted) = (tm.get_color("background"), tm.get_color("foreground"), tm.get_color("muted"));

    let window = video.window("splash", 320, 170).build().unwrap();
    let mut c = window.into_canvas().software().build().unwrap();
    c.set_draw_color(bg);
    c.clear();

    // The wordmark: every opaque pixel in the theme's ink.
    let mark = Surface::from_file("assets/images/splash.png").expect("splash.png").convert_format(PixelFormatEnum::RGBA32).unwrap();
    let (w, h, pitch) = (mark.width() as i32, mark.height() as i32, mark.pitch() as usize);
    c.set_draw_color(fg);
    mark.with_lock(|px| {
        for y in 0..h {
            for x in 0..w {
                if px[y as usize * pitch + x as usize * 4 + 3] > 127 {
                    let _ = c.fill_rect(Rect::new(x, y, 1, 1));
                }
            }
        }
    });

    // Version and device, centred at the bottom.
    let mut fs = FontSystem::new();
    fs.display = load_display_font();
    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    let device = "for M5Stack Cardputer Zero";
    for (text, y, col) in [(version.as_str(), 136, muted), (device, 150, fg)] {
        let tw = fs.measure_display_text(text);
        fs.draw_display_text(&mut c, (320 - tw) / 2, y, text, col);
    }

    let px = c.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // fb_load only accepts 16-bit BMPs ("expected 16-bit BMP, got bpp=24") and expects them in the panel's own
    // portrait orientation: it rotates the file to the 320x170 mode. The file is therefore the picture transposed
    // (170 wide, 320 tall); checked on the device with labelled candidates. RGB565 with BI_BITFIELDS masks, rows
    // bottom-up.
    let (w, h) = (170usize, 320usize);
    let mut data = Vec::with_capacity(w * h * 2);
    for y in (0..h).rev() {
        for x in 0..w {
            // pixel (x, y) of the file = pixel (y, x) of the landscape picture
            let i = (x * 320 + y) * 3;
            let v = (((px[i] as u16) >> 3) << 11) | (((px[i + 1] as u16) >> 2) << 5) | ((px[i + 2] as u16) >> 3);
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    let off = 14u32 + 40 + 12;
    let mut f = Vec::with_capacity(off as usize + data.len());
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(off + data.len() as u32).to_le_bytes());
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&off.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(w as i32).to_le_bytes());
    f.extend_from_slice(&(h as i32).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&[0x13, 0x0B, 0, 0, 0x13, 0x0B, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    f.extend_from_slice(&0xF800u32.to_le_bytes());
    f.extend_from_slice(&0x07E0u32.to_le_bytes());
    f.extend_from_slice(&0x001Fu32.to_le_bytes());
    f.extend_from_slice(&data);
    std::fs::write(&out, f).unwrap();
    let _ = Color::RGB(0, 0, 0);
    println!("wrote {out}");
}
