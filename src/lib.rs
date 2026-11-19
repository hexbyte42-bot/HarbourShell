//! Harbour OS - ライブラリルート。
//! バイナリ（main.rs / src/bin/*）から共有できるようにモジュールを再エクスポートする。

pub mod app;
pub mod assets;
pub mod bdf;
pub mod dither;
pub mod font;
pub mod grid;
pub mod ime;
pub mod input;
pub mod jis;
pub mod pdfont;
pub mod snake;
pub mod text_editor;
pub mod theme;
pub mod ui;

// Cardputer Zero launcher layers (native Linux/macOS only; not part of the web build).
#[cfg(not(target_os = "emscripten"))]
pub mod chrome;
#[cfg(not(target_os = "emscripten"))]
pub mod launcher;
#[cfg(not(target_os = "emscripten"))]
pub mod platform;
#[cfg(not(target_os = "emscripten"))]
pub mod system;
