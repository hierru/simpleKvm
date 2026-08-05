//! egui front-end for the Windows server: edits settings, starts/stops the
//! capture engine, and shows live status.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, RichText, Stroke, TextStyle};
use kvm_protocol::Side;
use tray_icon::menu::MenuId;

use crate::autostart;
use crate::config::ServerConfig;
use crate::engine::Engine;
use crate::hooks;
use crate::tray::{Tray, TrayState};

const DEFAULT_POS: Pos2 = Pos2::new(120.0, 120.0);
const OFFSCREEN: Pos2 = Pos2::new(12000.0, 12000.0);

const BG: Color32 = Color32::from_rgb(24, 25, 28);
const CARD: Color32 = Color32::from_rgb(33, 35, 40);
const CARD_STROKE: Color32 = Color32::from_rgb(48, 51, 58);
const ACCENT: Color32 = Color32::from_rgb(76, 141, 255);
const DANGER: Color32 = Color32::from_rgb(214, 78, 78);
const TEXT: Color32 = Color32::from_rgb(232, 234, 238);
const MUTED: Color32 = Color32::from_rgb(150, 155, 164);
const OK: Color32 = Color32::from_rgb(70, 190, 120);

pub struct App {
    cfg: ServerConfig,
    engine: Option<Engine>,
    notice: Option<String>,
    styled: bool,
    tray: Option<Tray>,
    menu_queue: Arc<Mutex<Vec<MenuId>>>,
    autostart_on: bool,
    hidden: bool,
    shown_pos: Option<Pos2>,
    last_tray_state: Option<TrayState>,
    first_frame: bool,
}

impl App {
    pub fn new(
        cfg: ServerConfig,
        tray: Option<Tray>,
        menu_queue: Arc<Mutex<Vec<MenuId>>>,
        autostart_on: bool,
        start_hidden: bool,
    ) -> Self {
        let mut app = App {
            cfg,
            engine: None,
            notice: None,
            styled: false,
            tray,
            menu_queue,
            autostart_on,
            hidden: start_hidden,
            shown_pos: None,
            last_tray_state: None,
            first_frame: true,
        };
        // An autostarted (tray-only) server should serve without requiring the
        // user to open the window and click start.
        if start_hidden {
            app.start();
        }
        app
    }

    fn start(&mut self) {
        self.notice = self.cfg.save().err().map(|e| format!("설정 저장 실패: {e}"));
        self.engine = Some(Engine::start(&self.cfg));
    }

    fn stop(&mut self) {
        if let Some(mut e) = self.engine.take() {
            e.stop();
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        let pos = self.shown_pos.unwrap_or(DEFAULT_POS);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// "Hidden" = moved far off-screen so the event loop keeps running (see
    /// main.rs); the tray brings it back.
    fn hide(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        self.shown_pos = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
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
                self.stop();
                std::process::exit(0);
            } else if id == auto {
                let target = !self.autostart_on;
                self.set_autostart(target);
            }
        }
    }

    fn tray_state(&self) -> TrayState {
        match &self.engine {
            None => TrayState::Stopped,
            Some(e) => {
                let s = e.status.lock().unwrap();
                if s.client.is_some() {
                    TrayState::Connected
                } else if s.listening {
                    TrayState::Listening
                } else {
                    TrayState::Stopped
                }
            }
        }
    }

