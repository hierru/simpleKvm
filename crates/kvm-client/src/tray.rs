//! macOS menu-bar (status item) icon and its menu. Lives for the whole app;
//! menu clicks are delivered through muda's global `MenuEvent::receiver()`,
//! which the egui app polls each frame.

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    // Kept alive; dropping it removes the status item.
    _tray: TrayIcon,
    autostart_item: CheckMenuItem,
    pub open_id: MenuId,
    pub quit_id: MenuId,
    pub autostart_id: MenuId,
}

impl Tray {
    pub fn build(autostart_on: bool) -> Option<Tray> {
        let open_item = MenuItem::new("설정 열기", true, None);
        let autostart_item = CheckMenuItem::new("로그인 시 자동 실행", true, autostart_on, None);
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
            .with_tooltip("simpleKvm")
            .with_icon(make_icon())
            .with_icon_as_template(true)
            .build()
            .ok()?;

        Some(Tray { _tray: tray, autostart_item, open_id, quit_id, autostart_id })
    }

    pub fn set_autostart_checked(&self, checked: bool) {
        self.autostart_item.set_checked(checked);
    }
}

/// A 22×22 template icon (filled black ring on transparent); macOS tints it to
/// match the menu bar in light/dark mode.
fn make_icon() -> Icon {
    let size: u32 = 22;
    let c = size as f32 / 2.0;
    let outer = c - 1.5;
    let inner = c - 6.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let on = d <= outer && d >= inner;
            rgba.extend_from_slice(if on { &[0, 0, 0, 255] } else { &[0, 0, 0, 0] });
        }
    }
    Icon::from_rgba(rgba, size, size).expect("valid icon bytes")
}
