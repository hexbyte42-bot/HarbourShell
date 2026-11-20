//! Harbour Cellular: the Cellular app (QDC507 4G data + SMS) as a standalone
//! program, for running under the official APPLaunch instead of inside
//! Harbour Shell. It hosts exactly one [`LauncherApp`] — the same
//! `cellular::make()` the shell embeds — and talks to the same backend
//! service (harbour-celld, over /run/cell.sock), so the phone state, the SMS
//! store and the settings are shared with the shell's built-in page.
//!
//! The host contract mirrors the shell's (see `launcher::app`): a short Esc
//! goes to the app (back), holding Esc for 0.9 s leaves, Alt tapped alone
//! (or Ctrl+Tab) opens the app menu, and the shell's menu strip is drawn
//! across the top.
//!
//! On the device it renders like the shell does: SDL software rendering
//! into a dummy-driver window, pixels copied to /dev/fb0, keyboard and
//! mouse read from evdev. On a desktop it opens an ordinary window.
//!
//! Environment (a subset of the shell's):
//! - `HARBOUR_FB=/dev/fb0`   render to the LCD framebuffer (default on the
//!   device when /dev/fb0 exists; `HARBOUR_NO_FB=1` disables the default)
//! - `HARBOUR_HEADLESS=1`    no window (screenshots / scripted runs)
//! - `HARBOUR_MOCK=1`        use the in-memory backend even on Linux
//! - `HARBOUR_NO_EVDEV=1`    do not open the evdev keyboard / mouse
//! - `HARBOUR_SCALE=N`       window scale on a desktop (default 3)
//! - `HARBOUR_ROOT=dir`      asset root (default: /usr/share/harbour-cellular
//!   when present, else the current directory)
//! - `HARBOUR_SCRIPT=...`    comma-separated steps for scripted runs, e.g.
//!   `wait:300,shot:phone,Return,text:10010,Escape,quit`
//! - `HARBOUR_SHOT_DIR=dir`  where `shot:` writes BMP files
//! - `HARBOUR_THEME=path`    theme YAML instead of the saved one

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::Canvas;
use sdl2::surface::Surface;
use sdl2::video::Window;

use harbour_shell::assets::AssetStore;
use harbour_shell::chrome::{self, StatusView, SCREEN_H, SCREEN_W};
use harbour_shell::font::{load_mplus_f12r, FontSystem};
use harbour_shell::launcher::app::{AppAction, Ctx};
use harbour_shell::launcher::apps;
use harbour_shell::launcher::packages;
use harbour_shell::launcher::widgets::{self, AppMenu, Pointer};
use harbour_shell::platform::display::Display;
use harbour_shell::platform::keys::{HoldKey, Input, MediaKey};
use harbour_shell::platform::Shared;
use harbour_shell::system::config::{self, Config};
use harbour_shell::system::linux::LinuxBackend;
use harbour_shell::system::mock::MockBackend;
use harbour_shell::system::Backend;
use harbour_shell::theme::{Theme, ThemeManager};

type Fb = Box<dyn Display>;

struct Toast {
    text: String,
    until: Instant,
}

static TERMINATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    TERMINATE.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn env_bool(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

// ---- status thread (same readings as the shell's menu strip) ---------------

fn reachable() -> bool {
    use std::net::{SocketAddr, TcpStream};
    ["1.1.1.1:53", "8.8.8.8:53"].iter().any(|a| a.parse::<SocketAddr>().ok().map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(1500)).is_ok()).unwrap_or(false))
}

