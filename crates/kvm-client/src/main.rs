//! simpleKvm client — runs on the Mac. GUI (egui) front-end over the worker
//! that connects to the Windows server, receives input events, and injects
//! them with CoreGraphics (CGEvent).
//!
//! Requires the Accessibility permission:
//! System Settings > Privacy & Security > Accessibility > add this app
//! (or the terminal it is launched from).
#![cfg_attr(
    all(target_os = "macos", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod config;

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod autostart;
#[cfg(target_os = "macos")]
mod discovery;
#[cfg(target_os = "macos")]
mod inject;
#[cfg(target_os = "macos")]
mod keymap;
#[cfg(target_os = "macos")]
mod permission;
#[cfg(target_os = "macos")]
mod tray;
#[cfg(target_os = "macos")]
mod worker;

#[cfg(target_os = "macos")]
fn main() -> eframe::Result<()> {
    use std::sync::{Arc, Mutex};

    // Single-instance guard: hold a fixed loopback port for our lifetime. If the
    // bind fails another copy is already running (e.g. launchd relaunched us at
    // login while one was open), so exit instead of showing a second menu bar
    // icon. `_lock` stays bound until main returns (process exit).
    let _lock = match std::net::TcpListener::bind(("127.0.0.1", 24799)) {
        Ok(l) => l,
        Err(_) => {
            eprintln!("simpleKvm is already running.");
            return Ok(());
        }
    };

    let cfg = config::ClientConfig::load();

    // A configured returning user starts quietly in the menu bar; first-time
    // setup (no server yet) shows the window. "Hidden" = created off-screen so
    // the event loop keeps running (a truly hidden window deadlocks re-showing).
    let start_hidden =
        !cfg.server.trim().is_empty() && std::env::var_os("KVM_SHOW").is_none();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([440.0, 640.0])
        .with_min_inner_size([380.0, 460.0])
        .with_title("simpleKvm");
    if start_hidden {
        viewport = viewport.with_position([12000.0, 12000.0]);
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "simpleKvm",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);

            let autostart_on = autostart::is_enabled();
            let tray = tray::Tray::build(autostart_on);

            // Route menu clicks into a queue the app drains each frame, waking
            // the egui loop even while the window is hidden.
            let queue: Arc<Mutex<Vec<tray_icon::menu::MenuId>>> = Arc::new(Mutex::new(Vec::new()));
            {
                let q = queue.clone();
                let ctx = cc.egui_ctx.clone();
                tray_icon::menu::MenuEvent::set_event_handler(Some(
                    move |ev: tray_icon::menu::MenuEvent| {
                        q.lock().unwrap().push(ev.id);
                        ctx.request_repaint();
                    },
                ));
            }

            Ok(Box::new(app::App::new(cfg, tray, queue, autostart_on, start_hidden)))
        }),
    )
}

/// egui's default fonts lack Korean glyphs, so labels render as tofu boxes.
/// Load a system Korean font (present on every macOS) and give it top priority.
#[cfg(target_os = "macos")]
fn install_korean_font(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ];

    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("kr".to_owned(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "kr".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("warning: no Korean-capable system font found; labels may show as boxes");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    let _ = config::ClientConfig::load();
    eprintln!("kvm-client only runs on macOS. Build kvm-server on the Windows PC instead.");
    std::process::exit(1);
}
