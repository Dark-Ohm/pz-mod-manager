//! Общий файл сборки в интерфейсе: окно «Установить сборку» у получателя и фоновое слежение
//! (автор: переписывает .pzpack при изменениях; получатель: замечает новую ревизию).
use crate::config::Profile;
use crate::i18n::t;
use crate::mods::Mod;
use crate::{App, Bg, packs, share, steam, tracking};
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(120, 170, 90);
const WARN: Color32 = Color32::from_rgb(230, 180, 70);
const ERR: Color32 = Color32::from_rgb(225, 95, 90);

/// Сборка, которую предлагается поставить или обновить
pub struct Incoming {
    pub path: PathBuf,
    pub file: packs::PackFile,
    /// индекс сборки у получателя, если эту сборку он уже получал
    pub existing: Option<usize>,
    /// элементы воркшопа, которых нет на диске: будут скачаны через SteamCMD
    pub missing_ws: Vec<String>,
    pub diff: packs::Diff,
}

impl App {
    /// Открыть файл сборки: найти уже полученную сборку или предложить установить новую
    pub fn open_pack(&mut self, path: PathBuf) {
        let res = share::check_ext(&path).and_then(|_| share::read_pack(&path));
        match res {
            Ok(f) => {
                let existing = (!f.pack_id.is_empty()).then(|| self.store.profiles.iter().position(|p| p.pack_id == f.pack_id)).flatten();
                self.pk_incoming = Some(self.make_incoming(path, f, existing));
                self.tab = crate::Tab::Packs;
            }
            Err(e) => self.status = tf!("Не удалось открыть сборку: {}", format!("{e:#}")),
        }
    }

    fn make_incoming(&self, path: PathBuf, file: packs::PackFile, existing: Option<usize>) -> Incoming {
        let mut missing_ws: Vec<String> = vec![];
        for m in &file.mods {
            if let Some(w) = &m.workshop_id {
                if !self.installed.contains_key(&m.id) && !missing_ws.contains(w) {
                    missing_ws.push(w.clone());
                }
            }
        }
        let old = existing.map(|i| self.store.profiles[i].enabled.clone()).unwrap_or_default();
        let diff = share::describe(&old, &file);
        Incoming { path, file, existing, missing_ws, diff }
    }

