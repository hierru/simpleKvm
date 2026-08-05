//! simpleKvm server — runs on the Windows PC that owns the physical
//! keyboard/mouse. GUI (egui) front-end over the capture engine that hooks
//! input and streams it to the macOS client when the cursor crosses the edge.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod engine;
#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod net;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() -> eframe::Result<()> {
    // Single-instance guard: hold a fixed loopback port for our lifetime. A
    // second copy would fail to bind 24800 anyway ("bind 실패: 10048") but only
    // after the user clicks start — surface it clearly at launch instead.
    // KVM_ALLOW_SECOND skips the guard for development (e.g. previewing UI
    // changes while a real server is running).
    let _lock = if std::env::var_os("KVM_ALLOW_SECOND").is_some() {
        None
    } else {
        match std::net::TcpListener::bind(("127.0.0.1", 24798)) {
        Ok(l) => Some(l),
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

    // Autostarted copies launch with --minimized and come up in the tray only.
    let start_hidden = std::env::args().any(|a| a == "--minimized");

    // The window is controlled from the tray, so it skips the taskbar. Hidden
    // means moved far off-screen: eframe stops calling `ui` for a window made
    // Visible(false), which would deadlock re-showing (see the client app).
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([440.0, 600.0])
        .with_min_inner_size([380.0, 460.0])
        .with_title("simpleKvm 서버")
        .with_taskbar(false);
    if start_hidden {
        viewport = viewport.with_position([12000.0, 12000.0]);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "simpleKvm 서버",
        options,
        Box::new(move |cc| {
            use std::sync::{Arc, Mutex};

            install_korean_font(&cc.egui_ctx);

            let autostart_on = autostart::is_enabled();
            let tray = tray::Tray::build(autostart_on);

            // Raw window handle so tray handlers can drag the window back
            // on-screen even if the hidden window stopped repainting.
            let hwnd = find_own_hwnd();

            // Route menu clicks into a queue the app drains each frame.
            let queue: Arc<Mutex<Vec<tray_icon::menu::MenuId>>> =
                Arc::new(Mutex::new(Vec::new()));
            {
                let q = queue.clone();
                let ctx = cc.egui_ctx.clone();
                let open_id = tray.as_ref().map(|t| t.open_id.clone());
                let quit_id = tray.as_ref().map(|t| t.quit_id.clone());
                tray_icon::menu::MenuEvent::set_event_handler(Some(
                    move |ev: tray_icon::menu::MenuEvent| {
                        let bring_up = Some(&ev.id) == open_id.as_ref()
                            || Some(&ev.id) == quit_id.as_ref();
                        q.lock().unwrap().push(ev.id);
                        if bring_up {
                            force_show_window(hwnd);
                        }
                        ctx.request_repaint();
                    },
                ));
            }
            {
                let q = queue.clone();
                let ctx = cc.egui_ctx.clone();
                let open_id = tray.as_ref().map(|t| t.open_id.clone());
                tray_icon::TrayIconEvent::set_event_handler(Some(
                    move |ev: tray_icon::TrayIconEvent| {
                        if let tray_icon::TrayIconEvent::DoubleClick { .. } = ev {
                            if let Some(id) = &open_id {
                                q.lock().unwrap().push(id.clone());
                            }
                            force_show_window(hwnd);
                            ctx.request_repaint();
                        }
                    },
                ));
            }

            Ok(Box::new(app::App::new(cfg, tray, queue, autostart_on, start_hidden)))
        }),
    )
}

/// HWND of our main window (found by its unique title), as a Send-able isize.
/// 0 if not found.
#[cfg(windows)]
fn find_own_hwnd() -> isize {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    unsafe {
        FindWindowW(windows::core::PCWSTR::null(), w!("simpleKvm 서버"))
            .map(|h| h.0 as isize)
            .unwrap_or(0)
    }
}

/// Move the window on-screen and focus it, bypassing egui (used from tray
/// handlers, which may fire while the off-screen window is not repainting).
#[cfg(windows)]
fn force_show_window(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, SetWindowPos, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    };
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = HWND(hwnd as *mut _);
        let _ = SetWindowPos(h, HWND::default(), 120, 120, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(h);
    }
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
