//! egui front-end for the Mac client: a menu-bar (status item) app. The window
//! holds a carefully styled settings/status UI and hides (rather than quits)
//! when closed; the tray menu opens it, toggles login-at-start, and quits.
//!
//! Hiding: eframe stops calling `ui` once the root window is `Visible(false)`,
//! which permanently deadlocks re-showing (verified). So "hidden" is instead a
//! move far off-screen — the window stays visible to winit, the event loop keeps
//! running, and the tray can pull it back on-screen.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, RichText, Stroke, TextStyle};
use tray_icon::menu::MenuId;

use crate::autostart;
use crate::config::ClientConfig;
use crate::permission;
use crate::tray::Tray;
use crate::worker::{ConnState, Worker};

// ---- palette -------------------------------------------------------------
const BG: Color32 = Color32::from_rgb(24, 25, 28);
const CARD: Color32 = Color32::from_rgb(33, 35, 40);
const CARD_STROKE: Color32 = Color32::from_rgb(48, 51, 58);
const ACCENT: Color32 = Color32::from_rgb(76, 141, 255);
const DANGER: Color32 = Color32::from_rgb(214, 78, 78);
const TEXT: Color32 = Color32::from_rgb(232, 234, 238);
const MUTED: Color32 = Color32::from_rgb(150, 155, 164);
const OK: Color32 = Color32::from_rgb(70, 190, 120);
const WARN: Color32 = Color32::from_rgb(224, 176, 60);

/// Far enough off every display to be invisible; the window stays "visible" to
/// winit so the event loop keeps ticking.
const OFFSCREEN: Pos2 = Pos2::new(12000.0, 12000.0);
const DEFAULT_POS: Pos2 = Pos2::new(160.0, 120.0);

pub struct App {
    cfg: ClientConfig,
    worker: Option<Worker>,
    tray: Option<Tray>,
    menu_queue: Arc<Mutex<Vec<MenuId>>>,
    autostart_on: bool,
    notice: Option<String>,
    hidden: bool,
    shown_pos: Option<Pos2>,
    styled: bool,
    first_frame: bool,
    discovery: Option<crate::discovery::Discovery>,
}

impl App {
    pub fn new(
        cfg: ClientConfig,
        tray: Option<Tray>,
        menu_queue: Arc<Mutex<Vec<MenuId>>>,
        autostart_on: bool,
        start_hidden: bool,
    ) -> Self {
        App {
            cfg,
            worker: None,
            tray,
            menu_queue,
            autostart_on,
            notice: None,
            hidden: start_hidden,
            shown_pos: None,
            styled: false,
            first_frame: true,
            discovery: crate::discovery::Discovery::start(),
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.notice = self.cfg.save().err().map(|e| format!("설정 저장 실패: {e}"));
        self.worker = Some(Worker::start(self.cfg.clone(), ctx.clone()));
    }

    fn disconnect(&mut self) {
        if let Some(mut w) = self.worker.take() {
            w.stop();
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        let pos = self.shown_pos.unwrap_or(DEFAULT_POS);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(OFFSCREEN));
    }

    fn set_autostart(&mut self, enabled: bool) {
        match autostart::set(enabled) {
            Ok(()) => {
                self.autostart_on = enabled;
                if let Some(t) = &self.tray {
                    t.set_autostart_checked(enabled);
                }
            }
            Err(e) => self.notice = Some(format!("자동 실행 설정 실패: {e}")),
        }
    }

    fn handle_menu_events(&mut self, ctx: &egui::Context) {
        let ids: Vec<MenuId> = std::mem::take(&mut *self.menu_queue.lock().unwrap());
        for id in ids {
            let Some(tray) = &self.tray else { continue };
            let (open, quit, auto) =
                (tray.open_id.clone(), tray.quit_id.clone(), tray.autostart_id.clone());
            if id == open {
                self.show(ctx);
            } else if id == quit {
                self.disconnect();
                std::process::exit(0);
            } else if id == auto {
                let target = !self.autostart_on;
                self.set_autostart(target);
            }
        }
    }

    fn connection_badge(&self) -> (Color32, &'static str) {
        match &self.worker {
            None => (MUTED, "연결 안 됨"),
            Some(w) => match w.status.lock().unwrap().state {
                ConnState::Idle => (MUTED, "대기 중"),
                ConnState::Connecting => (WARN, "연결 중…"),
                ConnState::Connected => (OK, "연결됨"),
            },
        }
    }
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = BG;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(40, 68, 120);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(44, 47, 53);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 47, 53);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(54, 58, 66);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(60, 64, 72);
        let r = CornerRadius::same(7);
        style.visuals.widgets.inactive.corner_radius = r;
        style.visuals.widgets.hovered.corner_radius = r;
        style.visuals.widgets.active.corner_radius = r;
        style.visuals.widgets.open.corner_radius = r;

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.interact_size.y = 30.0;

