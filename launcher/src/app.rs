//! The HUD: header, deployment panel, status column and operation log.
//! All chrome is painted here so the layout stays identical on Linux and Windows.

use crate::config::{self, Config};
use crate::jobs::{self, Job, Level, Msg};
use crate::pack::{Manifest, PackInfo};
use crate::theme;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind,
};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

struct Line {
    level: Level,
    time: String,
    text: String,
}

pub struct Launcher {
    cfg: Config,
    backup: Config,
    settings: bool,
    info: Option<PackInfo>,
    info_error: Option<String>,
    manifest: Option<Manifest>,
    manifest_error: Option<String>,
    checked_at: Option<String>,
    log: Vec<Line>,
    job: Option<Job>,
    progress: Option<(u64, Option<u64>)>,
    downloaded: Option<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Launcher {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut cfg = Config::load();
        cfg.normalize();
        let (tx, rx) = channel();
        let backup = cfg.clone();
        let mut app = Self {
            cfg,
            backup,
            settings: false,
            info: None,
            info_error: None,
            manifest: None,
            manifest_error: None,
            checked_at: None,
            log: Vec::new(),
            job: None,
            progress: None,
            downloaded: None,
            tx,
            rx,
        };
        app.push(
            Level::Info,
            format!(
                "TACTICAL DEPLOYMENT v{} // {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS
            ),
        );
        app.refresh_info();
        if app.job.is_none() && app.cfg.auto_check && !app.cfg.manifest_url.trim().is_empty() {
            app.start_check();
        }
        app
    }

    // ---------- state ----------

