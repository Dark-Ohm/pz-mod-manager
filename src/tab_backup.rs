//! Вкладка «Бэкапы»: создание, список и безопасное восстановление.
use crate::i18n::t;
use crate::{App, Bg, backup};
use eframe::egui::{self, Color32, RichText};

impl App {
    pub fn tab_backup(&mut self, ui: &mut egui::Ui) {
        let srv = self.store.settings.server().clone();
        let running = self.server_running();
        if self.bk_list.is_empty() && self.busy == 0 {
            self.bk_list = backup::list(&srv);
        }
        ui.heading(t("Бэкапы"));
        ui.weak(tf!("Мир: {}/Saves/Multiplayer/{}  ·  хранилище: {}", crate::platform::short(&srv.cache_dir), srv.name, crate::platform::short(&backup::root(&srv))));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.bk_label).hint_text(t("метка (необязательно)")).desired_width(200.0));
            if ui.add_enabled(self.busy == 0, egui::Button::new(t("💾 Создать бэкап"))).clicked() {
                self.busy += 1;
                self.status = t("Делаю бэкап…").into();
                let (tx, label, keep, cfg) = (self.bg_tx.clone(), self.bk_label.clone(), srv.backup_keep, srv.clone());
                std::thread::spawn(move || {
                    let m = match backup::create(&cfg, &label, keep) {
                        Ok(p) => tf!("Бэкап готов: {}", p.display()),
                        Err(e) => tf!("Ошибка бэкапа: {}", format!("{e:#}")),
                    };
                    let _ = tx.send(Bg::Job(m));
                });
            }
            ui.label(t("Хранить:"));
            let mut keep = self.store.settings.server().backup_keep;
            if ui.add(egui::DragValue::new(&mut keep).range(1..=200)).changed() {
                self.store.settings.server_mut().backup_keep = keep;
                self.save();
            }
            if ui.button(t("📂 Открыть папку")).clicked() {
                let _ = std::fs::create_dir_all(backup::root(&srv));
                let _ = open::that(backup::root(&srv));
            }
            if self.busy > 0 {
                ui.spinner();
            }
        });
        if running {
            ui.colored_label(Color32::from_rgb(230, 180, 70), t("Сервер запущен: бэкап мира возможен, восстановление — только после остановки."));
        }
        ui.separator();

        let mut restore: Option<String> = None;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if self.bk_list.is_empty() {
                ui.weak(t("Бэкапов пока нет"));
            }
            for e in &self.bk_list {
                egui::Frame::new().fill(Color32::from_rgb(30, 33, 30)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&e.name).strong());
                            ui.label(RichText::new(&e.info).weak());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.bk_confirm.as_deref() == Some(e.name.as_str()) {
                                if ui.add(egui::Button::new(t("Да, восстановить")).fill(Color32::from_rgb(120, 60, 50))).clicked() {
                                    restore = Some(e.name.clone());
                                }
                                if ui.button(t("Отмена")).clicked() {
                                    self.bk_confirm = None;
                                }
                            } else if ui.add_enabled(!running && self.busy == 0, egui::Button::new(t("⟲ Восстановить"))).clicked() {
                                self.bk_confirm = Some(e.name.clone());
                            }
                        });
                    });
                    if self.bk_confirm.as_deref() == Some(e.name.as_str()) {
                        ui.colored_label(Color32::from_rgb(230, 180, 70), t("Сначала будет сделан снимок текущего мира («before-restore»), затем мир заменится."));
                    }
                });
            }
        });
        if let Some(name) = restore {
            self.bk_confirm = None;
            self.busy += 1;
            self.status = t("Восстанавливаю…").into();
            let (tx, cfg) = (self.bg_tx.clone(), srv);
            std::thread::spawn(move || {
                let m = match backup::restore(&cfg, &name, false) {
                    Ok(safety) => tf!("Восстановлено из {}. Снимок прежнего состояния: {}", name, safety.display()),
                    Err(e) => tf!("Ошибка восстановления: {}", format!("{e:#}")),
                };
                let _ = tx.send(Bg::Job(m));
            });
        }
    }
}