fn spawn_status(sys: Arc<dyn Backend>, status: Arc<Mutex<StatusView>>) {
    let mock = std::env::var("HARBOUR_MOCK").is_ok();
    std::thread::spawn(move || loop {
        let b = sys.battery();
        let w = sys.wifi_status();
        let cell = sys.cell_status();
        let eth = sys.eth_ip().is_some();
        let online = mock || reachable();
        let vol = sys.volume();
        if let Ok(mut s) = status.lock() {
            s.battery = b.percent;
            s.charging = b.charging;
            s.wifi = w.signal;
            s.wifi_radio = w.radio;
            s.cell = if cell.present { Some(cell.bars.unwrap_or(0)) } else { None };
            s.ethernet = eth;
            s.online = online;
            s.volume = vol;
        }
        std::thread::sleep(Duration::from_secs(2));
    });
}

// ---- framebuffer + screenshots (same helpers as the shell) -----------------

fn push_frame(canvas: &Canvas<Window>, fb: &mut Fb) -> Result<(), String> {
    let px = canvas.read_pixels(None, PixelFormatEnum::RGB565)?;
    fb.present(&px).map_err(|e| e.to_string())?;
    Ok(())
}

fn save_shot(canvas: &Canvas<Window>, path: &str) -> Result<(), String> {
    let (w, h) = canvas.output_size()?;
    let mut px = canvas.read_pixels(None, PixelFormatEnum::ARGB8888)?;
    let surf = Surface::from_data(&mut px, w, h, w * 4, PixelFormatEnum::ARGB8888)?;
    surf.save_bmp(path)
}

// ---- scripted runs -----------------------------------------------------------

enum Step {
    Wait(u64),
    Shot(String),
    Inputs(Vec<Input>),
    Quit,
}

/// A tiny subset of the shell's script engine: enough to drive the app
/// headlessly and take screenshots. Bare words are key names (`Return`,
/// `Tab`, `Escape`, `Up`, ...); a single printable character is a key press
/// plus its text, like a real key on the device keyboard.
struct Script {
    steps: VecDeque<Step>,
    wait_until: Option<Instant>,
    pending_shot: Option<String>,
    quit: bool,
}

fn keycode_for_char(ch: char) -> Option<Keycode> {
    Some(match ch.to_ascii_lowercase() {
        'a' => Keycode::A,
        'b' => Keycode::B,
        'c' => Keycode::C,
        'd' => Keycode::D,
        'e' => Keycode::E,
        'f' => Keycode::F,
        'g' => Keycode::G,
        'h' => Keycode::H,
        'i' => Keycode::I,
        'j' => Keycode::J,
        'k' => Keycode::K,
        'l' => Keycode::L,
        'm' => Keycode::M,
        'n' => Keycode::N,
        'o' => Keycode::O,
        'p' => Keycode::P,
        'q' => Keycode::Q,
        'r' => Keycode::R,
        's' => Keycode::S,
        't' => Keycode::T,
        'u' => Keycode::U,
        'v' => Keycode::V,
        'w' => Keycode::W,
        'x' => Keycode::X,
        'y' => Keycode::Y,
        'z' => Keycode::Z,
        '0' => Keycode::Num0,
        '1' => Keycode::Num1,
        '2' => Keycode::Num2,
        '3' => Keycode::Num3,
        '4' => Keycode::Num4,
        '5' => Keycode::Num5,
        '6' => Keycode::Num6,
        '7' => Keycode::Num7,
        '8' => Keycode::Num8,
        '9' => Keycode::Num9,
        ' ' => Keycode::Space,
        _ => return None,
    })
}

fn keycode_for_name(name: &str) -> Option<Keycode> {
    Some(match name {
        "Return" => Keycode::Return,
        "Tab" => Keycode::Tab,
        "Escape" => Keycode::Escape,
        "Backspace" => Keycode::Backspace,
        "Delete" => Keycode::Delete,
        "Up" => Keycode::Up,
        "Down" => Keycode::Down,
        "Left" => Keycode::Left,
        "Right" => Keycode::Right,
        "Space" => Keycode::Space,
        _ => return None,
    })
}