    fn push(&mut self, level: Level, text: String) {
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        self.log.push(Line { level, time, text });
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    fn handle(&mut self, ctx: &egui::Context, msg: Msg) {
        match msg {
            Msg::Log(level, text) => self.push(level, text),
            Msg::Progress(done, total) => self.progress = Some((done, total)),
            Msg::Info(info) => {
                self.info = Some(info);
                self.info_error = None;
            }
            Msg::Manifest(manifest) => {
                self.checked_at = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                let installed = self.info.as_ref().map(|i| i.revision).unwrap_or(0);
                if manifest.is_newer_than(installed) {
                    self.push(
                        Level::Ok,
                        format!("манифест: ревизия {}", manifest.revision()),
                    );
                } else {
                    self.push(
                        Level::Info,
                        format!(
                            "манифест: ревизия {} — обновление не нужно",
                            manifest.revision()
                        ),
                    );
                }
                self.manifest_error = None;
                self.manifest = Some(manifest);
            }
            Msg::Done(job) => {
                self.job = None;
                self.progress = None;
                match job {
                    Job::Info => {
                        if let Some(info) = &self.info {
                            self.push(
                                Level::Ok,
                                format!(
                                    "пакет: ревизия {}, моды {}/{}",
                                    info.revision, info.mods_installed, info.mods_total
                                ),
                            );
                        }
                        if self.cfg.auto_check
                            && self.manifest.is_none()
                            && !self.cfg.manifest_url.trim().is_empty()
                        {
                            self.start_check();
                        }
                    }
                    Job::Check => {}
                    Job::Download => {
                        self.push(Level::Ok, "загрузка завершена".into());
                        self.start_install();
                    }
                    Job::Install => {
                        self.push(Level::Ok, "установка завершена".into());
                        self.refresh_info();
                    }
                    Job::Launch => {
                        if self.cfg.close_on_launch {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            }
            Msg::Failed(job, err) => {
                self.job = None;
                self.progress = None;
                match job {
                    Job::Info => {
                        self.info = None;
                        self.info_error = Some(err.clone());
                    }
                    Job::Check => self.manifest_error = Some(err.clone()),
                    _ => {}
                }
                self.push(Level::Err, format!("{}: {}", job.label(), err));
            }
        }
    }

    fn state(&self) -> (&'static str, Color32) {
        if let Some(job) = self.job {
            return (job.label(), theme::AMBER);
        }
        if let Some(info) = &self.info {
            if let Some(manifest) = &self.manifest {
                if manifest.is_newer_than(info.revision) {
                    return ("ДОСТУПНО ОБНОВЛЕНИЕ", theme::AMBER);
                }
            }
            if info.complete() {
                return ("ГОТОВО К РАЗВЁРТЫВАНИЮ", theme::GREEN);
            }
            return ("ПАКЕТ НЕПОЛНЫЙ", theme::AMBER);
        }
        if self.info_error.is_some() {
            return ("ПАКЕТ НЕ НАЙДЕН", theme::RED);
        }
        ("ОЖИДАНИЕ", theme::DIM)
    }

    fn subtitle(&self) -> String {
        match &self.info {
            Some(info) => {
                let server = if !info.server.is_empty() {
                    info.server.clone()
                } else if !self.cfg.host.trim().is_empty() {
                    self.cfg.connect()
                } else {
                    "адрес не задан".to_string()
                };
                format!(
                    "МОДОВ {}/{}  //  РЕВИЗИЯ {}  //  {}",
                    info.mods_installed, info.mods_total, info.revision, server
                )
            }
            None => "ВЫБЕРИ .PZPACK И ПАПКУ ИГРЫ В НАСТРОЙКАХ [F1]".into(),
        }
    }

    fn connect(&self) -> String {
        if let Some(manifest) = &self.manifest {
            if let Some(connect) = manifest.connect() {
                if !connect.is_empty() {
                    return connect;
                }
            }
        }
        self.cfg.connect()
    }

    fn update_banner(&self) -> Option<(u64, String)> {
        let manifest = self.manifest.as_ref()?;
        let installed = self.info.as_ref().map(|i| i.revision).unwrap_or(0);
        if !manifest.is_newer_than(installed) {
            return None;
        }
        Some((
            manifest.revision(),
            manifest.changelog.clone().unwrap_or_default(),
        ))
    }

    // ---------- actions ----------

    fn refresh_info(&mut self) {
        if self.job.is_some() {
            return;
        }
        let path = PathBuf::from(self.cfg.pack_file.trim());
        if path.as_os_str().is_empty() || !path.is_file() {
            self.info = None;
            self.info_error = Some("файл .pzpack не выбран".into());
            return;
        }
        if self.cfg.manager.trim().is_empty() {
            self.info = None;
            self.info_error = Some("pz-mod-manager не найден".into());
            self.push(
                Level::Err,
                "pz-mod-manager не найден: укажи путь в настройках [F1]".into(),
            );
            return;
        }
        self.job = Some(Job::Info);
        jobs::info(
            self.tx.clone(),
            self.cfg.manager.clone(),
            path,
            self.cfg.env.clone(),
        );
    }

    fn start_check(&mut self) {
        if self.job.is_some() {
            self.push(Level::Warn, "дождись завершения текущей операции".into());
            return;
        }
        let url = self.cfg.manifest_url.trim().to_string();
        if url.is_empty() {
            self.push(
                Level::Err,
                "адрес manifest.json не задан — открой настройки [F1]".into(),
            );
            self.open_settings();
            return;
        }
        self.push(Level::Info, format!("проверка манифеста: {url}"));
        self.manifest_error = None;
        self.job = Some(Job::Check);
        jobs::check(self.tx.clone(), url);
    }

    fn start_download(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(manifest) = self.manifest.clone() else {
            self.push(Level::Err, "манифест ещё не получен".into());
            return;
        };
        let Some(url) = manifest.download_url().map(str::to_string) else {
            self.push(
                Level::Err,
                "в манифесте нет url пакета (pack.url / file.url / url)".into(),
            );
            return;
        };
        let dest = config::data_dir()
            .join("packs")
            .join(manifest.download_name());
        self.downloaded = Some(dest.clone());
        self.progress = Some((0, manifest.declared_size()));
        self.job = Some(Job::Download);
        jobs::download(self.tx.clone(), url, dest, manifest.sha256());
    }

    fn start_install(&mut self) {
        if self.job.is_some() {
            return;
        }
        let path = self.downloaded.clone().filter(|p| p.is_file()).or_else(|| {
            let p = PathBuf::from(self.cfg.pack_file.trim());
            p.is_file().then_some(p)
        });
        let Some(path) = path else {
            self.push(Level::Err, "нет .pzpack для установки".into());
            return;
        };
        if self.cfg.manager.trim().is_empty() {
            self.push(Level::Err, "pz-mod-manager не найден".into());
            return;
        }
        self.job = Some(Job::Install);
        jobs::install(
            self.tx.clone(),
            self.cfg.manager.clone(),
            path,
            self.cfg.env.clone(),
        );
    }

    fn deploy(&mut self) {
        if self.job.is_some() {
            return;
        }
        let dir = PathBuf::from(self.cfg.game_dir.trim());
        if config::game_bin(&dir).is_none() {
            self.push(
                Level::Err,
                format!("ProjectZomboid64 не найден в {}", dir.display()),
            );
            self.open_settings();
            return;
        }
        let connect = self.connect();
        if connect.is_empty() {
            self.push(
                Level::Warn,
                "адрес сервера не задан: клиент запустится без +connect".into(),
            );
        }
        self.job = Some(Job::Launch);
        jobs::launch(
            self.tx.clone(),
            dir,
            PathBuf::from(self.cfg.cachedir.trim()),
            connect,
        );
    }

    fn open_settings(&mut self) {
        self.backup = self.cfg.clone();
        self.settings = true;
    }

    // ---------- paint ----------

    fn shell(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, egui::CornerRadius::ZERO, theme::BG);

        let header = Rect::from_min_size(full.min, egui::vec2(full.width(), 74.0));
        let footer = Rect::from_min_size(
            egui::pos2(full.left(), full.bottom() - 28.0),
            egui::vec2(full.width(), 28.0),
        );
        let body = Rect::from_min_max(
            egui::pos2(full.left(), header.bottom()),
            egui::pos2(full.right(), footer.top()),
        );
        let left_width = 300.0_f32.min(body.width() * 0.34);
        let right_width = 390.0_f32.min(body.width() * 0.42);
        let left = Rect::from_min_size(body.min, egui::vec2(left_width, body.height()));
        let right = Rect::from_min_size(
            egui::pos2(body.right() - right_width, body.top()),
            egui::vec2(right_width, body.height()),
        );
        let center = Rect::from_min_max(
            egui::pos2(left.right(), body.top()),
            egui::pos2(right.left(), body.bottom()),
        );

        {
            let painter = ui.painter();
            painter.rect_filled(header, egui::CornerRadius::ZERO, theme::PANEL);
            painter.rect_filled(left, egui::CornerRadius::ZERO, theme::PANEL);
            painter.rect_filled(right, egui::CornerRadius::ZERO, theme::PANEL);
            painter.hline(
                header.x_range(),
                header.bottom() - 0.5,
                Stroke::new(1.0, theme::LINE),
            );
            painter.hline(
                footer.x_range(),
                footer.top(),
                Stroke::new(1.0, theme::LINE),
            );
            painter.vline(
                left.right() - 0.5,
                body.y_range(),
                Stroke::new(1.0, theme::LINE),
            );
            painter.vline(right.left(), body.y_range(), Stroke::new(1.0, theme::LINE));
        }

        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            self.header(ui)
        });
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(left.shrink2(egui::vec2(16.0, 14.0))),
            |ui| self.left(ui),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(right.shrink2(egui::vec2(14.0, 12.0))),
            |ui| self.log_panel(ui),
        );
        self.center(ui, center);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(footer.shrink2(egui::vec2(16.0, 4.0))),
            |ui| self.footer(ui),
        );

