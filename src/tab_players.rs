//! Вкладка «Игроки»: список из логов + команды серверу (консоль запущенного сервера или RCON).
use crate::i18n::t;
use crate::{App, commands, players};
use eframe::egui::{self, Color32, RichText};
use std::time::{Duration, Instant};

const ONLINE: Color32 = Color32::from_rgb(120, 190, 90);

impl App {
    pub fn refresh_players(&mut self) {
        let stale = self.pl_loaded.map_or(true, |t| t.elapsed() > Duration::from_secs(5));
        if stale {
            self.players = players::load(&self.store.settings.server().cache_dir);
            self.pl_loaded = Some(Instant::now());
        }
    }

    pub fn tab_players(&mut self, ui: &mut egui::Ui) {
        self.refresh_players();
        let rc = crate::rcon::from_ini(&self.store.settings.server().ini()).is_some();
        let running = self.server_running();

        egui::Panel::right("cmds").default_size(380.0).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(t("Команды"));
            if running {
                ui.colored_label(ONLINE, t("→ в консоль запущенного сервера"));
            } else if rc {
                ui.colored_label(ONLINE, t("→ через RCON (порт и пароль из ini)"));
            } else {
                ui.colored_label(Color32::from_rgb(230, 180, 70), t("Сервер не запущен здесь, RCONPassword в ini пуст — команды недоступны"));
            }
            ui.add_space(4.0);
            let sel = self.pl_sel.clone().unwrap_or_default();
            ui.label(if sel.is_empty() { RichText::new(t("Игрок не выбран")).weak() } else { RichText::new(tf!("Игрок: {}", sel)).strong() });
            ui.horizontal_wrapped(|ui| {
                for tp in commands::PLAYER_TEMPLATES {
                    let r = ui.add_enabled(!sel.is_empty(), egui::Button::new(t(tp.label)).small()).on_hover_text(t(tp.hint));
                    if r.clicked() {
                        match commands::fill(tp, &sel) {
                            Ok(c) => self.cmd_input = c,
                            Err(e) => self.cmd_log.push(format!("✘ {e}")),
                        }
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (label, cmd) in commands::SERVER_COMMANDS {
                    if ui.add(egui::Button::new(t(label)).small()).clicked() {
                        self.send_cmd(cmd.to_string());
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.broadcast).hint_text(t("сообщение всем")).desired_width(240.0));
                if ui.button(t("📢 Отправить")).clicked() && !self.broadcast.trim().is_empty() {
                    let c = commands::servermsg(&std::mem::take(&mut self.broadcast));
                    self.send_cmd(c);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.cmd_input).hint_text(t("команда, Enter — отправить")).desired_width(ui.available_width() - 80.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button(t("Отправить")).clicked() || enter) && !self.cmd_input.trim().is_empty() {
                    let c = std::mem::take(&mut self.cmd_input);
                    self.send_cmd(c);
                    r.request_focus();
                }
            });
            egui::ScrollArea::vertical().id_salt("cmdlog").stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                egui::Frame::new().fill(Color32::from_rgb(16, 18, 16)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if self.cmd_log.is_empty() {
                        ui.weak(t("Ответы сервера появятся здесь"));
                    }
                    for l in &self.cmd_log {
                        ui.label(RichText::new(l).monospace().size(12.0));
                    }
                });
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(t("Игроки"));
                if ui.button(t("⟳ Обновить")).clicked() {
                    self.pl_loaded = None;
                }
                let online = self.players.iter().filter(|p| p.online).count();
                ui.label(tf!("онлайн: {} из {}", online, self.players.len()));
            });
            ui.weak(tf!("Данные из {}/Logs", crate::platform::short(&self.store.settings.server().cache_dir)));
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                if self.players.is_empty() {
                    ui.weak(t("В логах пока нет игроков"));
                }
                for p in &self.players {
                    let selected = self.pl_sel.as_deref() == Some(p.name.as_str());
                    egui::Frame::new().fill(if selected { Color32::from_rgb(40, 52, 38) } else { Color32::from_rgb(30, 33, 30) }).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let r = ui.horizontal(|ui| {
                            ui.colored_label(if p.online { ONLINE } else { Color32::GRAY }, "●");
                            ui.label(RichText::new(&p.name).strong());
                            if !p.role.is_empty() && p.role != "none" {
                                ui.weak(format!("[{}]", p.role));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if let Some((x, y, z)) = p.pos {
                                    ui.monospace(format!("{x}, {y}, {z}"));
                                }
                            });
                        });
                        ui.weak(format!("{}  {}", p.last_seen, p.ip));
                        if r.response.interact(egui::Sense::click()).clicked() || ui.interact(ui.min_rect(), ui.id().with(&p.name), egui::Sense::click()).clicked() {
                            self.pl_sel = Some(p.name.clone());
                        }
                    });
                }
            });
        });
    }
}