    /// Раз в пару секунд: автор переписывает файл при изменениях, получатель проверяет новую ревизию
    pub fn poll_share(&mut self) {
        if self.share_polled.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.share_polled = Some(Instant::now());
        let srv = self.store.settings.server().clone();
        let info = share::server_info(&srv);
        let env = srv.key();
        // автор. Файл собирается из модов и адреса своего окружения, поэтому пишем только когда оно активно:
        // иначе при переключении stable/lab в файл попал бы чужой порт и чужие локальные моды
        let mut dirty = false;
        let mut write_idx = None;
        for i in 0..self.store.profiles.len() {
            let p = &mut self.store.profiles[i];
            if p.sync_path.is_empty() || !p.source_path.is_empty() || (!p.sync_env.is_empty() && p.sync_env != env) {
                continue;
            }
            dirty |= share::bump_if_changed(p, &info);
            let (id, rev, path) = (p.pack_id.clone(), p.revision, PathBuf::from(&p.sync_path));
            // после запуска программы: что уже лежит в файле
            if !self.share_written.contains_key(&id) {
                if let Some((fid, frev, _)) = share::peek(&path) {
                    if fid == id {
                        self.share_written.insert(id.clone(), frev);
                    }
                }
            }
            if write_idx.is_none() && !self.share_writing && self.share_written.get(&id) != Some(&rev) {
                write_idx = Some((i, path));
            }
        }
        if dirty {
            self.save();
        }
        if let Some((i, path)) = write_idx {
            self.write_pack_async(i, path, false);
        }
        // получатель: файл читаем, только когда он изменился
        if self.pk_incoming.is_none() {
            for i in 0..self.store.profiles.len() {
                let p = &self.store.profiles[i];
                if p.source_path.is_empty() || p.pack_id.is_empty() {
                    continue;
                }
                let path = PathBuf::from(&p.source_path);
                let Some(mt) = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok()) else { continue };
                if self.share_seen.get(&p.source_path) == Some(&mt) {
                    continue;
                }
                // недописанный файл (общая папка ещё синхронизирует) не запоминаем: проверим снова
                let Ok(f) = share::read_pack(&path) else { continue };
                self.share_seen.insert(p.source_path.clone(), mt);
                if f.pack_id == p.pack_id && f.revision > p.applied_revision && self.share_skip.get(&f.pack_id) != Some(&f.revision) {
                    self.pk_incoming = Some(self.make_incoming(path, f, Some(i)));
                    break;
                }
            }
        }
    }

    /// Записать .pzpack в фоне. `notify` — сообщить в статусе, даже если всё хорошо
    pub fn write_pack_async(&mut self, idx: usize, dest: PathBuf, notify: bool) {
        let key = self.track_key();
        let st = self.tracking.server(&key).clone();
        let srv = self.store.settings.server().clone();
        let p: Profile = self.store.profiles[idx].clone();
        let roots = vec![srv.mods_dir(), srv.client_mods_dir(&self.store.settings)];
        let (installed, scan, tx) = (self.installed.clone(), self.scan_links.clone(), self.bg_tx.clone());
        let info = share::server_info(&srv);
        self.share_writing = true;
        self.share_written.insert(p.pack_id.clone(), p.revision);
        std::thread::spawn(move || {
            let ws = |m: &Mod| tracking::workshop_of(m, &st, &scan);
            let (msg, failed) = match share::write_pack(&dest, &p, &installed, &roots, &ws, info) {
                Ok(r) => (tf!("Файл сборки записан: {} (ревизия {}, локальных папок {})", crate::platform::short(&dest), p.revision, r.folders), false),
                Err(e) => (tf!("Ошибка записи сборки: {}", format!("{e:#}")), true),
            };
            let _ = tx.send(Bg::Shared((notify || failed).then_some(msg)));
        });
    }

    /// Окно подтверждения: установить сборку или применить обновление
    pub fn share_window(&mut self, ctx: &egui::Context) {
        let Some(inc) = &self.pk_incoming else { return };
        let srv = self.store.settings.server().clone();
        let upd = inc.existing.is_some();
        let (mut install, mut cancel) = (false, false);
        let f = &inc.file;
        egui::Window::new(if upd { t("Обновление сборки") } else { t("Установить сборку") })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(&f.name).heading());
                ui.weak(tf!("ревизия {} · модов: {} · {}", f.revision, f.mods.len(), f.created));
                if let Some(s) = &f.server {
                    if !s.host.is_empty() || !s.port.is_empty() {
                        let addr = if s.port.is_empty() { s.host.clone() } else { format!("{}:{}", s.host, s.port) };
                        ui.horizontal(|ui| {
                            ui.label(t("Сервер:"));
                            ui.label(RichText::new(&addr).monospace().strong());
                            if ui.small_button(t("копировать")).clicked() {
                                ui.ctx().copy_text(addr.clone());
                            }
                        });
                    }
                }
                if !f.notes.trim().is_empty() {
                    ui.label(RichText::new(&f.notes).weak());
                }
                ui.separator();
                if upd {
                    let d = &inc.diff;
                    ui.label(tf!("Изменилось: добавлено {}, убрано {}, сдвинуто {}", d.added.len(), d.removed.len(), d.moved.len()));
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        for x in &d.added {
                            ui.colored_label(ACCENT, format!("+ {x}"));
                        }
                        for x in &d.removed {
                            ui.colored_label(ERR, format!("− {x}"));
                        }
                        for x in &d.moved {
                            ui.colored_label(WARN, format!("↕ {x}"));
                        }
                    });
                }
                let local = f.mods.iter().filter(|m| m.workshop_id.is_none() && !self.installed.contains_key(&m.id)).count();
                ui.label(tf!("Будет скачано из воркшопа: {} · локальных модов из файла: {}", inc.missing_ws.len(), local));
                ui.weak(tf!("Моды ставятся на клиент окружения «{}»: {}", srv.label, crate::platform::short(&srv.client_mods_dir(&self.store.settings))));
                ui.weak(t("Порядок модов в твоей сборке станет таким же, как в файле."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if upd { t("Применить обновление") } else { t("Установить на клиент") };
                    if ui.add_enabled(self.busy == 0, egui::Button::new(RichText::new(label).strong())).clicked() {
                        install = true;
                    }
                    if ui.button(t("Позже")).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            if let Some(inc) = self.pk_incoming.take() {
                self.share_skip.insert(inc.file.pack_id.clone(), inc.file.revision);
            }
        }
        if install {
            self.install_incoming();
        }
    }

    fn install_incoming(&mut self) {
        let Some(inc) = self.pk_incoming.take() else { return };
        let f = inc.file;
        let key = self.track_key();
        {
            let sv = self.tracking.server(&key);
            for m in &f.mods {
                if let Some(w) = &m.workshop_id {
                    sv.links.entry(m.id.clone()).or_insert_with(|| w.clone());
                }
            }
        }
        self.tracking.save();
        let ids: Vec<String> = f.mods.iter().map(|m| m.id.clone()).collect();
        let idx = match inc.existing {
            Some(i) => i,
            None => {
                self.store.profiles.push(Profile { name: f.name.clone(), ..Default::default() });
                self.store.profiles.len() - 1
            }
        };
        let p = &mut self.store.profiles[idx];
        p.name = f.name.clone();
        p.enabled = ids;
        p.notes = f.notes.clone();
        if !f.pack_id.is_empty() {
            p.pack_id = f.pack_id.clone();
            p.source_path = inc.path.to_string_lossy().into_owned();
            p.applied_revision = f.revision;
        }
        self.store.active = idx;
        self.save();

        let (settings, srv, tx) = (self.store.settings.clone(), self.store.settings.server().clone(), self.bg_tx.clone());
        let (path, missing) = (inc.path, inc.missing_ws);
        self.busy += 1;
        self.status = t("Устанавливаю сборку на клиент…");
        std::thread::spawn(move || {
            let _ = tx.send(Bg::PackInstalled(install_job(&path, &settings, &srv, &missing)));
        });
    }
}