        if self.settings {
            self.settings_overlay(ui, full);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let painter = ui.painter().clone();
        painter.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, theme::LINE),
        );

        let logo = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 20.0, rect.center().y - 21.0),
            egui::vec2(42.0, 42.0),
        );
        painter.rect_stroke(
            logo,
            egui::CornerRadius::ZERO,
            Stroke::new(1.0, theme::AMBER),
            StrokeKind::Inside,
        );
        painter.rect_filled(logo.shrink(3.0), egui::CornerRadius::ZERO, theme::AMBER);
        painter.text(
            logo.center(),
            Align2::CENTER_CENTER,
            "10",
            FontId::monospace(20.0),
            theme::BG,
        );

        painter.text(
            egui::pos2(logo.right() + 16.0, logo.top() + 1.0),
            Align2::LEFT_TOP,
            "TACTICAL DEPLOYMENT",
            FontId::monospace(20.0),
            theme::TEXT,
        );
        painter.text(
            egui::pos2(logo.right() + 16.0, logo.bottom() - 12.0),
            Align2::LEFT_TOP,
            "PROJECT ZOMBOID // 10 YEARS LATER // EFZ EXPEDITION",
            FontId::monospace(9.5),
            theme::DIM,
        );

        let (label, color) = self.state();
        let time = ui.input(|i| i.time);
        let pulse = 0.55 + 0.45 * (((time * 2.6).sin() as f32) * 0.5 + 0.5);
        let dot = egui::pos2(rect.right() - 300.0, rect.center().y + 1.0);
        theme::dot(&painter, dot, 4.0, color.gamma_multiply(pulse));
        painter.text(
            dot + egui::vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(10.0),
            color,
        );

        let now = chrono::Local::now();
        painter.text(
            egui::pos2(rect.right() - 20.0, rect.center().y - 10.0),
            Align2::RIGHT_CENTER,
            now.format("%Y-%m-%d").to_string(),
            FontId::monospace(10.5),
            theme::DIM,
        );
        painter.text(
            egui::pos2(rect.right() - 20.0, rect.center().y + 8.0),
            Align2::RIGHT_CENTER,
            now.format("%H:%M:%S LOCAL").to_string(),
            FontId::monospace(13.0),
            theme::TEXT,
        );
    }

    fn left(&mut self, ui: &mut egui::Ui) {
        section(ui, "СОСТОЯНИЕ СБОРКИ");
        let revision = self
            .info
            .as_ref()
            .map(|i| i.revision.to_string())
            .unwrap_or_else(|| "—".into());
        kv(ui, "РЕВИЗИЯ", &revision, theme::AMBER);
        let mods = self
            .info
            .as_ref()
            .map(|i| format!("{} / {}", i.mods_installed, i.mods_total))
            .unwrap_or_else(|| "—".into());
        kv(ui, "МОДОВ", &mods, theme::TEXT);
        let server = if self
            .info
            .as_ref()
            .map(|i| i.server.is_empty())
            .unwrap_or(true)
        {
            if self.cfg.host.trim().is_empty() {
                "—".to_string()
            } else {
                self.cfg.connect()
            }
        } else {
            self.info
                .as_ref()
                .map(|i| i.server.clone())
                .unwrap_or_default()
        };
        kv(ui, "СЕРВЕР", &server, theme::TEXT);
        kv(ui, "ОКРУЖЕНИЕ", &self.cfg.env, theme::TEXT);
        if let Some(info) = &self.info {
            if !info.profile.is_empty() {
                kv(ui, "ПРОФИЛЬ", &info.profile, theme::TEXT);
            }
        }

        section(ui, "МАНИФЕСТ");
        kv(
            ui,
            "ПРОВЕРЕН",
            self.checked_at.as_deref().unwrap_or("—"),
            theme::TEXT,
        );
        if let Some(err) = &self.manifest_error {
            kv(ui, "СОСТОЯНИЕ", &theme::short(err, 26), theme::RED);
        } else if let Some(manifest) = &self.manifest {
            kv(
                ui,
                "ДОСТУПНА",
                &format!("ревизия {}", manifest.revision()),
                theme::AMBER,
            );
        } else {
            kv(ui, "ДОСТУПНА", "—", theme::DIM);
        }

        section(ui, "ПУТИ");
        kv(
            ui,
            "ИГРА",
            &theme::short(&self.cfg.game_dir, 28),
            theme::DIM,
        );
        kv(
            ui,
            "CACHEDIR",
            &theme::short(&self.cfg.cachedir, 28),
            theme::DIM,
        );
        kv(
            ui,
            "МЕНЕДЖЕР",
            &theme::short(&self.cfg.manager, 28),
            theme::DIM,
        );

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::Button::new(RichText::new("НАСТРОЙКИ  [F1]").size(10.5)),
                )
                .clicked()
            {
                self.open_settings();
            }
            ui.add_space(4.0);
            if ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::Button::new(RichText::new("ПРОВЕРИТЬ ОБНОВЛЕНИЕ").size(10.5)),
                )
                .clicked()
            {
                self.start_check();
            }
        });
    }

    fn center(&mut self, ui: &mut egui::Ui, rect: Rect) {
        theme::backdrop(ui.painter(), rect, ui.ctx().input(|i| i.time));
        let inner = rect.shrink2(egui::vec2(26.0, 18.0));

        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("ОПЕРАЦИЯ // РАЗВЁРТЫВАНИЕ")
                        .size(10.0)
                        .color(theme::AMBER),
                );
                ui.add_space(10.0);
                let (label, color) = self.state();
                ui.label(RichText::new(label).size(28.0).color(color));
                ui.add_space(3.0);
                ui.label(RichText::new(self.subtitle()).size(10.0).color(theme::DIM));
                ui.add_space(18.0);

                let response = self.deploy_button(ui, color);
                if response.clicked() {
                    self.deploy();
                }

                ui.add_space(10.0);
                if self.progress.is_some() || self.job.is_some() {
                    self.progress_bar(ui);
                    ui.add_space(8.0);
                }

                if let Some((revision, changelog)) = self.update_banner() {
                    if self.banner(ui, revision, &changelog) {
                        self.start_download();
                    }
                    ui.add_space(8.0);
                }

                let mut check = false;
                let mut install = false;
                let mut settings = false;
                ui.horizontal(|ui| {
                    check = ui.add(egui::Button::new("ПРОВЕРИТЬ МАНИФЕСТ")).clicked();
                    install = ui.add(egui::Button::new("УСТАНОВИТЬ ПАКЕТ")).clicked();
                    settings = ui.add(egui::Button::new("НАСТРОЙКИ")).clicked();
                });
                if check {
                    self.start_check();
                }
                if install {
                    self.start_install();
                }
                if settings {
                    self.open_settings();
                }
            });
        });
    }

    fn deploy_button(&self, ui: &mut egui::Ui, accent: Color32) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 106.0), Sense::click());
        let busy = self.job.is_some();
        let hover = response.hovered() && !busy;
        let painter = ui.painter();

        let fill = if busy {
            theme::PANEL
        } else if hover {
            Color32::from_rgb(38, 34, 18)
        } else {
            theme::PANEL_2
        };
        painter.rect_filled(rect, egui::CornerRadius::ZERO, fill);
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            Stroke::new(
                1.0,
                if hover {
                    theme::AMBER
                } else if busy {
                    theme::AMBER_DIM
                } else {
                    theme::LINE
                },
            ),
            StrokeKind::Inside,
        );
        theme::brackets(
            painter,
            rect.shrink(6.0),
            16.0,
            Stroke::new(
                1.0,
                if hover {
                    theme::AMBER
                } else {
                    theme::AMBER_DIM
                },
            ),
        );

        if busy {
            let t = ui.input(|i| i.time) as f32;
            let x = rect.left() + (t * 90.0) % rect.width();
            painter.vline(
                x,
                rect.y_range(),
                Stroke::new(1.0, Color32::from_rgba_premultiplied(224, 167, 60, 26)),
            );
        }

        let title = self.job.map(|j| j.label()).unwrap_or("РАЗВЕРНУТЬ");
        painter.text(
            rect.center() - egui::vec2(0.0, 10.0),
            Align2::CENTER_CENTER,
            title,
            FontId::monospace(26.0),
            if busy { theme::AMBER } else { accent },
        );
        painter.text(
            rect.center() + egui::vec2(0.0, 21.0),
            Align2::CENTER_CENTER,
            if busy {
                "ОПЕРАЦИЯ ВЫПОЛНЯЕТСЯ // НЕ ЗАКРЫВАЙ"
            } else {
                "LAUNCH CLIENT // +CONNECT"
            },
            FontId::monospace(9.0),
            theme::DIM,
        );

        if hover {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }

    fn progress_bar(&mut self, ui: &mut egui::Ui) {
        let Some((done, total)) = self.progress else {
            return;
        };
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::PANEL_2);
        let fraction = total
            .filter(|t| *t > 0)
            .map(|t| (done as f32 / t as f32).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let mut fill = rect;
        fill.set_width(rect.width() * fraction);
        painter.rect_filled(fill, egui::CornerRadius::ZERO, theme::AMBER);

        let text = match total {
            Some(total) if total > 0 => format!(
                "{} / {}  //  {:.0}%",
                theme::bytes(done),
                theme::bytes(total),
                fraction * 100.0
            ),
            _ => format!("{}  //  размер неизвестен", theme::bytes(done)),
        };
        ui.add_space(3.0);
        ui.label(RichText::new(text).size(9.5).color(theme::DIM));
    }

    fn banner(&self, ui: &mut egui::Ui, revision: u64, changelog: &str) -> bool {
        let mut go = false;
        egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(Stroke::new(1.0, theme::AMBER_DIM))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("ДОСТУПНА РЕВИЗИЯ {revision}"))
                            .size(11.5)
                            .color(theme::AMBER),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        go = ui
                            .add(
                                egui::Button::new(
                                    RichText::new("СКАЧАТЬ И УСТАНОВИТЬ")
                                        .size(10.0)
                                        .color(theme::BG),
                                )
                                .fill(theme::AMBER),
                            )
                            .clicked();
                    });
                });
                if !changelog.trim().is_empty() {
                    ui.label(RichText::new(changelog).size(9.5).color(theme::DIM));
                }
            });
        go
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("ЖУРНАЛ ОПЕРАЦИЙ")
                    .size(10.0)
                    .color(theme::AMBER),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("ОЧИСТИТЬ").clicked() {
                    self.log.clear();
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.log {
                    let color = match line.level {
                        Level::Ok => theme::GREEN,
                        Level::Warn => theme::AMBER,
                        Level::Err => theme::RED,
                        Level::Info => theme::TEXT,
                    };
                    ui.label(
                        RichText::new(format!("{}  {}", line.time, line.text))
                            .monospace()
                            .size(10.5)
                            .color(color),
                    );
                }
            });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "PZ LAUNCHER v{}  //  {}  //  PACK: PZPACK  //  UPDATES: MANIFEST",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS
                ))
                .size(9.0)
                .color(theme::FAINT),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("F1 — НАСТРОЙКИ   ·   ЛОГ: ПРАВАЯ ПАНЕЛЬ")
                        .size(9.0)
                        .color(theme::FAINT),
                );
            });
        });
    }

    fn settings_overlay(&mut self, ui: &mut egui::Ui, screen: Rect) {
        // Block the HUD underneath: a click outside the panel must not deploy or re-check.
        ui.allocate_rect(screen, Sense::click());

        let rect = Rect::from_center_size(
            screen.center(),
            egui::vec2(700.0, 560.0).min(screen.size() - egui::vec2(40.0, 40.0)),
        );
        {
            let painter = ui.painter();
            painter.rect_filled(
                screen,
                egui::CornerRadius::ZERO,
                Color32::from_black_alpha(205),
            );
            painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::PANEL);
            painter.rect_stroke(
                rect,
                egui::CornerRadius::ZERO,
                Stroke::new(1.0, theme::AMBER_DIM),
                StrokeKind::Inside,
            );
            theme::brackets(
                painter,
                rect.shrink(4.0),
                14.0,
                Stroke::new(1.0, theme::AMBER_DIM),
            );
            painter.text(
                rect.left_top() + egui::vec2(18.0, 14.0),
                Align2::LEFT_TOP,
                "НАСТРОЙКИ // CONNECTION",
                FontId::monospace(13.0),
                theme::AMBER,
            );
            painter.text(
                rect.right_top() + egui::vec2(-18.0, 16.0),
                Align2::RIGHT_TOP,
                "[F1] ЗАКРЫТЬ",
                FontId::monospace(9.5),
                theme::DIM,
            );
        }

        let mut save = false;
        let mut reset = false;
        let mut cancel = false;
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(18.0, 46.0))),
            |ui| {
                ui.label(RichText::new("ПОДКЛЮЧЕНИЕ").size(10.0).color(theme::AMBER));
                ui.add_space(6.0);
                path_field(
                    ui,
                    "PZ-MOD-MANAGER",
                    &mut self.cfg.manager,
                    Pick::FileManager,
                );
                path_field(ui, "ФАЙЛ .PZPACK", &mut self.cfg.pack_file, Pick::Pack);
                path_field(ui, "ПАПКА ИГРЫ", &mut self.cfg.game_dir, Pick::Folder);
                path_field(ui, "CACHEDIR КЛИЕНТА", &mut self.cfg.cachedir, Pick::Folder);

                ui.add_space(10.0);
                ui.label(RichText::new("ОБНОВЛЕНИЯ").size(10.0).color(theme::AMBER));
                ui.add_space(6.0);
                path_field(
                    ui,
                    "MANIFEST.JSON (URL)",
                    &mut self.cfg.manifest_url,
                    Pick::None,
                );

                ui.horizontal(|ui| {
                    ui.label(RichText::new("ОКРУЖЕНИЕ").size(9.5).color(theme::DIM));
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.env).desired_width(170.0));
                    ui.label(RichText::new("СЕРВЕР").size(9.5).color(theme::DIM));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cfg.host)
                            .desired_width(150.0)
                            .hint_text("ip или домен"),
                    );
                    ui.label(RichText::new("ПОРТ").size(9.5).color(theme::DIM));
                    ui.add(egui::DragValue::new(&mut self.cfg.port).range(1..=65535u16));
                });

                ui.add_space(6.0);
                ui.checkbox(
                    &mut self.cfg.auto_check,
                    RichText::new("проверять манифест при запуске").size(10.0),
                );
                ui.checkbox(
                    &mut self.cfg.close_on_launch,
                    RichText::new("закрывать лаунчер после запуска игры").size(10.0),
                );

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    save = ui
                        .add(
                            egui::Button::new(
                                RichText::new("СОХРАНИТЬ").size(10.5).color(theme::BG),
                            )
                            .fill(theme::AMBER),
                        )
                        .clicked();
                    reset = ui.button("СБРОСИТЬ ПУТИ").clicked();
                    cancel = ui.button("ОТМЕНА").clicked();
                });
            },
        );

        if cancel {
            self.cfg = self.backup.clone();
            self.settings = false;
        }
        if reset {
            self.cfg = Config::default();
            self.cfg.normalize();
        }
        if save {
            if let Err(err) = self.cfg.save() {
                self.push(Level::Err, format!("настройки не сохранены: {err}"));
            } else {
                self.push(Level::Ok, "настройки сохранены".into());
                self.settings = false;
                self.refresh_info();
            }
        }
    }
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(&ctx, msg);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            if self.settings {
                self.settings = false;
                self.cfg = self.backup.clone();
            } else {
                self.open_settings();
            }
        }
        ctx.request_repaint_after(if self.job.is_some() {
            Duration::from_millis(70)
        } else {
            Duration::from_millis(350)
        });
        self.shell(ui);
    }
}

