//! egui front-end for the Windows server: edits settings, starts/stops the
//! capture engine, and shows live status.

use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Stroke, TextStyle};
use kvm_protocol::Side;

use crate::config::ServerConfig;
use crate::engine::Engine;
use crate::hooks;

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
    layout: Option<String>,
    notice: Option<String>,
    styled: bool,
}

impl App {
    pub fn new(cfg: ServerConfig) -> Self {
        App { cfg, engine: None, layout: None, notice: None, styled: false }
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
                    if ui.button("모니터 배치 보기").clicked() {
                        self.layout = Some(hooks::layout_string(self.cfg.mac_side));
                    }
                    if let Some(layout) = &self.layout {
                        ui.add_space(8.0);
                        card(ui, "모니터 배치", |ui| {
                            ui.label(RichText::new(layout).monospace().size(11.0).color(TEXT));
                        });
                    }

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