        use FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (TextStyle::Small, FontId::new(11.0, Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, Monospace)),
        ]
        .into();
    });
}

/// A titled, padded card container.
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) {
    ui.label(RichText::new(title).size(12.0).color(MUTED).strong());
    ui.add_space(5.0);
    egui::Frame::group(ui.style())
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(14.0);
}

/// A label above a full-width input.
fn field(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(label).size(12.0).color(MUTED));
    ui.add_space(3.0);
    add(ui);
    ui.add_space(9.0);
}

fn status_badge(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(40, 43, 49))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(11, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("●").color(color).size(11.0));
                ui.label(RichText::new(text).size(12.0).color(TEXT));
            });
        });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.styled {
            self.styled = true;
            setup_style(&ctx);
        }
        // Keep the loop turning so menu clicks and status changes are picked up.
        ctx.request_repaint_after(Duration::from_millis(400));

        self.handle_menu_events(&ctx);

        if self.first_frame {
            self.first_frame = false;
            // No tray to reopen from: never leave the window stranded off-screen.
            if self.hidden && self.tray.is_none() {
                self.show(&ctx);
            }
        }

        // Remember where the user keeps the window, so we restore it on re-open.
        if !self.hidden {
            if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                if rect.min.x < 8000.0 && rect.min.y < 8000.0 {
                    self.shown_pos = Some(rect.min);
                }
            }
        }

        // Closing the window hides it to the menu bar instead of quitting.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(&ctx);
        }

        let running = self.worker.is_some();
        let (badge_color, badge_text) = self.connection_badge();

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::same(20)))
            .show(ui, |ui| {
              egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                // Accessibility permission: without it, injection is silently
                // dropped, so connected-but-nothing-happens looks like a bug.
                if !permission::is_trusted() {
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(58, 40, 30))
                        .stroke(Stroke::new(1.0, DANGER))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new("● 손쉬운 사용 권한 필요").color(WARN).strong().size(14.0),
                            );
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new(
                                    "이 권한이 없으면 연결되어도 마우스·키보드가 Mac에 \
                                     주입되지 않습니다. 목록에 simpleKvm을 추가·체크하세요. \
                                     (앱을 다시 빌드하면 권한을 다시 추가해야 합니다.)",
                                )
                                .size(12.0)
                                .color(TEXT),
                            );
                            ui.add_space(8.0);
                            if ui.button("손쉬운 사용 설정 열기").clicked() {
                                permission::open_settings();
                            }
                        });
                    ui.add_space(14.0);
                }
                // Header: title + subtitle on the left, status badge on the right.
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("simpleKvm").size(22.0).strong().color(TEXT));
                        ui.label(
                            RichText::new("Windows → Mac 입력 공유").size(12.0).color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        status_badge(ui, badge_color, badge_text);
                    });
                });

                ui.add_space(16.0);

                ui.add_enabled_ui(!running, |ui| {
                    card(ui, "연결 설정", |ui| {
                        field(ui, "서버 IP / 호스트", |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.cfg.server)
                                    .hint_text("예: 192.168.0.5")
                                    .desired_width(f32::INFINITY),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(96.0);
                                ui.label(RichText::new("포트").size(12.0).color(MUTED));
                                ui.add_space(3.0);
                                ui.add(
                                    egui::DragValue::new(&mut self.cfg.port).range(1..=65535),
                                );
                            });
                            ui.add_space(14.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new("이름").size(12.0).color(MUTED));
                                ui.add_space(3.0);
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.cfg.name)
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        });

                        // Servers advertising themselves on the LAN (mDNS).
                        let discovered = self
                            .discovery
                            .as_ref()
                            .map(|d| d.servers.lock().unwrap().clone())
                            .unwrap_or_default();
                        if !discovered.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("네트워크에서 발견된 서버")
                                    .size(12.0)
                                    .color(MUTED),
                            );
                            ui.add_space(3.0);
                            for s in discovered {
                                ui.horizontal(|ui| {
                                    if ui.small_button("사용").clicked() {
                                        self.cfg.server = s.addr.clone();
                                        self.cfg.port = s.port;
                                    }
                                    ui.label(
                                        RichText::new(format!(
                                            "{} — {}:{}",
                                            s.name, s.addr, s.port
                                        ))
                                        .size(12.0),
                                    );
                                });
                            }
                        }
                    });

                    card(ui, "옵션", |ui| {
                        field(ui, "마우스 감도", |ui| {
                            ui.add(
                                egui::Slider::new(&mut self.cfg.speed, 0.1..=5.0)
                                    .text("배")
                                    .fixed_decimals(1),
                            );
                        });
                        ui.checkbox(
                            &mut self.cfg.ctrl_as_cmd,
                            "Windows Ctrl → Mac Command (⌘) 매핑",
                        );
                        ui.add_space(2.0);
                        let mut autostart = self.autostart_on;
                        if ui.checkbox(&mut autostart, "로그인 시 자동 실행").changed() {
                            self.set_autostart(autostart);
                        }
                    });
                });

                // Primary action, full width.
                self.action_button(ui, running, &ctx);

                if let Some(msg) = &self.notice {
                    ui.add_space(6.0);
                    ui.colored_label(WARN, RichText::new(msg).size(12.0));
                }

                ui.add_space(16.0);
                self.status_pane(ui);
                });
            });
    }
}