enum Pick {
    FileManager,
    Pack,
    Folder,
    None,
}

fn path_field(ui: &mut egui::Ui, label: &str, value: &mut String, pick: Pick) {
    ui.label(RichText::new(label).size(9.5).color(theme::DIM));
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 40.0).max(80.0);
        ui.add(egui::TextEdit::singleline(value).desired_width(width));
        if matches!(pick, Pick::None) {
            return;
        }
        if ui.button("…").clicked() {
            let mut dialog = rfd::FileDialog::new();
            if !value.trim().is_empty() {
                let path = std::path::Path::new(value.trim());
                if let Some(dir) = if path.is_dir() {
                    Some(path)
                } else {
                    path.parent()
                } {
                    if dir.is_dir() {
                        dialog = dialog.set_directory(dir);
                    }
                }
            }
            let chosen = match pick {
                Pick::Pack => dialog.add_filter("pzpack", &["pzpack"]).pick_file(),
                Pick::FileManager => dialog.pick_file(),
                Pick::Folder => dialog.pick_folder(),
                Pick::None => None,
            };
            if let Some(path) = chosen {
                *value = path.display().to_string();
            }
        }
    });
    ui.add_space(4.0);
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(9.5).color(theme::AMBER));
    let rect = ui.available_rect_before_wrap();
    ui.painter()
        .hline(rect.x_range(), rect.top(), Stroke::new(1.0, theme::LINE));
    ui.add_space(6.0);
}

fn kv(ui: &mut egui::Ui, key: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(9.0).color(theme::DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(theme::short(value, 34))
                    .size(10.5)
                    .color(color),
            );
        });
    });
}