/// Фоновая установка: локальные моды из файла, затем скачивание и копирование воркшоп-модов
fn install_job(path: &Path, settings: &crate::config::Settings, srv: &crate::config::ServerCfg, missing_ws: &[String]) -> String {
    let dest = srv.client_mods_dir(settings);
    let mut notes: Vec<String> = vec![];
    match share::extract_local(path, &dest) {
        Ok(n) if n > 0 => notes.push(tf!("локальных папок из файла: {}", n)),
        Ok(_) => {}
        Err(e) => notes.push(tf!("ошибка распаковки: {}", format!("{e:#}"))),
    }
    let mut failed: Vec<String> = vec![];
    if !missing_ws.is_empty() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ok = steam::run_blocking(missing_ws, &tx).unwrap_or(false);
        drop(tx);
        let tail: Vec<String> = rx.try_iter().filter_map(|m| if let steam::Msg::Log(l) = m { Some(l) } else { None }).collect();
        if !ok {
            if let Some(l) = tail.last() {
                notes.push(l.clone());
            }
        }
        for w in missing_ws {
            if steam::install_into(w, settings, srv, true, false).is_err() {
                failed.push(w.clone());
            }
        }
        notes.push(tf!("из воркшопа установлено: {} из {}", missing_ws.len() - failed.len(), missing_ws.len()));
    }
    if !failed.is_empty() {
        notes.push(tf!("не удалось: {}", failed.join(", ")));
    }
    if notes.is_empty() {
        notes.push(t("всё уже было на месте").into());
    }
    tf!("Сборка установлена: {}", notes.join("; "))
}
