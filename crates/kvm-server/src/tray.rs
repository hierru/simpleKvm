//! Windows notification-area (tray) icon and its menu. Lives for the whole
//! app; menu clicks are delivered through muda's global menu event handler,
//! which the egui app polls each frame. The icon is a colored dot reflecting
//! the server state.

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayState {
    /// Engine not running.
    Stopped,
    /// Listening, no client attached.
    Listening,
    /// A client is connected.
    Connected,
}

impl TrayState {
    fn color(self) -> [u8; 3] {
        match self {
            TrayState::Stopped => [138, 143, 152],  // gray
            TrayState::Listening => [214, 175, 40], // amber
            TrayState::Connected => [70, 190, 120], // green
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            TrayState::Stopped => "simpleKvm 서버 — 중지됨",
            TrayState::Listening => "simpleKvm 서버 — 클라이언트 대기 중",
            TrayState::Connected => "simpleKvm 서버 — 연결됨",
        }
    }
}

pub struct Tray {
    // Kept alive; dropping it removes the tray icon.
    tray: TrayIcon,
    autostart_item: CheckMenuItem,
    pub open_id: MenuId,
    pub quit_id: MenuId,
    pub autostart_id: MenuId,
}

impl Tray {
    pub fn build(autostart_on: bool) -> Option<Tray> {
        let open_item = MenuItem::new("열기", true, None);
        let autostart_item =
            CheckMenuItem::new("Windows 시작 시 자동 실행", true, autostart_on, None);
        let quit_item = MenuItem::new("종료", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])
        .ok()?;

        let open_id = open_item.id().clone();
        let quit_id = quit_item.id().clone();
        let autostart_id = autostart_item.id().clone();

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(TrayState::Stopped.tooltip())
            .with_icon(make_icon(TrayState::Stopped.color()))
            .build()
            .ok()?;

        Some(Tray { tray, autostart_item, open_id, quit_id, autostart_id })
    }

    pub fn set_autostart_checked(&self, checked: bool) {
        self.autostart_item.set_checked(checked);
    }

    pub fn set_state(&self, state: TrayState) {
        let _ = self.tray.set_icon(Some(make_icon(state.color())));
        let _ = self.tray.set_tooltip(Some(state.tooltip()));
    }
}

/// A 32×32 filled dot with an anti-aliased edge in the given color.
fn make_icon(color: [u8; 3]) -> Icon {
    let size: u32 = 32;
    let c = size as f32 / 2.0;
    let radius = c - 4.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = ((radius - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            rgba.extend_from_slice(&[color[0], color[1], color[2], alpha]);
        }
    }
    Icon::from_rgba(rgba, size, size).expect("valid icon bytes")
}
