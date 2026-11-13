//! Вкладка «Обновления»: слежение за обновлениями локальных модов.
//! Проверка и установка — только по кнопке; перед заменой старая версия сохраняется (есть откат).
use crate::i18n::t;
use crate::mods::{Mod, Source};
use crate::tracking::{self, Status};
use crate::workshop_api;
use crate::{App, Bg, modupdate, steam};
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;
use std::sync::mpsc::channel;

const ACCENT: Color32 = Color32::from_rgb(120, 170, 90);
const WARN: Color32 = Color32::from_rgb(230, 180, 70);
const ERR: Color32 = Color32::from_rgb(225, 95, 90);

enum UAct {
    Check,
    AskPin,
    Pin,
    PinOne(String),
    Detect,
    Import,
    Select(String),
    Update(String),
    Skip(String),
    Freeze(String),
    Rollback(String),
    Link(String, String),
    Match,
    CancelMatch,
    AcceptProposals,
    CloseProposals,
    Ignore(String),
    UnignoreAll,
}

impl App {
    pub(crate) fn track_key(&self) -> String {
        self.store.settings.server().cache_dir.to_string_lossy().into_owned()
    }

    /// Привязки, найденные при сканировании: один и тот же id мода лежит в воркшоп-папке или в загрузках
    pub fn compute_scan_links(all: &[Mod], cache: &[Mod]) -> std::collections::HashMap<String, String> {
        all.iter().chain(cache.iter()).filter_map(|m| m.workshop_id.clone().map(|w| (m.id.clone(), w))).collect()
    }