    fn sync_tray(&mut self) {
        let state = self.tray_state();
        if self.last_tray_state != Some(state) {
            if let Some(t) = &self.tray {
                t.set_state(state);
            }
            self.last_tray_state = Some(state);
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

/// Windows-display-settings-style arrangement map: proportional rounded
/// rectangles with big numbers. Monitors owning the shared (Mac-side) edge
/// are filled with the accent color.
fn monitor_map(ui: &mut egui::Ui, rects: &[(f32, f32, f32, f32)], mac_side: Option<Side>) {
    if rects.is_empty() {
        ui.label(RichText::new("모니터를 찾지 못했습니다.").size(12.0).color(MUTED));
        return;
    }
    let min_x = rects.iter().map(|r| r.0).fold(f32::MAX, f32::min);
    let min_y = rects.iter().map(|r| r.1).fold(f32::MAX, f32::min);
    let max_x = rects.iter().map(|r| r.0 + r.2).fold(f32::MIN, f32::max);
    let max_y = rects.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
    let world_w = (max_x - min_x).max(1.0);
    let world_h = (max_y - min_y).max(1.0);

    let pad = 12.0;
    let avail = ui.available_width();
    let scale = ((avail - pad * 2.0) / world_w).min(150.0 / world_h);
    let (response, painter) = ui.allocate_painter(
        egui::vec2(avail, world_h * scale + pad * 2.0),
        egui::Sense::hover(),
    );
    let panel = response.rect;
    painter.rect_filled(panel, CornerRadius::same(8), Color32::from_rgb(20, 21, 24));

    let origin = egui::pos2(
        panel.center().x - world_w * scale / 2.0,
        panel.min.y + pad,
    );
    for (i, r) in rects.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(origin.x + (r.0 - min_x) * scale, origin.y + (r.1 - min_y) * scale),
            egui::vec2(r.2 * scale, r.3 * scale),
        )
        .shrink(2.5);
        let owns_edge = match mac_side {
            Some(Side::Left) => (r.0 - min_x).abs() < 0.5,
            Some(Side::Right) => (r.0 + r.2 - max_x).abs() < 0.5,
            None => false,
        };
        let fill = if owns_edge { ACCENT } else { Color32::from_rgb(44, 47, 53) };
        painter.rect_filled(rect, CornerRadius::same(6), fill);
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, if owns_edge { ACCENT } else { CARD_STROKE }),
            egui::StrokeKind::Inside,
        );
        let num_size = (rect.height() * 0.36).clamp(16.0, 34.0);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            (i + 1).to_string(),
            FontId::new(num_size, FontFamily::Proportional),
            Color32::WHITE,
        );
        if rect.height() > 46.0 {
            painter.text(
                egui::pos2(rect.center().x, rect.bottom() - 6.0),
                egui::Align2::CENTER_BOTTOM,
                format!("{:.0}×{:.0}", r.2, r.3),
                FontId::new(10.0, FontFamily::Proportional),
                Color32::from_rgba_unmultiplied(255, 255, 255, 170),
            );
        }
    }
}

