//! `snare-desktop` (§5.3) — the native desktop face.
//!
//! A deliberately thin Tauri shell: it opens a native window pointed at the
//! daemon's dashboard (`http://127.0.0.1:9000/` by default), so the desktop app
//! *is* the web frontend running natively — no duplicated logic. Override the
//! target with `SNARE_URL` (e.g. a remote `snared`).

// Windows: no console window in a GUI build.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main() {
    let url = std::env::var("SNARE_URL").unwrap_or_else(|_| "http://127.0.0.1:9000/".into());
    let parsed: tauri::Url = url.parse().expect("invalid SNARE_URL");

    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(parsed))
                .title("🪤 Snare — Live")
                .inner_size(1400.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Snare desktop");
}