impl Script {
    fn from_env() -> Option<Self> {
        let text = std::env::var("HARBOUR_SCRIPT").ok()?;
        let mut steps = VecDeque::new();
        for tok in text.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if let Some(v) = tok.strip_prefix("wait:") {
                steps.push_back(Step::Wait(v.parse().unwrap_or(0)));
            } else if let Some(v) = tok.strip_prefix("shot:") {
                steps.push_back(Step::Shot(v.to_string()));
            } else if let Some(v) = tok.strip_prefix("text:") {
                steps.push_back(Step::Inputs(vec![Input::Text(v.to_string())]));
            } else if tok == "quit" {
                steps.push_back(Step::Quit);
            } else if let Some(k) = keycode_for_name(tok) {
                steps.push_back(Step::Inputs(vec![
                    Input::Key { key: k, mods: Mod::NOMOD, repeat: false },
                    Input::KeyUp { key: k },
                ]));
            } else if tok.chars().count() == 1 {
                let ch = tok.chars().next().unwrap();
                let mut v = Vec::new();
                if let Some(k) = keycode_for_char(ch) {
                    v.push(Input::Key { key: k, mods: Mod::NOMOD, repeat: false });
                }
                v.push(Input::Text(ch.to_string()));
                if let Some(k) = keycode_for_char(ch) {
                    v.push(Input::KeyUp { key: k });
                }
                steps.push_back(Step::Inputs(v));
            } else {
                eprintln!("script: unknown step {tok:?} ignored");
            }
        }
        Some(Self { steps, wait_until: None, pending_shot: None, quit: false })
    }

    /// Advance the script; returns inputs to inject this frame.
    fn step(&mut self, now: Instant) -> Vec<Input> {
        if let Some(until) = self.wait_until {
            if now < until {
                return Vec::new();
            }
            self.wait_until = None;
        }
        let mut out = Vec::new();
        while let Some(step) = self.steps.pop_front() {
            match step {
                Step::Wait(ms) => {
                    self.wait_until = Some(now + Duration::from_millis(ms));
                    break;
                }
                Step::Shot(name) => self.pending_shot = Some(name),
                Step::Inputs(v) => out.extend(v),
                Step::Quit => {
                    self.quit = true;
                    break;
                }
            }
        }
        out
    }

    fn finished(&self) -> bool {
        self.quit && self.pending_shot.is_none()
    }
}

// ---- the host -----------------------------------------------------------------

