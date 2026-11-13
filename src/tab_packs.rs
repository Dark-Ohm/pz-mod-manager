//! Вкладка «Сборки»: управление сборками модов, импорт/экспорт, сравнение,
//! пакет для игроков и применение сборки на сервер.
use crate::config::Profile;
use crate::i18n::t;
use crate::mods::Mod;
use crate::{App, Bg, export, packs, share, tracking};
use eframe::egui::{self, Color32, RichText};

const ACCENT: Color32 = Color32::from_rgb(120, 170, 90);
const WARN: Color32 = Color32::from_rgb(230, 180, 70);
const ERR: Color32 = Color32::from_rgb(225, 95, 90);

enum PAct {
    Select(usize),
    New,
    Dup,
    Delete,
    CopyMods,
    CopyIds,
    SaveFile,
    SyncPick,
    SyncOff,
    SyncOpen,
    Unlink,
    CheckNow,
    ImportFile,
    ImportNew,
    ImportAdd,
    Zip,
    AskApply,
    Apply,
    DownloadMissing,
}

fn id_list_text(order: &[String]) -> String {
    order.join("\n")
}

impl App {
    pub fn tab_packs(&mut self, ui: &mut egui::Ui) {
        let key = self.track_key();
        let st = self.tracking.server(&key).clone();
        let srv = self.store.settings.server().clone();
        let running = self.server_running();
        let ai = self.store.active.min(self.store.profiles.len() - 1);
        let mut act: Option<PAct> = None;

        let ini_style = export::read_ini_mods(&srv.ini());
        let cur_ids = self.store.profiles[ai].enabled.clone();
        let missing: Vec<String> = cur_ids.iter().filter(|i| !self.installed.contains_key(*i)).cloned().collect();

        egui::Panel::left("packs_list").default_size(250.0).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(t("Сборки"));
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("＋ Новая")).clicked() {
                    act = Some(PAct::New);
                }
                if ui.button(t("⎘ Дублировать")).clicked() {
                    act = Some(PAct::Dup);
                }
                if self.store.profiles.len() > 1 && ui.button(t("🗑 Удалить")).clicked() {
                    act = Some(PAct::Delete);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                for (i, p) in self.store.profiles.iter().enumerate() {
                    let label = format!("{}{}  ({})", if p.stable { "★ " } else { "" }, p.name, p.enabled.len());
                    if ui.selectable_label(i == ai, label).clicked() {
                        act = Some(PAct::Select(i));
                    }
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label(t("Название"));
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.store.profiles[ai].name).desired_width(280.0)).changed();
                    changed |= ui.checkbox(&mut self.store.profiles[ai].stable, t("★ стабильная")).on_hover_text(t("Пометка для себя: эта сборка проверена и играбельна")).changed();
                });
                ui.label(t("Заметки"));
                changed |= ui.add(egui::TextEdit::multiline(&mut self.store.profiles[ai].notes).desired_rows(3).desired_width(f32::INFINITY).hint_text(t("что в сборке, что менялось, что сломано"))).changed();
                ui.horizontal(|ui| {
                    ui.label(tf!("Модов в сборке: {}", cur_ids.len()));
                    if missing.is_empty() {
                        ui.colored_label(ACCENT, t("✔ все найдены на диске"));
                    } else {
                        ui.colored_label(WARN, tf!("не найдено на диске: {}", missing.len()));
                    }
                });
                if !missing.is_empty() {
                    egui::CollapsingHeader::new(t("Какие не найдены")).show(ui, |ui| {
                        for m in &missing {
                            ui.label(RichText::new(m).monospace());
                        }
                        if !self.pk_missing_ws.is_empty() && ui.button(tf!("⬇ Скачать по id воркшопа ({})", self.pk_missing_ws.len())).clicked() {
                            act = Some(PAct::DownloadMissing);
                        }
                    });
                }
                if changed {
                    self.save();
                }

                ui.separator();
                ui.heading(t("Поделиться с другом"));
                ui.weak(t("Файл .pzpack: порядок модов, адрес сервера, ссылки на воркшоп и локальные моды. У друга программа предложит поставить сборку на клиент. С автообновлением файл переписывается сам при каждом изменении сборки; если он лежит в общей папке (Syncthing, Dropbox, Google Drive), программа друга заметит новую ревизию и предложит применить."));
                let (src, sync, rev, applied) = {
                    let p = &self.store.profiles[ai];
                    (p.source_path.clone(), p.sync_path.clone(), p.revision, p.applied_revision)
                };
                if src.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(t("💾 Сохранить .pzpack…")).on_hover_text(t("Разовый файл: отправь его другу любым способом")).clicked() {
                            act = Some(PAct::SaveFile);
                        }
                        if ui.button(t("🔄 Автообновляемый файл…")).on_hover_text(t("Выбери место один раз, дальше файл обновляется сам")).clicked() {
                            act = Some(PAct::SyncPick);
                        }
                    });
                    if !sync.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(ACCENT, t("● автообновление включено"));
                            ui.label(RichText::new(crate::platform::short(std::path::Path::new(&sync))).monospace());
                            ui.weak(tf!("ревизия {}", rev));
                            let senv = self.store.profiles[ai].sync_env.clone();
                            if !senv.is_empty() && senv != srv.key() {
                                ui.colored_label(WARN, tf!("пишется только из окружения «{}»", senv));
                            }
                            if ui.small_button(t("папка")).clicked() {
                                act = Some(PAct::SyncOpen);
                            }
                            if ui.small_button(t("отключить")).clicked() {
                                act = Some(PAct::SyncOff);
                            }
                        });
                    }
                    if srv.public_host.trim().is_empty() {
                        ui.colored_label(WARN, t("Адрес сервера не задан: в файл попадёт только порт. Укажи его в «Управление» → «Сеть»."));
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t("Эта сборка получена из файла:"));
                        ui.label(RichText::new(crate::platform::short(std::path::Path::new(&src))).monospace());
                        ui.weak(tf!("применена ревизия {}", applied));
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(t("Проверить сейчас")).clicked() {
                            act = Some(PAct::CheckNow);
                        }
                        if ui.button(t("Отвязать от файла")).on_hover_text(t("Сборка останется, но обновления из файла больше не предлагаются")).clicked() {
                            act = Some(PAct::Unlink);
                        }
                    });
                }

                ui.separator();
                ui.heading(t("Экспорт"));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t("Копировать Mods=")).on_hover_text(t("Строка для servertest.ini")).clicked() {
                        act = Some(PAct::CopyMods);
                    }
                    if ui.button(t("Копировать id списком")).clicked() {
                        act = Some(PAct::CopyIds);
                    }
                    if ui.add_enabled(self.busy == 0, egui::Button::new(t("📦 Пакет для игроков (zip)…"))).on_hover_text(t("Папки локальных модов + modlist.txt + инструкция. Моды из воркшопа указываются ссылками")).clicked() {
                        act = Some(PAct::Zip);
                    }
                    if self.busy > 0 {
                        ui.spinner();
                    }
                });

                ui.separator();
                ui.heading(t("Импорт"));
                ui.weak(t("Вставь список id модов (по одному в строке, через , или ;), строку Mods=… из ini, ссылки или id воркшопа."));
                ui.add(egui::TextEdit::multiline(&mut self.pk_text).desired_rows(4).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                ui.horizontal_wrapped(|ui| {
                    let has = !self.pk_text.trim().is_empty();
                    if ui.add_enabled(has, egui::Button::new(t("Создать сборку из списка"))).clicked() {
                        act = Some(PAct::ImportNew);
                    }
                    if ui.add_enabled(has, egui::Button::new(t("Добавить в эту сборку"))).clicked() {
                        act = Some(PAct::ImportAdd);
                    }
                    if ui.button(t("⤓ Открыть .pzpack…")).clicked() {
                        act = Some(PAct::ImportFile);
                    }
                });

                ui.separator();
                ui.heading(t("Сравнение"));
                ui.horizontal(|ui| {
                    ui.label(t("С чем сравнить:"));
                    let cur = self.pk_cmp.filter(|c| *c < self.store.profiles.len() && *c != ai);
                    egui::ComboBox::from_id_salt("pk_cmp").selected_text(cur.map(|c| self.store.profiles[c].name.clone()).unwrap_or_else(|| t("— выбери —"))).show_ui(ui, |ui| {
                        for (i, p) in self.store.profiles.iter().enumerate() {
                            if i != ai {
                                ui.selectable_value(&mut self.pk_cmp, Some(i), &p.name);
                            }
                        }
                    });
                });
                if let Some(c) = self.pk_cmp.filter(|c| *c < self.store.profiles.len() && *c != ai) {
                    let d = packs::diff(&self.store.profiles[c].enabled, &cur_ids);
                    ui.weak(tf!("Относительно «{}»: добавлено {}, убрано {}, сдвинуто {}", self.store.profiles[c].name, d.added.len(), d.removed.len(), d.moved.len()));
                    egui::ScrollArea::vertical().id_salt("pk_diff").max_height(180.0).show(ui, |ui| {
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

                ui.separator();
                ui.heading(t("Применить на сервер"));
                ui.weak(tf!("Запишет порядок модов в {} (с копией .bak). Установленные моды не удаляются и не скачиваются.", crate::platform::short(&srv.ini())));
                if running {
                    ui.colored_label(WARN, t("Сервер запущен менеджером — останови его перед применением."));
                }
                if ui.add_enabled(!running, egui::Button::new(t("Применить на сервер…"))).clicked() {
                    act = Some(PAct::AskApply);
                }
                if self.pk_apply_confirm {
                    let (old, _) = ini_style.clone();
                    let d = packs::diff(&old, &cur_ids);
                    egui::Frame::new().fill(Color32::from_rgb(40, 38, 28)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                        ui.label(RichText::new(tf!("Применить «{}» к серверу «{}»?", self.store.profiles[ai].name, srv.name)).strong());
                        ui.label(tf!("В ini сейчас {} модов, будет {}: добавится {}, уберётся {}, сдвинется {}.", old.len(), cur_ids.len(), d.added.len(), d.removed.len(), d.moved.len()));
                        if !missing.is_empty() {
                            ui.colored_label(WARN, tf!("Внимание: {} модов из сборки не найдены на диске — сервер может не запуститься.", missing.len()));
                        }
                        ui.horizontal(|ui| {
                            if ui.button(t("Применить")).clicked() {
                                act = Some(PAct::Apply);
                            }
                            if ui.button(t("Отмена")).clicked() {
                                self.pk_apply_confirm = false;
                            }
                        });
                    });
                }
            });
        });

        // ---- действия ----
        let Some(a) = act else { return };
        let ws_of = |m: &Mod| tracking::workshop_of(m, &st, &self.scan_links);
        let roots = vec![srv.mods_dir(), srv.client_mods_dir(&self.store.settings)];
        match a {
            PAct::Select(i) => self.store.active = i,
            PAct::New => {
                self.store.profiles.push(Profile { name: tf!("Новая сборка {}", self.store.profiles.len() + 1), ..Default::default() });
                self.store.active = self.store.profiles.len() - 1;
                self.save();
            }
            PAct::Dup => {
                let mut p = self.store.profiles[ai].clone();
                p.name = tf!("{} (копия)", p.name);
                p.stable = false;
                self.store.profiles.push(p);
                self.store.active = self.store.profiles.len() - 1;
                self.save();
            }
            PAct::Delete => {
                self.store.profiles.remove(ai);
                self.store.active = 0;
                self.pk_cmp = None;
                self.save();
            }
            PAct::CopyMods => {
                let line = format!("Mods={}", packs::mods_line(&cur_ids, ini_style.1));
                ui.ctx().copy_text(line);
                self.status = t("Строка Mods= скопирована");
            }
            PAct::CopyIds => {
                ui.ctx().copy_text(id_list_text(&cur_ids));
                self.status = t("Список id скопирован");
            }
            PAct::SaveFile => {
                let name = format!("{}.pzpack", self.store.profiles[ai].name);
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("pzpack", &["pzpack"]).save_file() {
                    let info = share::server_info(&srv);
                    if share::bump_if_changed(&mut self.store.profiles[ai], &info) {
                        self.save();
                    }
                    self.write_pack_async(ai, path, true);
                }
            }
            PAct::SyncPick => {
                let name = format!("{}.pzpack", self.store.profiles[ai].name);
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("pzpack", &["pzpack"]).save_file() {
                    self.store.profiles[ai].sync_path = path.to_string_lossy().into_owned();
                    self.store.profiles[ai].sync_env = srv.key();
                    let info = share::server_info(&srv);
                    share::bump_if_changed(&mut self.store.profiles[ai], &info);
                    self.save();
                    self.write_pack_async(ai, path, true);
                }
            }
            PAct::SyncOff => {
                self.store.profiles[ai].sync_path.clear();
                self.save();
            }
            PAct::SyncOpen => {
                if let Some(d) = std::path::Path::new(&self.store.profiles[ai].sync_path).parent().map(|d| d.to_path_buf()) {
                    self.open_folder(&d);
                }
            }
            PAct::Unlink => {
                self.store.profiles[ai].source_path.clear();
                self.save();
            }
            PAct::CheckNow => {
                self.share_skip.clear();
                self.share_seen.clear();
                self.share_polled = None;
                self.status = t("Проверяю файл сборки…");
            }
            PAct::ImportFile => {
                if let Some(path) = rfd::FileDialog::new().add_filter("pzpack", &["pzpack", "json"]).pick_file() {
                    self.open_pack(path);
                }
            }
            PAct::ImportNew | PAct::ImportAdd => {
                let parsed = packs::parse_id_list(&self.pk_text);
                let add_only = matches!(a, PAct::ImportAdd);
                if add_only {
                    for id in &parsed.mods {
                        if !self.store.profiles[ai].enabled.contains(id) {
                            self.store.profiles[ai].enabled.push(id.clone());
                        }
                    }
                } else {
                    let name = tf!("Импорт {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
                    self.store.profiles.push(Profile { name, enabled: parsed.mods.clone(), ..Default::default() });
                    self.store.active = self.store.profiles.len() - 1;
                }
                self.pk_missing_ws = parsed.workshop.clone();
                self.pk_text.clear();
                self.save();
                self.status = tf!("Из списка: модов {}, элементов воркшопа {}", parsed.mods.len(), parsed.workshop.len());
            }
            PAct::DownloadMissing => {
                self.dl_input = self.pk_missing_ws.join("\n");
                self.tab = crate::Tab::Download;
            }
            PAct::Zip => {
                let p = self.store.profiles[ai].clone();
                if let Some(dest) = rfd::FileDialog::new().set_file_name(format!("{}.zip", p.name)).add_filter("zip", &["zip"]).save_file() {
                    self.busy += 1;
                    self.status = t("Собираю пакет…");
                    let (installed, tx, scan, stc) = (self.installed.clone(), self.bg_tx.clone(), self.scan_links.clone(), st.clone());
                    let info = share::server_info(&srv);
                    let en = crate::i18n::get() == crate::i18n::Lang::En;
                    std::thread::spawn(move || {
                        let ws = |m: &Mod| tracking::workshop_of(m, &stc, &scan);
                        let msg = match packs::build_player_zip(&dest, &p, &installed, &roots, &ws, en, info) {
                            Ok(r) => tf!("Пакет готов: {} папок, {} файлов, {}; из воркшопа (ссылками): {}, не найдено: {}", r.folders, r.files, crate::backup::human(r.bytes), r.subscribe.len(), r.missing.len()),
                            Err(e) => tf!("Ошибка сборки пакета: {}", format!("{e:#}")),
                        };
                        let _ = tx.send(Bg::Job(msg));
                    });
                }
            }
            PAct::AskApply => self.pk_apply_confirm = true,
            PAct::Apply => {
                self.pk_apply_confirm = false;
                let (m, w) = export::build(&cur_ids, &self.installed, srv.nosteam, ini_style.1);
                self.status = match export::write_ini(&srv.ini(), &m, &w) {
                    Ok(_) => tf!("Сборка применена: {} модов записано в {}", cur_ids.len(), srv.ini().display()),
                    Err(e) => tf!("Ошибка записи: {}", format!("{e:#}")),
                };
            }
        }
    }
}