/// Combined arrangement of both machines while connected: each side is scaled
/// so the two facing edge-spans render the same height — which is exactly how
/// cursor positions map across the shared edge — and joined with accent lines.
/// `remote_side` is the side of the local machine the remote machine sits on.
fn combined_map(
    ui: &mut egui::Ui,
    local_label: &str,
    local: &[(f32, f32, f32, f32)],
    remote_label: &str,
    remote: &[(f32, f32, f32, f32)],
    remote_side: Side,
) {
    if local.is_empty() || remote.is_empty() {
        return;
    }

    struct Group<'a> {
        label: &'a str,
        rects: &'a [(f32, f32, f32, f32)],
        is_local: bool,
        facing: Side,
    }
    let (left, right) = match remote_side {
        Side::Left => (
            Group { label: remote_label, rects: remote, is_local: false, facing: Side::Right },
            Group { label: local_label, rects: local, is_local: true, facing: Side::Left },
        ),
        Side::Right => (
            Group { label: local_label, rects: local, is_local: true, facing: Side::Right },
            Group { label: remote_label, rects: remote, is_local: false, facing: Side::Left },
        ),
    };

    fn bounds(rects: &[(f32, f32, f32, f32)]) -> (f32, f32, f32, f32) {
        let min_x = rects.iter().map(|r| r.0).fold(f32::MAX, f32::min);
        let min_y = rects.iter().map(|r| r.1).fold(f32::MAX, f32::min);
        let max_x = rects.iter().map(|r| r.0 + r.2).fold(f32::MIN, f32::max);
        let max_y = rects.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
        (min_x, min_y, max_x, max_y)
    }
    fn facing_span(rects: &[(f32, f32, f32, f32)], facing: Side) -> (f32, f32) {
        let (min_x, min_y, max_x, max_y) = bounds(rects);
        let mut top = f32::MAX;
        let mut bottom = f32::MIN;
        for r in rects {
            let owns = match facing {
                Side::Right => r.0 + r.2 >= max_x - 0.5,
                Side::Left => r.0 <= min_x + 0.5,
            };
            if owns {
                top = top.min(r.1);
                bottom = bottom.max(r.1 + r.3);
            }
        }
        if top < bottom { (top, bottom) } else { (min_y, max_y) }
    }

    const SPAN_PX: f32 = 100.0;
    const GAP: f32 = 16.0;
    let pad = 12.0;
    let label_h = 18.0;

    let groups = [&left, &right];
    let mut b = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); 2];
    let mut span = [(0.0f32, 0.0f32); 2];
    let mut scale = [0.0f32; 2];
    for (i, g) in groups.iter().enumerate() {
        b[i] = bounds(g.rects);
        span[i] = facing_span(g.rects, g.facing);
        scale[i] = SPAN_PX / (span[i].1 - span[i].0).max(1.0);
    }
    let span_off = [(span[0].0 - b[0].1) * scale[0], (span[1].0 - b[1].1) * scale[1]];
    let align = span_off[0].max(span_off[1]);
    let y0 = [align - span_off[0], align - span_off[1]];
    let size = [
        ((b[0].2 - b[0].0) * scale[0], (b[0].3 - b[0].1) * scale[0]),
        ((b[1].2 - b[1].0) * scale[1], (b[1].3 - b[1].1) * scale[1]),
    ];
    let content_w = size[0].0 + GAP + size[1].0;
    let content_h = (y0[0] + size[0].1).max(y0[1] + size[1].1);

    let avail = ui.available_width();
    let fit = ((avail - pad * 2.0) / content_w).min(1.0);
    let (response, painter) = ui.allocate_painter(
        egui::vec2(avail, content_h * fit + label_h + pad * 2.0),
        egui::Sense::hover(),
    );
    let panel = response.rect;
    painter.rect_filled(panel, CornerRadius::same(8), Color32::from_rgb(20, 21, 24));

    let origin_x = panel.center().x - content_w * fit / 2.0;
    let origin_y = panel.min.y + pad + label_h;
    let group_x = [origin_x, origin_x + (size[0].0 + GAP) * fit];

    for (i, g) in groups.iter().enumerate() {
        let s = scale[i] * fit;
        let gx = group_x[i];
        let gy = origin_y + y0[i] * fit;

        painter.text(
            egui::pos2(gx + size[i].0 * fit / 2.0, panel.min.y + pad),
            egui::Align2::CENTER_TOP,
            g.label,
            FontId::new(11.0, FontFamily::Proportional),
            MUTED,
        );

        let (fill, stroke_c) = if g.is_local {
            (Color32::from_rgb(44, 47, 53), CARD_STROKE)
        } else {
            (Color32::from_rgb(36, 46, 66), Color32::from_rgb(58, 82, 128))
        };
        for (j, r) in g.rects.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                egui::pos2(gx + (r.0 - b[i].0) * s, gy + (r.1 - b[i].1) * s),
                egui::vec2(r.2 * s, r.3 * s),
            )
            .shrink(2.0);
            painter.rect_filled(rect, CornerRadius::same(5), fill);
            painter.rect_stroke(
                rect,
                CornerRadius::same(5),
                Stroke::new(1.0, stroke_c),
                egui::StrokeKind::Inside,
            );
            let num_size = (rect.height() * 0.34).clamp(12.0, 28.0);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                (j + 1).to_string(),
                FontId::new(num_size, FontFamily::Proportional),
                Color32::WHITE,
            );
        }

        // Accent line along the shared (facing) edge, spanning the mapped rows.
        let edge_x = match g.facing {
            Side::Right => gx + size[i].0 * fit,
            Side::Left => gx,
        };
        let sy0 = gy + (span[i].0 - b[i].1) * s;
        let sy1 = gy + (span[i].1 - b[i].1) * s;
        painter.line_segment(
            [egui::pos2(edge_x, sy0), egui::pos2(edge_x, sy1)],
            Stroke::new(3.0, ACCENT),
        );
    }

    // Faint connector between the two shared edges.
    let mid_y = origin_y + (align + SPAN_PX / 2.0) * fit;
    painter.line_segment(
        [
            egui::pos2(group_x[0] + size[0].0 * fit + 2.0, mid_y),
            egui::pos2(group_x[1] - 2.0, mid_y),
        ],
        Stroke::new(1.5, Color32::from_rgba_unmultiplied(76, 141, 255, 140)),
    );
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
        ctx.request_repaint_after(Duration::from_millis(400));

        self.handle_menu_events(&ctx);
        self.sync_tray();

        if self.first_frame {
            self.first_frame = false;
            // No tray to reopen from: never leave the window stranded off-screen.
            if self.hidden && self.tray.is_none() {
                self.show(&ctx);
            }
        }

        // Closing the window hides it to the tray instead of quitting.
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(&ctx);
        }

        let running = self.engine.is_some();
        let (badge_color, badge_text) = if let Some(e) = &self.engine {
            let s = e.status.lock().unwrap();
            if s.client.is_some() {
                (OK, "클라이언트 연결됨")
            } else if s.listening {
                (Color32::from_rgb(200, 160, 0), "대기 중 (리슨)")
            } else {
                (MUTED, "시작 중…")
            }
        } else {
            (MUTED, "중지됨")
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::same(20)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("simpleKvm 서버").size(22.0).strong().color(TEXT));
                            ui.label(RichText::new("Windows → Mac 입력 공유").size(12.0).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            status_badge(ui, badge_color, badge_text);
                        });
                    });
                    ui.add_space(16.0);

                    ui.add_enabled_ui(!running, |ui| {
                        card(ui, "연결 설정", |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(96.0);
                                    ui.label(RichText::new("포트").size(12.0).color(MUTED));
                                    ui.add_space(3.0);
                                    ui.add(egui::DragValue::new(&mut self.cfg.port).range(1..=65535));
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
                            ui.add_space(9.0);
                            ui.label(RichText::new("Mac 위치 (이 PC 화면 기준)").size(12.0).color(MUTED));
                            ui.add_space(3.0);
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.cfg.mac_side, Side::Left, "왼쪽");
                                ui.selectable_value(&mut self.cfg.mac_side, Side::Right, "오른쪽");
                            });
                        });
                    });

                    card(ui, "일반", |ui| {
                        let mut auto = self.autostart_on;
                        if ui.checkbox(&mut auto, "Windows 시작 시 자동 실행").changed() {
                            self.set_autostart(auto);
                        }
                        ui.label(
                            RichText::new(
                                "자동 실행 시 트레이로 조용히 시작됩니다. 창을 닫아도 종료되지 \
                                 않고 트레이로 내려가며, 종료는 트레이 메뉴에서 하세요.",
                            )
                            .size(11.0)
                            .color(MUTED),
                        );
                    });

                    // Start/stop, full width.
                    let w = ui.available_width();
                    if !running {
                        let btn = egui::Button::new(
                            RichText::new("시작").size(15.0).strong().color(Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .corner_radius(CornerRadius::same(9))
                        .min_size(egui::vec2(w, 40.0));
                        if ui.add(btn).clicked() {
                            self.start();
                        }
                    } else {
                        let btn = egui::Button::new(
                            RichText::new("중지").size(15.0).strong().color(TEXT),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, DANGER))
                        .corner_radius(CornerRadius::same(9))
                        .min_size(egui::vec2(w, 40.0));
                        if ui.add(btn).clicked() {
                            self.stop();
                        }
                    }

                    if let Some(msg) = &self.notice {
                        ui.add_space(6.0);
                        ui.colored_label(DANGER, RichText::new(msg).size(12.0));
                    }

                    ui.add_space(14.0);
                    let client_layout = self
                        .engine
                        .as_ref()
                        .and_then(|e| e.status.lock().unwrap().client_layout.clone());
                    card(ui, "모니터 배치", |ui| {
                        if let Some(remote) = &client_layout {
                            let remote: Vec<(f32, f32, f32, f32)> =
                                remote.iter().map(|d| (d.x, d.y, d.w, d.h)).collect();
                            combined_map(
                                ui,
                                "Windows (이 PC)",
                                &hooks::ui_monitor_rects(),
                                "Mac",
                                &remote,
                                self.cfg.mac_side,
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(
                                    "연결된 Mac을 포함한 전체 배치입니다. 파란 선이 서로 \
                                     이어지는 전환 엣지이며, 커서는 같은 높이로 넘어갑니다.",
                                )
                                .size(11.0)
                                .color(MUTED),
                            );
                        } else {
                            monitor_map(ui, &hooks::ui_monitor_rects(), Some(self.cfg.mac_side));
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(
                                    "파란색 모니터의 바깥쪽 가장자리를 넘으면 Mac으로 전환됩니다. \
                                     Mac이 연결되면 전체 배치가 표시됩니다.",
                                )
                                .size(11.0)
                                .color(MUTED),
                            );
                        }
                    });

                    ui.add_space(8.0);
                    self.status_pane(ui);
                });
            });
    }
}

impl App {
    fn status_pane(&self, ui: &mut egui::Ui) {
        let Some(engine) = &self.engine else {
            card(ui, "상태", |ui| {
                ui.label(
                    RichText::new(
                        "시작을 누르면 이 PC의 키보드/마우스를 캡처합니다. 커서를 Mac 쪽 \
                         화면 가장자리 너머로 밀면 제어가 Mac으로 넘어갑니다. 비상 복귀: \
                         Ctrl+Alt+F12.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            });
            return;
        };
        let status = engine.status.lock().unwrap();
        card(ui, "활동", |ui| {
            if let Some(name) = &status.client {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("클라이언트").size(12.0).color(MUTED));
                    ui.label(RichText::new(name).size(13.0).color(TEXT));
                });
                ui.add_space(6.0);
            }
            ui.label(RichText::new("로그").size(12.0).color(MUTED));
            ui.add_space(2.0);
            egui::Frame::NONE
                .fill(Color32::from_rgb(20, 21, 24))
                .corner_radius(CornerRadius::same(7))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for line in &status.log {
                                ui.label(RichText::new(line).monospace().size(11.0).color(MUTED));
                            }
                        });
                });
        });
    }
}