fn main() -> Result<(), String> {
    // systemd / APPLaunch stop the process with SIGTERM: leave cleanly.
    unsafe {
        libc::signal(libc::SIGTERM, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("harbour-cellular   (the Cellular app on its own; see the header of src/bin/harbour-cellular.rs)");
        return Ok(());
    }
    // Asset root: fonts, themes, images and the app package live under it,
    // addressed by relative paths exactly like in the shell.
    if let Ok(root) = std::env::var("HARBOUR_ROOT") {
        std::env::set_current_dir(&root).map_err(|e| format!("HARBOUR_ROOT {root}: {e}"))?;
    } else if Path::new("/usr/share/harbour-cellular").is_dir() {
        std::env::set_current_dir("/usr/share/harbour-cellular").map_err(|e| e.to_string())?;
    }
    let fb_path = std::env::var("HARBOUR_FB").ok().or_else(|| {
        if !env_bool("HARBOUR_HEADLESS") && !env_bool("HARBOUR_NO_FB") && cfg!(target_os = "linux") && Path::new("/dev/fb0").exists() {
            Some("/dev/fb0".to_string())
        } else {
            None
        }
    });
    let headless = env_bool("HARBOUR_HEADLESS") || fb_path.is_some();
    if headless {
        std::env::set_var("SDL_VIDEODRIVER", "dummy");
    }

    let _ = sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "0");
    sdl2::image::init(sdl2::image::InitFlag::PNG)?;
    let sdl = sdl2::init()?;
    sdl.mouse().show_cursor(false);
    let video = sdl.video()?;
    video.text_input().start();
    let scale: u32 = if headless {
        1
    } else {
        std::env::var("HARBOUR_SCALE").ok().and_then(|s| s.parse().ok()).filter(|s| (1..=6).contains(s)).unwrap_or(3)
    };
    let window = video
        .window("Cellular", SCREEN_W * scale, SCREEN_H * scale)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut builder = window.into_canvas();
    if headless {
        builder = builder.software();
    }
    let mut canvas = builder.build().map_err(|e| e.to_string())?;
    if scale > 1 {
        canvas.set_scale(scale as f32, scale as f32)?;
    }
    let texture_creator = canvas.texture_creator();

    // Fonts, theme, assets — the same set the shell loads (integration
    // branch form: Fusion Pixel wide-char font via load_cjk_font; on
    // feat/cellular this block uses load_jis_font + Shinonome instead —
    // see packaging/cellular-app/README.md).
    let mut fs = FontSystem::new();
    fs.load_font(load_mplus_f12r());
    if fs.load_cjk_font("assets/fonts/fusion-pixel-12px-zh_hk.bdf").is_ok() {
        fs.jis_height = 12;
        fs.jis_dy = 1;
        match harbour_shell::bdf::BdfFont::load("assets/fonts/shnm6x12r.bdf") {
            Ok(f) => {
                fs.body_latin = Some(f);
                fs.latin_dy = 1;
            }
            Err(e) => eprintln!("[font] shnm6x12r.bdf: {e}"),
        }
    } else {
        eprintln!("[font] CJK BDF load failed");
    }
    fs.display = harbour_shell::font::load_display_font();
    fs.display_cavs = true;
    let mut assets = AssetStore::new(&texture_creator)?;
    {
        let pkgs = packages::scan(Path::new(packages::APPS_ROOT));
        packages::load_assets(&mut assets, &texture_creator, &pkgs);
        packages::install(pkgs);
    }

    // System backend and shared config (theme / language / status bar style
    // follow the same settings the shell uses).
    let sys: Arc<dyn Backend> = if cfg!(target_os = "linux") && !env_bool("HARBOUR_MOCK") {
        Arc::new(LinuxBackend::new())
    } else {
        Arc::new(MockBackend::new())
    };
    let cfg = Arc::new(Mutex::new(Config::load(config::default_path())));
    harbour_shell::launcher::i18n::set(harbour_shell::launcher::i18n::Lang::from_index(cfg.lock().map(|c| c.get_int("harbour_lang", 0)).unwrap_or(0)));
    let theme_path = std::env::var("HARBOUR_THEME").unwrap_or_else(|_| {
        let (i, inv) = cfg.lock().map(|c| (c.get_int("harbour_theme", 0), c.get_int("harbour_theme_invert", 0) == 1)).unwrap_or((0, false));
        harbour_shell::launcher::settings::theme_path(i, inv)
    });
    let tm = ThemeManager::new(Theme::load_from_yaml(&theme_path)?);

    // Input: evdev keyboard + mouse on the device, SDL events on a desktop.
    let shared = Arc::new(Shared::default());
    let (tx, rx) = channel::<Input>();
    #[cfg(target_os = "linux")]
    if !env_bool("HARBOUR_NO_EVDEV") {
        match harbour_shell::platform::evdev::find_keyboard() {
            Some(path) => match harbour_shell::platform::evdev::spawn(path.clone(), tx.clone(), shared.clone()) {
                Ok(_) => eprintln!("[input] evdev keyboard: {path}"),
                Err(e) => eprintln!("[input] cannot open {path}: {e}"),
            },
            None => eprintln!("[input] no evdev keyboard found"),
        }
        if let Err(e) = harbour_shell::platform::mouse::spawn(tx.clone(), shared.clone()) {
            eprintln!("[mouse] reader not started: {e}");
        }
    }
    let _ = &tx;

    let mut fb: Option<Fb> = match &fb_path {
        Some(p) => Some(harbour_shell::platform::display::open(p).map_err(|e| format!("framebuffer {p}: {e}"))?),
        None => None,
    };
    if let Some(f) = fb.as_ref() {
        f.unblank();
    }
    let mut fb_first = true;

    let status = Arc::new(Mutex::new(StatusView::default()));
    spawn_status(sys.clone(), status.clone());

    let mut app = apps::cellular::make();

    let mut events = sdl.event_pump()?;
    let mut esc = HoldKey::default();
    let mut app_menu: Option<AppMenu> = None;
    let mut toast: Option<Toast> = None;
    let mut pointer = Pointer::new(SCREEN_W as i32 / 2, SCREEN_H as i32 / 2);
    let mut cursor_until: Option<Instant> = None;
    let mut script = Script::from_env();
    let mut want_quit = false;

    'main: loop {
        let frame_start = Instant::now();
        if TERMINATE.load(std::sync::atomic::Ordering::Relaxed) {
            break 'main;
        }
        // ---- gather input ---------------------------------------------------
        let mut inputs: Vec<Input> = Vec::new();
        for e in events.poll_iter() {
            match e {
                Event::Quit { .. } => inputs.push(Input::Quit),
                Event::KeyDown { keycode: Some(key), keymod, repeat, .. } => {
                    inputs.push(Input::Key { key, mods: keymod, repeat });
                }
                Event::KeyUp { keycode: Some(key), .. } => inputs.push(Input::KeyUp { key }),
                Event::TextInput { text, .. } => inputs.push(Input::Text(text)),
                Event::MouseMotion { xrel, yrel, .. } => {
                    inputs.push(Input::MouseMove { dx: xrel / scale as i32, dy: yrel / scale as i32 });
                }
                Event::MouseButtonDown { mouse_btn, .. } | Event::MouseButtonUp { mouse_btn, .. } => {
                    let down = matches!(e, Event::MouseButtonDown { .. });
                    let button = match mouse_btn {
                        sdl2::mouse::MouseButton::Left => 0,
                        sdl2::mouse::MouseButton::Right => 1,
                        _ => 2,
                    };
                    inputs.push(Input::MouseButton { button, down });
                }
                Event::MouseWheel { y, .. } => inputs.push(Input::MouseWheel { dy: y }),
                _ => {}
            }
        }
        inputs.extend(rx.try_iter());
        if let Some(s) = script.as_mut() {
            inputs.extend(s.step(frame_start));
            if s.quit {
                want_quit = true;
            }
        }

        // ---- dispatch ---------------------------------------------------------
        for input in inputs {
            let ctx = Ctx { fs: &fs, tm: &tm, sys: sys.as_ref(), now: chrono::Local::now() };
            match input {
                Input::Quit => want_quit = true,
                Input::Key { key, repeat: false, .. } if key == Keycode::Escape => esc.press(frame_start),
                Input::KeyUp { key } if key == Keycode::Escape => {
                    esc.release();
                }
                _ => {}
            }
            if want_quit {
                break;
            }
            // The app menu takes the keyboard while it is open.
            if let Some(menu) = app_menu.as_mut() {
                match input {
                    Input::Key { key, repeat: false, .. } => match key {
                        Keycode::Up => menu.up(),
                        Keycode::Down => menu.down(),
                        Keycode::Left => menu.sideways(-1),
                        Keycode::Right => menu.sideways(1),
                        Keycode::Escape => app_menu = None,
                        Keycode::Return => {
                            let id = menu.items[menu.cursor].id.clone();
                            app_menu = None;
                            if id == widgets::QUIT_ID {
                                want_quit = true;
                            } else {
                                let action = app.menu_action(&id, &ctx);
                                handle_action(action, &mut toast, &mut want_quit);
                            }
                        }
                        _ => {}
                    },
                    Input::MouseButton { button: 0, down: true } => {
                        match menu.hit(&fs, pointer.x, pointer.y) {
                            Some(i) => {
                                let id = menu.items[i].id.clone();
                                app_menu = None;
                                if id == widgets::QUIT_ID {
                                    want_quit = true;
                                } else {
                                    let action = app.menu_action(&id, &ctx);
                                    handle_action(action, &mut toast, &mut want_quit);
                                }
                            }
                            None => app_menu = None,
                        }
                    }
                    Input::MouseMove { dx, dy } => {
                        pointer.moved(dx, dy);
                        cursor_until = Some(frame_start + Duration::from_secs(3));
                    }
                    _ => {}
                }
                continue;
            }
            match input {
                Input::Quit => want_quit = true,
                Input::Menu => app_menu = Some(AppMenu::new(app.menu())),
                Input::Key { key, mods, .. } if key == Keycode::Tab && mods.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    app_menu = Some(AppMenu::new(app.menu()));
                }
                Input::Key { key, mods, .. } if key == Keycode::Q && mods.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) && fb.is_none() => {
                    break 'main;
                }
                Input::Key { key, mods, repeat } => {
                    let action = app.key(key, mods, repeat, &ctx);
                    // The app's own Esc-at-top-level "Quit" is ignored: hold Esc or use the menu.
                    let action = if key == Keycode::Escape && action == AppAction::Quit { AppAction::None } else { action };
                    handle_action(action, &mut toast, &mut want_quit);
                }
                Input::Text(text) => {
                    let action = app.text(&text, &ctx);
                    handle_action(action, &mut toast, &mut want_quit);
                }
                Input::Media(k) => {
                    if let Some(t) = media(&sys, k) {
                        toast = Some(Toast { text: t, until: frame_start + Duration::from_millis(1200) });
                    }
                }
                Input::MouseMove { dx, dy } => {
                    let ev = pointer.moved(dx, dy);
                    cursor_until = Some(frame_start + Duration::from_secs(3));
                    let action = app.pointer(ev, &ctx);
                    handle_action(action, &mut toast, &mut want_quit);
                }
                Input::MouseButton { button, down } => {
                    let ev = pointer.button(button, down, frame_start);
                    if button == 1 && down {
                        // Right click: the app's context menu, else the app menu.
                        match app.context_menu(pointer.x, pointer.y, &ctx) {
                            Some(items) => app_menu = Some(AppMenu::context(items, pointer.x, pointer.y)),
                            None => app_menu = Some(AppMenu::new(app.menu())),
                        }
                    } else {
                        let action = app.pointer(ev, &ctx);
                        handle_action(action, &mut toast, &mut want_quit);
                    }
                }
                Input::MouseWheel { dy } => {
                    let ev = widgets::PointerEvent::Wheel { x: pointer.x, y: pointer.y, dy };
                    let action = app.pointer(ev, &ctx);
                    handle_action(action, &mut toast, &mut want_quit);
                }
                Input::KeyUp { .. } | Input::Voice(_) => {}
            }
            if want_quit {
                break;
            }
        }
        // Holding Esc leaves the app (the gauge shows while it charges).
        if esc.poll(frame_start, HoldKey::ESC_LONG) {
            want_quit = true;
        }
        if want_quit {
            if app.request_close() {
                break 'main;
            }
            want_quit = false;
        }

        // ---- tick + draw -------------------------------------------------------
        let ctx = Ctx { fs: &fs, tm: &tm, sys: sys.as_ref(), now: chrono::Local::now() };
        app.tick(&ctx);
        let mut st = status.lock().map(|s| s.clone()).unwrap_or_default();
        st.time = chrono::Local::now().format("%H:%M").to_string();
        st.bar_style = cfg.lock().map(|c| c.get_int("harbour_statusbar", 0)).unwrap_or(0).clamp(0, 2) as u8;
        chrome::draw_menu_bar(&mut canvas, &mut assets, &fs, &tm, &app.title(), &st);
        app.draw(&mut canvas, &mut assets, &ctx);
        if let Some(m) = &app_menu {
            m.draw(&mut canvas, &fs, &tm);
        }
        if let Some(t) = esc.gauge(frame_start) {
            chrome::draw_hold_gauge(&mut canvas, &fs, &tm, "Hold Esc: Quit", t / HoldKey::ESC_LONG.as_secs_f32());
        }
        if let Some(t) = &toast {
            if frame_start < t.until {
                chrome::draw_toast(&mut canvas, &fs, &tm, &t.text, None);
            }
        }
        chrome::draw_corners(&mut canvas, &tm);
        if cursor_until.map(|u| frame_start < u).unwrap_or(false) {
            chrome::draw_cursor(&mut canvas, &tm, pointer.x, pointer.y);
        }
        canvas.present();
        if let Some(f) = fb.as_mut() {
            if fb_first {
                f.invalidate();
                fb_first = false;
            }
            if let Err(e) = push_frame(&canvas, f) {
                eprintln!("[fb] {e}");
            }
        }
        if let Some(s) = script.as_mut() {
            if let Some(name) = s.pending_shot.take() {
                let dir = std::env::var("HARBOUR_SHOT_DIR").unwrap_or_else(|_| ".".into());
                let path = format!("{dir}/{name}.bmp");
                match save_shot(&canvas, &path) {
                    Ok(()) => eprintln!("shot {path}"),
                    Err(e) => eprintln!("shot failed: {e}"),
                }
            }
            if s.finished() {
                break 'main;
            }
        }
        // ~30 fps is plenty for this UI; queued inputs are not lost meanwhile.
        let elapsed = frame_start.elapsed();
        if elapsed < Duration::from_millis(33) {
            std::thread::sleep(Duration::from_millis(33) - elapsed);
        }
    }
    shared.quit.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Apply an app action the way the shell would (minus the parts a single-app
/// host does not have: there is no other app to launch or open files with).
fn handle_action(action: AppAction, toast: &mut Option<Toast>, want_quit: &mut bool) {
    match action {
        AppAction::None => {}
        AppAction::Quit => *want_quit = true,
        AppAction::Toast(text) => {
            *toast = Some(Toast { text, until: Instant::now() + Duration::from_millis(1800) });
        }
        AppAction::Launch(_) | AppAction::Open { .. } => {
            *toast = Some(Toast { text: "Not available in the standalone app".into(), until: Instant::now() + Duration::from_millis(1800) });
        }
    }
}

/// Hardware media keys: volume / brightness steps with a toast.
fn media(sys: &Arc<dyn Backend>, key: MediaKey) -> Option<String> {
    match key {
        MediaKey::VolumeUp | MediaKey::VolumeDown => {
            let (vol, muted) = sys.volume().unwrap_or((50, false));
            let next = if key == MediaKey::VolumeUp { (vol + 10).min(100) } else { vol.saturating_sub(10) };
            let _ = sys.set_volume(next);
            Some(format!("Volume {}%{}", next, if muted { " (muted)" } else { "" }))
        }
        MediaKey::Mute => {
            let (vol, muted) = sys.volume().unwrap_or((50, false));
            let _ = sys.set_volume(if muted { vol } else { 0 });
            Some(if muted { "Sound on".into() } else { "Muted".into() })
        }
        MediaKey::BrightnessUp | MediaKey::BrightnessDown => {
            let (cur, max) = sys.brightness().unwrap_or((50, 100));
            let step = (max / 10).max(1);
            let next = if key == MediaKey::BrightnessUp { (cur + step).min(max) } else { cur.saturating_sub(step) };
            let _ = sys.set_brightness(next.max(1));
            Some(format!("Brightness {}%", next * 100 / max.max(1)))
        }
        _ => None,
    }
}
