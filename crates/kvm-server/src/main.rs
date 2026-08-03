//! simpleKvm server — runs on the Windows PC that owns the physical
//! keyboard/mouse. GUI (egui) front-end over the capture engine that hooks
//! input and streams it to the macOS client when the cursor crosses the edge.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod net;

#[cfg(windows)]
fn main() -> eframe::Result<()> {
    // Single-instance guard: hold a fixed loopback port for our lifetime. A
    // second copy would fail to bind 24800 anyway ("bind 실패: 10048") but only
    // after the user clicks start — surface it clearly at launch instead.
    let _lock = match std::net::TcpListener::bind(("127.0.0.1", 24798)) {
        Ok(l) => l,
        Err(_) => {
            unsafe {
                use windows::core::w;
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    MessageBoxW, MB_ICONWARNING, MB_OK,
                };
                MessageBoxW(
                    HWND::default(),
                    w!("simpleKvm 서버가 이미 실행 중입니다. 기존 창을 사용하세요."),
                    w!("simpleKvm"),
                    MB_OK | MB_ICONWARNING,
                );
            }
            return Ok(());
        }
    };

    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // Without this, cursor coordinates are virtualized on scaled displays.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let cfg = config::ServerConfig::load();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([440.0, 600.0])
            .with_min_inner_size([380.0, 460.0])
            .with_title("simpleKvm 서버"),
        ..Default::default()
    };

    eframe::run_native(
        "simpleKvm 서버",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);
            Ok(Box::new(app::App::new(cfg)))
        }),
    )
}

/// egui's default fonts lack Korean glyphs. Load a Windows Korean system font
/// (Malgun Gothic) so labels aren't tofu boxes.
#[cfg(windows)]
fn install_korean_font(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\malgun.ttf",
        r"C:\Windows\Fonts\malgunsl.ttf",
    ];
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("kr".to_owned(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "kr".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("warning: Malgun Gothic not found; Korean labels may show as boxes");
}

#[cfg(not(windows))]
fn main() {
    let _ = config::ServerConfig::load();
    eprintln!("kvm-server only runs on Windows. Build kvm-client on the Mac instead.");
    std::process::exit(1);
}