    fn start_check(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            self.status = t("Нет модов с привязкой к воркшопу").into();
            return;
        }
        self.checking = true;
        self.status = tf!("Проверяю {} элементов в Steam…", ids.len());
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let r = workshop_api::fetch_details(&ids).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Bg::Remote(r));
        });
    }

    /// Сколько секунд ещё действует пауза после 429 (0 — можно слать запросы)
    pub fn cooldown_left(&self) -> u64 {
        self.steam_cooldown.map_or(0, |t| t.saturating_duration_since(std::time::Instant::now()).as_secs())
    }

    fn start_changelog(&mut self, ws: String) {
        // уже есть результат или идёт загрузка; после ошибки можно повторить
        if matches!(self.changelogs.get(&ws), Some(None) | Some(Some(Ok(_)))) {
            return;
        }
        // 1) свежий кэш на диске — сеть не нужна
        if let Some(c) = workshop_api::cached_changelog(&ws) {
            self.changelogs.insert(ws, Some(Ok(c)));
            return;
        }
        // 2) пауза после 429
        let left = self.cooldown_left();
        if left > 0 {
            self.changelogs.insert(ws, Some(Err(tf!("Steam просил подождать. Повтори через {} мин {} с (ничего не нужно делать, просто нажми ещё раз позже)", left / 60, left % 60))));
            return;
        }
        // 3) не чаще, чем раз в пару секунд
        if self.last_changelog_req.is_some_and(|t| t.elapsed().as_secs() < workshop_api::MIN_GAP_SECS) {
            self.changelogs.insert(ws, Some(Err(t("Слишком часто: подожди пару секунд и нажми ещё раз").into())));
            return;
        }
        self.last_changelog_req = Some(std::time::Instant::now());
        self.changelogs.insert(ws.clone(), None);
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let r = workshop_api::fetch_changelog_cached(&ws).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Bg::Changelog(ws, r));
        });
    }

    fn start_update(&mut self, ws: String, expected: Vec<String>) {
        let srv = self.store.settings.server().clone();
        let client_dir = self.upd_client.then(|| self.store.settings.server().client_mods_dir(&self.store.settings));
        let baseline = self.tracking.server(&self.track_key()).items.get(&ws).and_then(|i| i.baseline);
        self.busy += 1;
        self.upd_log.clear();
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let log = |s: String| {
                let _ = tx.send(Bg::Log(s));
            };
            let res = (|| -> anyhow::Result<(Vec<String>, u64)> {
                use anyhow::Context;
                log(tf!("Проверяю {} в Steam…", ws));
                let r = workshop_api::fetch_details(&[ws.clone()])?.into_iter().next().filter(|r| r.ok).context(t("элемент недоступен в Steam (удалён или скрыт)"))?;
                let item_dir = steam::cache_content().join(&ws);
                if item_dir.exists() {
                    std::fs::remove_dir_all(&item_dir)?; // свежая загрузка, а не старый кэш
                }
                let (stx, srx) = channel();
                let tx2 = tx.clone();
                let fwd = std::thread::spawn(move || {
                    for m in srx {
                        if let steam::Msg::Log(l) = m {
                            let _ = tx2.send(Bg::Log(l));
                        }
                    }
                });
                let ok = steam::run_blocking(&[ws.clone()], &stx);
                drop(stx);
                let _ = fwd.join();
                if !ok? {
                    anyhow::bail!("{}", t("SteamCMD не скачал элемент"));
                }
                // защита от неверной привязки: в скачанном элементе должен быть наш мод
                let found: Vec<String> = crate::mods::scan(&item_dir, crate::mods::Source::Cache).into_iter().map(|m| m.id).collect();
                if !modupdate::item_matches(&found, &expected) {
                    anyhow::bail!("{}", tf!("в скачанном элементе нет мода {} (там: {}). Скорее всего, привязка неверна — исправь её. Ничего не изменено", expected.join(", "), found.join(", ")));
                }
                log(t("Устанавливаю на сервер…").into());
                let mut names = modupdate::install(&item_dir, &ws, "server", &srv.mods_dir(), &srv.cache_dir, baseline)?;
                if let Some(cd) = client_dir {
                    log(t("Устанавливаю на клиент…").into());
                    names.extend(modupdate::install(&item_dir, &ws, "client", &cd, &srv.cache_dir, baseline)?);
                }
                Ok((names, r.time_updated))
            })();
            let _ = tx.send(match res {
                Ok((names, ts)) => Bg::Updated { ws: ws.clone(), ok: true, msg: tf!("Обновлено: {} ({})", ws, names.join(", ")), baseline: Some(ts) },
                Err(e) => Bg::Updated { ws: ws.clone(), ok: false, msg: tf!("Не обновлено {}: {}", ws, format!("{e:#}")), baseline: None },
            });
        });
    }

    pub fn tab_updates(&mut self, ui: &mut egui::Ui) {
        let key = self.track_key();
        let st = self.tracking.server(&key).clone();
        let running = self.server_running();
        let locals: Vec<&Mod> = self.installed.values().filter(|m| matches!(m.source, Source::ServerLocal | Source::ClientLocal)).collect();
        let (groups, unlinked) = tracking::group(&locals, |m| tracking::workshop_of(m, &st, &self.scan_links));
        let n_ignored = st.ignored.len();
        let unlinked: Vec<String> = unlinked.into_iter().filter(|id| !st.ignored.contains(id)).collect();
        let names: BTreeMap<String, String> = locals.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
        let stat = |ws: &str| tracking::status(st.items.get(ws), self.remote.get(ws));

        let mut act: Option<UAct> = None;
        let cd = self.cooldown_left();
        let (mut n_up, mut n_ok, mut n_nb, mut n_fr) = (0, 0, 0, 0);
        for ws in groups.keys() {
            match stat(ws) {
                Status::Update(_) => n_up += 1,
                Status::UpToDate | Status::Skipped(_) => n_ok += 1,
                Status::Frozen => n_fr += 1,
                _ => n_nb += 1,
            }
        }

        // ---- правая панель: изменения ----
        egui::Panel::right("upd_side").default_size(380.0).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(t("Что нового"));
            let Some(ws) = self.upd_sel.clone() else {
                ui.weak(t("Выбери мод и нажми «Что нового»"));
                return;
            };
            let title = self.remote.get(&ws).map(|r| r.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| ws.clone());
            ui.label(RichText::new(title).strong().size(16.0));
            ui.hyperlink_to(t("Открыть в Steam"), workshop_api::page_url(&ws));
            let base = st.items.get(&ws).and_then(|i| i.baseline);
            ui.weak(match base {
                Some(b) => tf!("Твоя версия: {}", tracking::fmt_ts(b)),
                None => t("Твоя версия не зафиксирована — показаны все записи").to_string(),
            });
            ui.separator();
            match self.changelogs.get(&ws) {
                None => {
                    ui.weak("…");
                }
                Some(None) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("загрузка…"));
                    });
                }
                Some(Some(Err(e))) => {
                    ui.colored_label(ERR, e);
                }
                Some(Some(Ok(list))) => {
                    if list.is_empty() {
                        ui.weak(t("У мода нет записей об изменениях"));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        for c in list {
                            let newer = match (c.ts, base) {
                                (Some(t), Some(b)) => t > b,
                                _ => false,
                            };
                            egui::Frame::new().fill(if newer { Color32::from_rgb(45, 42, 28) } else { Color32::from_rgb(28, 31, 28) }).corner_radius(6).inner_margin(8).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&c.when).strong());
                                    if newer {
                                        ui.colored_label(WARN, t("новее твоей версии"));
                                    }
                                });
                                ui.label(if newer { RichText::new(&c.text) } else { RichText::new(&c.text).weak() });
                            });
                        }
                    });
                }
            }
        });

        // ---- основная часть ----
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::CollapsingHeader::new(t("Как это работает (3 шага)")).default_open(self.remote.is_empty() && st.items.is_empty()).show(ui, |ui| {
                ui.label(t("1. «Проверить обновления» — один запрос в Steam на всё, дат хватает на весь пак."));
                ui.label(t("2. «Зафиксировать текущие как стабильные» — твои сегодняшние версии становятся точкой отсчёта."));
                ui.label(t("3. Дальше «есть обновление» значит, что мод в Steam новее твоей версии. «Что нового» покажет изменения, «Обновить» поставит (старая версия сохранится)."));
                ui.weak(t("«Что нового» открывает страницу Steam: не нажимай подряд на десятки модов — Steam ограничивает частоту. Результаты кэшируются на 6 часов."));
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.checking, egui::Button::new(t("⟳ Проверить обновления"))).clicked() {
                    act = Some(UAct::Check);
                }
                if ui.button(t("📌 Зафиксировать текущие как стабильные")).on_hover_text(t("Считать установленные сейчас версии «своими»: обновления будут считаться от этого момента")).clicked() {
                    act = Some(UAct::AskPin);
                }
                if ui.button(t("🔗 Определить привязки")).on_hover_text(t("Из url= в mod.info и из найденных воркшоп-копий")).clicked() {
                    act = Some(UAct::Detect);
                }
                if ui.button(t("⤓ Импорт из JSON…")).on_hover_text(t("Манифест вида [{\"workshop_id\":…, \"mods\":[{\"id\":…}]}]")).clicked() {
                    act = Some(UAct::Import);
                }
                if self.checking {
                    ui.spinner();
                }
                if let Some((i, n)) = self.matching {
                    ui.spinner();
                    ui.label(tf!("подбор по названию: {}/{}", i, n));
                    if ui.button(t("Стоп")).clicked() {
                        act = Some(UAct::CancelMatch);
                    }
                } else {
                    let has_key = !self.store.settings.steam_api_key.trim().is_empty();
                    let r = ui.add_enabled(has_key && !unlinked.is_empty(), egui::Button::new(t("🔎 Подобрать по названию"))).on_hover_text(if has_key { t("Ищет мод в Steam Workshop по названию; ничего не применяется без твоего подтверждения") } else { t("Нужен ключ Steam API: вкладка «Настройки»") });
                    if r.clicked() {
                        act = Some(UAct::Match);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(if n_up > 0 { WARN } else { Color32::GRAY }, tf!("к обновлению: {}", n_up));
                ui.label(tf!("актуальны: {}", n_ok));
                ui.label(tf!("не зафиксированы/не проверены: {}", n_nb));
                ui.label(tf!("заморожены: {}", n_fr));
                ui.label(tf!("без привязки: {}", unlinked.len()));
                if let Some(t) = &self.remote_at {
                    ui.weak(tf!("проверено {}", t));
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.upd_filter).hint_text(t("🔍 поиск")).desired_width(220.0));
                ui.checkbox(&mut self.upd_only, t("только с обновлениями"));
                ui.checkbox(&mut self.upd_client, t("ставить и на клиент")).on_hover_text(t("Кроме папки модов сервера обновить и локальные моды клиента"));
            });
            if cd > 0 {
                ui.colored_label(WARN, tf!("Steam просит не частить с запросами к страницам изменений. Подожди {} мин {} с: даты обновлений и установка работают как обычно.", cd / 60, cd % 60));
            }
            if running {
                ui.colored_label(WARN, t("Сервер запущен менеджером — останови его перед установкой обновлений."));
            }
            if let Some(ws) = self.upd_confirm.clone() {
                egui::Frame::new().fill(Color32::from_rgb(40, 38, 28)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.label(RichText::new(tf!("Обновить {}?", ws)).strong());
                    ui.label(t("Свежая версия скачается из Steam и заменит папки мода. Старая версия сохранится — её можно вернуть кнопкой «Откат»."));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!running && self.busy == 0, egui::Button::new(t("Обновить"))).clicked() {
                            act = Some(UAct::Update(ws.clone()));
                        }
                        if ui.button(t("Отмена")).clicked() {
                            self.upd_confirm = None;
                        }
                    });
                });
            }
            if self.upd_pin_confirm {
                egui::Frame::new().fill(Color32::from_rgb(40, 38, 28)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.label(t("Все установленные сейчас версии будут считаться стабильными, пропущенные обновления сбросятся. Продолжить?"));
                    ui.horizontal(|ui| {
                        if ui.button(t("Да, зафиксировать")).clicked() {
                            act = Some(UAct::Pin);
                        }
                        if ui.button(t("Отмена")).clicked() {
                            self.upd_pin_confirm = false;
                        }
                    });
                });
            }
            if !self.upd_log.is_empty() {
                egui::ScrollArea::vertical().id_salt("updlog").max_height(90.0).stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.upd_log {
                        ui.label(RichText::new(l).monospace().size(11.0).weak());
                    }
                });
            }
            ui.separator();

            if !self.proposals.is_empty() {
                egui::Frame::new().fill(Color32::from_rgb(34, 38, 46)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tf!("Предложения привязок ({})", self.proposals.len())).strong());
                        if ui.button(t("Отметить точные")).clicked() {
                            for p in &mut self.proposals {
                                p.accept = p.cands[p.chosen].score >= 0.999;
                            }
                        }
                        if ui.button(t("Снять все")).clicked() {
                            for p in &mut self.proposals {
                                p.accept = false;
                            }
                        }
                        let n = self.proposals.iter().filter(|p| p.accept).count();
                        if ui.add_enabled(n > 0, egui::Button::new(tf!("✔ Принять отмеченные ({})", n))).clicked() {
                            act = Some(UAct::AcceptProposals);
                        }
                        if ui.button(t("Закрыть")).clicked() {
                            act = Some(UAct::CloseProposals);
                        }
                    });
                    ui.weak(t("Галочки по умолчанию только у точных совпадений названия. Проверь остальные: неверная привязка приведёт к чужому моду (перед установкой менеджер это всё равно проверит)."));
                    egui::ScrollArea::vertical().id_salt("props").max_height(240.0).show(ui, |ui| {
                        for p in &mut self.proposals {
                            ui.horizontal_wrapped(|ui| {
                                ui.checkbox(&mut p.accept, "");
                                ui.label(RichText::new(&p.mod_name).strong());
                                ui.label("→");
                                let cur = p.cands[p.chosen].clone();
                                egui::ComboBox::from_id_salt(format!("prop_{}", p.mod_id)).width(360.0).selected_text(format!("{} · {}%", cur.title, (cur.score * 100.0) as u32)).show_ui(ui, |ui| {
                                    for (i, c) in p.cands.iter().enumerate() {
                                        ui.selectable_value(&mut p.chosen, i, format!("{} · {}% · {}", c.title, (c.score * 100.0) as u32, c.id));
                                    }
                                });
                                let id = p.cands[p.chosen].id.clone();
                                ui.hyperlink_to("🔗", workshop_api::page_url(&id));
                            });
                        }
                    });
                });
                ui.add_space(4.0);
            }

            let q = self.upd_filter.to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                for (ws, ids) in &groups {
                    let status = stat(ws);
                    if self.upd_only && !matches!(status, Status::Update(_)) {
                        continue;
                    }
                    let remote = self.remote.get(ws);
                    let title = remote.map(|r| r.title.clone()).filter(|t| !t.is_empty()).or_else(|| st.items.get(ws).map(|i| i.title.clone()).filter(|t| !t.is_empty())).unwrap_or_else(|| ids.iter().filter_map(|i| names.get(i)).cloned().collect::<Vec<_>>().join(", "));
                    if !q.is_empty() && !title.to_lowercase().contains(&q) && !ws.contains(&q) && !ids.iter().any(|i| i.to_lowercase().contains(&q)) {
                        continue;
                    }
                    let selected = self.upd_sel.as_deref() == Some(ws.as_str());
                    egui::Frame::new().fill(if selected { Color32::from_rgb(40, 52, 38) } else { Color32::from_rgb(30, 33, 30) }).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&title).strong());
                            let (txt, col) = match &status {
                                Status::Update(_) => (t("⬆ есть обновление"), WARN),
                                Status::UpToDate => (t("✔ актуален"), ACCENT),
                                Status::Skipped(_) => (t("пропущено"), Color32::GRAY),
                                Status::NoBaseline => (t("версия не зафиксирована"), Color32::GRAY),
                                Status::Unknown => (t("не проверено"), Color32::GRAY),
                                Status::Frozen => (t("❄ заморожен"), Color32::LIGHT_BLUE),
                                Status::Gone => (t("⛔ удалён в Steam"), ERR),
                                Status::NoLink => (t("без привязки"), Color32::GRAY),
                            };
                            ui.colored_label(col, txt);
                        });
                        let base = st.items.get(ws).and_then(|i| i.baseline).map(tracking::fmt_ts).unwrap_or_else(|| "—".into());
                        let last = remote.filter(|r| r.ok).map(|r| tracking::fmt_ts(r.time_updated)).unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(tf!("{}  ·  твоя версия: {}  ·  в Steam: {}  ·  {}", ws, base, last, ids.join(", "))).weak().size(12.0));
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button(t("Что нового")).clicked() {
                                act = Some(UAct::Select(ws.clone()));
                            }
                            let can_update = matches!(status, Status::Update(_) | Status::Skipped(_));
                            if ui.add_enabled(can_update, egui::Button::new(t("⬇ Обновить…")).small()).clicked() {
                                self.upd_confirm = Some(ws.clone());
                            }
                            if ui.add_enabled(matches!(status, Status::Update(_)), egui::Button::new(t("Пропустить")).small()).clicked() {
                                act = Some(UAct::Skip(ws.clone()));
                            }
                            if ui.add_enabled(remote.is_some_and(|r| r.ok), egui::Button::new(t("📌 Моя версия = эта")).small()).on_hover_text(t("Не скачивая: считать установленную версию равной текущей в Steam")).clicked() {
                                act = Some(UAct::PinOne(ws.clone()));
                            }
                            let frozen = st.items.get(ws).is_some_and(|i| i.frozen);
                            if ui.small_button(if frozen { t("🔥 Разморозить") } else { t("❄ Заморозить") }).clicked() {
                                act = Some(UAct::Freeze(ws.clone()));
                            }
                            if !modupdate::backups_for(&self.store.settings.server().cache_dir, ws).is_empty() && ui.add_enabled(!running, egui::Button::new(t("↶ Откат")).small()).on_hover_text(t("Вернуть версию, что стояла до последнего обновления")).clicked() {
                                act = Some(UAct::Rollback(ws.clone()));
                            }
                        });
                    });
                }

                if !unlinked.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(RichText::new(tf!("Без привязки к воркшопу ({})", unlinked.len())).strong()).show(ui, |ui| {
                        ui.weak(t("Вставь ссылку на страницу мода в Steam Workshop или id — менеджер запомнит. Или нажми «Подобрать по названию»."));
                        if n_ignored > 0 {
                            ui.horizontal(|ui| {
                                ui.weak(tf!("не отслеживаются: {}", n_ignored));
                                if ui.small_button(t("вернуть все")).clicked() {
                                    act = Some(UAct::UnignoreAll);
                                }
                            });
                        }
                        for id in &unlinked {
                            ui.horizontal(|ui| {
                                ui.label(names.get(id).cloned().unwrap_or_else(|| id.clone()));
                                ui.weak(id);
                                let buf = self.link_input.entry(id.clone()).or_default();
                                ui.add(egui::TextEdit::singleline(buf).hint_text(t("ссылка или id")).desired_width(220.0));
                                if ui.small_button(t("Привязать")).clicked() {
                                    act = Some(UAct::Link(id.clone(), buf.clone()));
                                }
                                if ui.small_button(t("не отслеживать")).on_hover_text(t("Свой патч или мод не из воркшопа")).clicked() {
                                    act = Some(UAct::Ignore(id.clone()));
                                }
                            });
                        }
                    });
                }
            });
        });

        // ---- действия ----
        let Some(a) = act else { return };
        match a {
            UAct::Check => {
                let ids: Vec<String> = groups.keys().filter(|w| !st.items.get(*w).is_some_and(|i| i.frozen)).cloned().collect();
                self.start_check(ids);
            }
            UAct::AskPin => {
                if self.remote.is_empty() {
                    self.status = t("Сначала нажми «Проверить обновления» — нужны актуальные даты из Steam").into();
                } else {
                    self.upd_pin_confirm = true;
                }
            }
            UAct::Pin => {
                self.upd_pin_confirm = false;
                let mut n = 0;
                for ws in groups.keys() {
                    if let Some(r) = self.remote.get(ws).filter(|r| r.ok) {
                        let it = self.tracking.server(&key).items.entry(ws.clone()).or_default();
                        it.baseline = Some(r.time_updated);
                        it.skipped_until = None;
                        it.title = r.title.clone();
                        n += 1;
                    }
                }
                self.tracking.save();
                self.status = tf!("Зафиксировано версий: {}", n);
            }
            UAct::PinOne(ws) => {
                if let Some(r) = self.remote.get(&ws).filter(|r| r.ok).cloned() {
                    let it = self.tracking.server(&key).items.entry(ws).or_default();
                    it.baseline = Some(r.time_updated);
                    it.skipped_until = None;
                    it.title = r.title;
                    self.tracking.save();
                }
            }
            UAct::Detect => {
                let mut n = 0;
                let sv = self.tracking.server(&key);
                for id in &unlinked {
                    if let Some(m) = locals.iter().find(|m| &m.id == id) {
                        if let Some(ws) = m.url.as_deref().and_then(tracking::id_from_url).or_else(|| self.scan_links.get(id).cloned()) {
                            sv.links.insert(id.clone(), ws);
                            n += 1;
                        }
                    }
                }
                self.tracking.save();
                self.status = tf!("Привязано модов: {}, осталось без привязки: {}", n, unlinked.len() - n);
            }
            UAct::Import => {
                if let Some(f) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                    let pairs = std::fs::read_to_string(&f).map(|t| tracking::import_manifest(&t)).unwrap_or_default();
                    let sv = self.tracking.server(&key);
                    for (id, ws) in &pairs {
                        sv.links.entry(id.clone()).or_insert_with(|| ws.clone());
                    }
                    self.tracking.save();
                    self.status = tf!("Из файла прочитано связей: {}", pairs.len());
                }
            }
            UAct::Select(ws) => {
                self.upd_sel = Some(ws.clone());
                self.start_changelog(ws);
            }
            UAct::Update(ws) => {
                self.upd_confirm = None;
                let expected = groups.get(&ws).cloned().unwrap_or_default();
                self.start_update(ws, expected);
            }
            UAct::Skip(ws) => {
                if let Some(r) = self.remote.get(&ws) {
                    let t = r.time_updated;
                    self.tracking.server(&key).items.entry(ws).or_default().skipped_until = Some(t);
                    self.tracking.save();
                }
            }
            UAct::Freeze(ws) => {
                let it = self.tracking.server(&key).items.entry(ws).or_default();
                it.frozen = !it.frozen;
                self.tracking.save();
            }
            UAct::Rollback(ws) => {
                let srv = self.store.settings.server().clone();
                let cur = self.tracking.server(&key).items.get(&ws).and_then(|i| i.baseline);
                match modupdate::rollback(&srv.cache_dir, &ws, cur) {
                    Ok(old) => {
                        let it = self.tracking.server(&key).items.entry(ws.clone()).or_default();
                        it.baseline = old;
                        it.skipped_until = None;
                        self.tracking.save();
                        self.status = tf!("Откат выполнен: {}", ws);
                        self.rescan();
                    }
                    Err(e) => self.status = tf!("Откат не удался: {}", format!("{e:#}")),
                }
            }
            UAct::Match => {
                let list: Vec<(String, String)> = unlinked.iter().map(|id| (id.clone(), names.get(id).cloned().unwrap_or_else(|| id.clone()))).collect();
                let apikey = self.store.settings.steam_api_key.trim().to_string();
                self.match_cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                let cancel = self.match_cancel.clone();
                self.matching = Some((0, list.len()));
                self.proposals.clear();
                let tx = self.bg_tx.clone();
                std::thread::spawn(move || {
                    let tx2 = tx.clone();
                    let r = crate::matching::propose(&apikey, &list, &move |i: usize, n: usize| {
                        let _ = tx2.send(Bg::MatchProgress(i, n));
                    }, &|| cancel.load(std::sync::atomic::Ordering::SeqCst))
                    .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(Bg::Matches(r));
                });
            }
            UAct::CancelMatch => self.match_cancel.store(true, std::sync::atomic::Ordering::SeqCst),
            UAct::CloseProposals => self.proposals.clear(),
            UAct::AcceptProposals => {
                let mut n = 0;
                let sv = self.tracking.server(&key);
                for p in self.proposals.iter().filter(|p| p.accept) {
                    sv.links.insert(p.mod_id.clone(), p.cands[p.chosen].id.clone());
                    n += 1;
                }
                self.proposals.retain(|p| !p.accept);
                self.tracking.save();
                self.status = tf!("Привязано модов: {}", n);
            }
            UAct::Ignore(id) => {
                self.tracking.server(&key).ignored.insert(id);
                self.tracking.save();
            }
            UAct::UnignoreAll => {
                self.tracking.server(&key).ignored.clear();
                self.tracking.save();
            }
            UAct::Link(id, input) => match tracking::id_from_url(&input) {
                Some(ws) => {
                    self.tracking.server(&key).links.insert(id.clone(), ws);
                    self.link_input.remove(&id);
                    self.tracking.save();
                }
                None => self.status = t("Не нашёл id воркшопа в тексте — вставь ссылку вида …/filedetails/?id=123 или число").into(),
            },
        }
    }
}