impl App {
    fn action_button(&mut self, ui: &mut egui::Ui, running: bool, ctx: &egui::Context) {
        let w = ui.available_width();
        if !running {
            let can = !self.cfg.server.trim().is_empty();
            let btn = egui::Button::new(RichText::new("연결").size(15.0).strong().color(Color32::WHITE))
                .fill(if can { ACCENT } else { Color32::from_rgb(60, 64, 72) })
                .corner_radius(CornerRadius::same(9))
                .min_size(egui::vec2(w, 40.0));
            if ui.add_enabled(can, btn).clicked() {
                self.connect(ctx);
            }
            if !can {
                ui.add_space(4.0);
                ui.label(RichText::new("서버 주소를 입력하면 연결할 수 있습니다.").size(11.0).color(MUTED));
            }
        } else {
            let btn = egui::Button::new(RichText::new("연결 해제").size(15.0).strong().color(TEXT))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, DANGER))
                .corner_radius(CornerRadius::same(9))
                .min_size(egui::vec2(w, 40.0));
            if ui.add(btn).clicked() {
                self.disconnect();
            }
        }
    }

    fn status_pane(&self, ui: &mut egui::Ui) {
        let Some(worker) = &self.worker else {
            card(ui, "상태", |ui| {
                ui.label(
                    RichText::new(
                        "Windows에서 kvm-server가 실행 중이어야 합니다.\n\
                         입력 주입에는 시스템 설정 → 개인정보 보호 및 보안 → \
                         손쉬운 사용 권한이 필요합니다.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            });
            return;
        };

        let status = worker.status.lock().unwrap();
        card(ui, "활동", |ui| {
            if let Some(name) = &status.server_name {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("서버").size(12.0).color(MUTED));
                    ui.label(RichText::new(name).size(13.0).color(TEXT));
                });
            }
            if !status.displays.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("인식된 디스플레이").size(12.0).color(MUTED));
                for d in &status.displays {
                    ui.label(RichText::new(format!("• {d}")).size(12.0).color(TEXT));
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("로그").size(12.0).color(MUTED));
            ui.add_space(2.0);
            egui::Frame::NONE
                .fill(Color32::from_rgb(20, 21, 24))
                .corner_radius(CornerRadius::same(7))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for line in &status.log {
                                ui.label(
                                    RichText::new(line).monospace().size(11.0).color(MUTED),
                                );
                            }
                        });
                });
        });
    }
}
