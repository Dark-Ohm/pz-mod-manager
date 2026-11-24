//! Вкладка «Проблемы»: всё, что требует внимания, в одном месте — ошибки и предупреждения
//! по модам, расхождения ini и клиента с сервером, конфликт параметров запуска Steam.
//! Рядом — журнал программы и логи сервера и клиента с фильтром по уровню.
use crate::i18n::t;
use crate::mods::{self, Severity};
use crate::{ACCENT, App, DIM, ERR, Tab, WARN, card, packs, platform, runtime, sync};
use eframe::egui::{self, Color32, RichText};

/// Куда вести по клику: где искать причину
#[derive(Clone)]
pub enum Go {
    /// открыть мод в списке
    Mod(String),
    /// окно «Клиент и сервер»
    Sync,
    Tab(Tab),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Mods,
    Sync,
    Ini,
    Launch,
}

impl Area {
    const ALL: [Area; 4] = [Area::Mods, Area::Sync, Area::Ini, Area::Launch];

    fn title(self) -> String {
        match self {
            Area::Mods => t("Моды сборки"),
            Area::Sync => t("Клиент и сервер"),
            Area::Ini => t("ini сервера"),
            Area::Launch => t("Запуск игры"),
        }
    }
}

#[derive(Clone)]
pub struct Problem {
    pub sev: Severity,
    pub area: Area,
    /// о чём речь: мод или раздел
    pub subject: String,
    pub msg: String,
    pub go: Option<Go>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Problems,
    Journal,
    Server,
    Client,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    All,
    WarnUp,
    ErrorsOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Уровень каждой строки лога PZ: `ERROR: …`, `WARN : …`, исключения и их стек
pub fn log_levels(lines: &[String]) -> Vec<LogLevel> {
    let mut prev = LogLevel::Info;
    lines
        .iter()
        .map(|l| {
            let s = l.trim_start();
            let cont = l.starts_with([' ', '\t']) && (s.starts_with("at ") || s.starts_with("..."));
            let lv = if cont {
                prev
            } else if s.starts_with("ERROR") || s.starts_with("SEVERE") || s.starts_with("FATAL") || s.starts_with("Caused by") || s.contains("Exception") {
                LogLevel::Error
            } else if s.starts_with("WARN") {
                LogLevel::Warn
            } else {
                LogLevel::Info
            };
            prev = lv;
            lv
        })
        .collect()
}

/// Уровень записи журнала программы (это обычные строки статуса)
fn journal_level(s: &str) -> LogLevel {
    let l = s.to_lowercase();
    if ["ошибк", "error", "не удалось", "failed", "✘"].iter().any(|k| l.contains(k)) {
        LogLevel::Error
    } else if ["предупрежд", "warning", "перезапусти", "restart"].iter().any(|k| l.contains(k)) {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

pub fn counts(problems: &[Problem]) -> (usize, usize) {
    let e = problems.iter().filter(|p| p.sev == Severity::Error).count();
    (e, problems.len() - e)
}

/// Строки не ниже уровня `min`, в которых есть `query` (без учёта регистра)
fn filter_lines<'a>(lines: &'a [String], levels: &[LogLevel], min: LogLevel, query: &str) -> Vec<(&'a str, LogLevel)> {
    let q = query.to_lowercase();
    lines.iter().zip(levels).filter(|(l, lv)| **lv >= min && (q.is_empty() || l.to_lowercase().contains(&q))).map(|(l, lv)| (l.as_str(), *lv)).collect()
}

fn list_text(title: String, v: &[String]) -> String {
    let shown: Vec<&str> = v.iter().take(8).map(String::as_str).collect();
    let more = if v.len() > 8 { format!(" … +{}", v.len() - 8) } else { String::new() };
    format!("{title} ({}): {}{more}", v.len(), shown.join(", "))
}

impl App {
    /// Всё, что сейчас не в порядке: моды сборки, ini, клиент и сервер, параметры запуска Steam
    pub fn collect_problems(&self) -> Vec<Problem> {
        let mut out = vec![];
        let order = &self.store.profile().enabled;
        let issues = mods::check(order, &self.installed, &self.dups);
        for id in order {
            let Some(list) = issues.get(id) else { continue };
            let name = self.installed.get(id).map(|m| m.name.as_str()).unwrap_or(id);
            let subject = if name == id { id.clone() } else { format!("{name} ({id})") };
            for i in list {
                out.push(Problem { sev: i.sev, area: Area::Mods, subject: subject.clone(), msg: i.msg.clone(), go: Some(Go::Mod(id.clone())) });
            }
        }
        // ошибки выше предупреждений, порядок сборки внутри уровня сохраняется
        out.sort_by_key(|p| p.sev == Severity::Warn);

        let (ini_exists, ini_mods, _) = &self.ini_state;
        if *ini_exists {
            let d = packs::diff(ini_mods, order);
            if !(d.added.is_empty() && d.removed.is_empty() && d.moved.is_empty()) {
                let file = self.store.settings.server().ini().file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                let msg = tf!("ini отличается от сборки: добавить {}, убрать {}, переставить {}", d.added.len(), d.removed.len(), d.moved.len());
                out.push(Problem { sev: Severity::Warn, area: Area::Ini, subject: file, msg, go: Some(Go::Tab(Tab::Mods)) });
            }
        }

        if let Some(rep) = &self.sync_rep {
            if rep.ini_exists && !rep.in_sync() {
                let mut add = |msg: String| out.push(Problem { sev: Severity::Warn, area: Area::Sync, subject: String::new(), msg, go: Some(Go::Sync) });
                if !rep.client_file_exists {
                    add(tf!("Нет файла списка клиента: {}", platform::short(&rep.client_file)));
                }
                if !rep.missing_on_client.is_empty() {
                    add(list_text(t("Нет в списке клиента"), &rep.missing_on_client));
                }
                if !rep.extra_on_client.is_empty() {
                    add(list_text(t("Лишние в списке клиента"), &rep.extra_on_client));
                }
                if let Some((n, a, b)) = &rep.first_order_diff {
                    add(tf!("Порядок расходится с позиции {}: сервер {}, клиент {}", n, a, b));
                }
                if !rep.folders_only_server.is_empty() {
                    add(list_text(t("Папки только на сервере"), &rep.folders_only_server));
                }
                if !rep.folders_only_client.is_empty() {
                    add(list_text(t("Папки только у клиента"), &rep.folders_only_client));
                }
            }
        }

        if let Some(v) = &self.steam_conflict {
            out.push(Problem {
                sev: Severity::Warn,
                area: Area::Launch,
                subject: "Steam".into(),
                msg: tf!("В параметрах запуска игры в Steam задан -cachedir ({}): он перебивает выбранное окружение", v),
                go: None,
            });
        }
        out
    }

    /// Каждое новое сообщение строки статуса попадает в журнал программы
    pub fn record_status(&mut self) {
        if self.status.is_empty() || self.status == self.journal_last {
            return;
        }
        self.journal_last = self.status.clone();
        self.journal.push((chrono::Local::now().format("%H:%M:%S").to_string(), self.status.clone()));
        if self.journal.len() > 500 {
            self.journal.drain(..100);
        }
    }

    pub fn go_problem(&mut self, go: Go) {
        match go {
            Go::Mod(id) => {
                self.selected = Some(id);
                self.tab = Tab::Mods;
            }
            Go::Tab(tab) => self.tab = tab,
            Go::Sync => {
                self.tab = Tab::Server;
                // пока клиент запущен, игра перепишет список при выходе: окно открываем только на остановленном
                let client_busy = self.cli_status.as_ref().is_some_and(|c| c.running);
                if let (Some(rep), false) = (self.sync_rep.clone(), client_busy) {
                    self.sync_items = sync::items(&rep);
                    self.sync_list = !rep.missing_on_client.is_empty() || !rep.extra_on_client.is_empty() || rep.first_order_diff.is_some() || !rep.client_file_exists;
                    self.sync_open = true;
                }
            }
        }
    }

    pub fn tab_problems(&mut self, ui: &mut egui::Ui) {
        let (errs, warns) = self.pb_counts;
        ui.horizontal(|ui| {
            ui.heading(t("Проблемы и логи"));
            ui.add_space(8.0);
            for (v, name) in [(View::Problems, t("Проблемы")), (View::Journal, t("Журнал программы")), (View::Server, t("Лог сервера")), (View::Client, t("Лог клиента"))] {
                if ui.selectable_value(&mut self.pb_view, v, name).clicked() {
                    self.pb_polled = None;
                }
            }
        });
        ui.add_space(2.0);
        match self.pb_view {
            View::Problems => self.problems_view(ui, errs, warns),
            View::Journal => self.log_view(ui, None),
            View::Server => self.log_view(ui, self.srv_status.as_ref().map(|s| s.log.clone())),
            View::Client => self.log_view(ui, self.cli_status.as_ref().map(|s| s.log.clone())),
        }
    }

    fn problems_view(&mut self, ui: &mut egui::Ui, errs: usize, warns: usize) {
        let mut go: Option<Go> = None;
        let mut auto_order = false;
        let mut recheck = false;
        ui.horizontal(|ui| {
            if errs > 0 {
                ui.colored_label(ERR, tf!("⛔ ошибок: {}", errs));
            }
            if warns > 0 {
                ui.colored_label(WARN, tf!("⚠ предупреждений: {}", warns));
            }
            if errs == 0 && warns == 0 {
                ui.colored_label(ACCENT, t("✔ проблем не найдено"));
            }
            ui.add_space(8.0);
            ui.add(egui::TextEdit::singleline(&mut self.pb_filter).hint_text(t("🔍 поиск")).desired_width(200.0));
            if ui.button(t("⟳ Перепроверить")).on_hover_text(t("Заново просканировать моды и сверить клиент с сервером")).clicked() {
                recheck = true;
            }
        });
        ui.add_space(4.0);

        let q = self.pb_filter.to_lowercase();
        let problems: Vec<Problem> = self.problems.iter().filter(|p| q.is_empty() || p.subject.to_lowercase().contains(&q) || p.msg.to_lowercase().contains(&q)).cloned().collect();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if self.problems.is_empty() {
                card(ui, |ui| {
                    ui.label(RichText::new(t("Всё в порядке")).strong());
                    ui.weak(t("Здесь собираются ошибки и предупреждения по модам сборки, расхождения ini и клиента с сервером и конфликты запуска. Сейчас их нет."));
                });
            }
            for area in Area::ALL {
                let items: Vec<&Problem> = problems.iter().filter(|p| p.area == area).collect();
                if items.is_empty() {
                    continue;
                }
                let total = self.problems.iter().filter(|p| p.area == area).count();
                let count = if items.len() == total { total.to_string() } else { format!("{}/{total}", items.len()) };
                egui::CollapsingHeader::new(RichText::new(format!("{} — {count}", area.title())).strong().size(15.0)).id_salt(("pb_area", area as u8)).default_open(true).show(ui, |ui| {
                    if area == Area::Mods {
                        ui.horizontal(|ui| {
                            if ui.button(t("⇅ Авто-порядок")).on_hover_text(t("Зависимости (require=) встанут выше зависящих от них модов")).clicked() {
                                auto_order = true;
                            }
                            ui.weak(tf!("сборка «{}»", self.store.profile().name));
                        });
                    }
                    for p in items {
                        ui.horizontal_wrapped(|ui| {
                            let (c, ic) = if p.sev == Severity::Error { (ERR, "⛔") } else { (WARN, "⚠") };
                            ui.colored_label(c, ic);
                            let sense = if p.go.is_some() { egui::Sense::click() } else { egui::Sense::hover() };
                            let mut hit = false;
                            // имя мода (или файла) жирным; клик по строке ведёт к причине
                            if !p.subject.is_empty() {
                                hit |= ui.add(egui::Label::new(RichText::new(&p.subject).strong()).sense(sense)).on_hover_text(t("Открыть")).clicked();
                            }
                            hit |= ui.add(egui::Label::new(RichText::new(&p.msg).color(Color32::from_gray(200))).sense(sense)).clicked();
                            if hit {
                                go = p.go.clone();
                            }
                        });
                    }
                    ui.add_space(6.0);
                });
            }
        });
        if recheck {
            self.rescan();
            self.sync_rep = Some(sync::check(&self.store.settings, self.store.settings.server()));
            self.steam_conflict = runtime::steam_launch_conflict();
            self.refresh_ini();
        }
        if auto_order {
            self.auto_order();
        }
        if let Some(g) = go {
            self.go_problem(g);
        }
    }

    /// Журнал программы (`path == None`) или лог игры из файла
    fn log_view(&mut self, ui: &mut egui::Ui, path: Option<std::path::PathBuf>) {
        let is_journal = self.pb_view == View::Journal;
        if !is_journal && self.pb_polled.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) {
            self.pb_log = path.as_ref().map(|p| runtime::tail_lines(p, 2000)).unwrap_or_default();
            self.pb_polled = Some(std::time::Instant::now());
        }
        let lines: Vec<String> = if is_journal { self.journal.iter().map(|(ts, m)| format!("{ts}  {m}")).collect() } else { self.pb_log.clone() };
        let levels: Vec<LogLevel> = if is_journal { self.journal.iter().map(|(_, m)| journal_level(m)).collect() } else { log_levels(&lines) };

        let (mut copy, mut clear) = (false, false);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("pb_level")
                .selected_text(match self.pb_level {
                    Level::All => t("Все строки"),
                    Level::WarnUp => t("Предупреждения и ошибки"),
                    Level::ErrorsOnly => t("Только ошибки"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.pb_level, Level::All, t("Все строки"));
                    ui.selectable_value(&mut self.pb_level, Level::WarnUp, t("Предупреждения и ошибки"));
                    ui.selectable_value(&mut self.pb_level, Level::ErrorsOnly, t("Только ошибки"));
                });
            ui.add(egui::TextEdit::singleline(&mut self.pb_filter).hint_text(t("🔍 поиск")).desired_width(200.0));
            copy = ui.button(t("Копировать")).on_hover_text(t("Скопировать показанные строки")).clicked();
            if is_journal {
                clear = ui.button(t("Очистить")).clicked();
            } else if let Some(p) = &path {
                if ui.small_button("📂").on_hover_text(t("Открыть папку с логом")).clicked() {
                    if let Some(dir) = p.parent() {
                        let _ = open::that(dir);
                    }
                }
                ui.label(RichText::new(platform::short(p)).color(DIM).size(12.0));
            }
        });
        if clear {
            self.journal.clear();
            return;
        }
        let min = match self.pb_level {
            Level::All => LogLevel::Info,
            Level::WarnUp => LogLevel::Warn,
            Level::ErrorsOnly => LogLevel::Error,
        };
        let shown = filter_lines(&lines, &levels, min, &self.pb_filter);
        if copy {
            ui.ctx().copy_text(shown.iter().map(|(l, _)| *l).collect::<Vec<_>>().join("\n"));
        }
        ui.weak(tf!("Строк: {} из {}", shown.len(), lines.len()));
        if lines.is_empty() {
            ui.weak(if is_journal { t("Журнал пуст: сообщения программы появятся здесь") } else { t("Лога пока нет: игра ещё не запускалась или файл пуст") });
            return;
        }
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        egui::Frame::new().fill(Color32::from_rgb(18, 20, 18)).corner_radius(6).inner_margin(6).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(("pb_log", self.pb_view as u8)).auto_shrink([false; 2]).stick_to_bottom(true).show_rows(ui, row_h, shown.len(), |ui, range| {
                for (l, lv) in &shown[range] {
                    let c = match lv {
                        LogLevel::Error => ERR,
                        LogLevel::Warn => WARN,
                        LogLevel::Info => Color32::from_gray(185),
                    };
                    ui.add(egui::Label::new(RichText::new(*l).monospace().color(c)).truncate()).on_hover_text(*l);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lv(lines: &[&str]) -> Vec<LogLevel> {
        log_levels(&lines.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn pz_log_levels_and_stack_continuations() {
        use LogLevel::*;
        let r = lv(&[
            "LOG  : General     f:0, t:1> mods loaded",
            "WARN : Lua         f:0, t:2> something odd",
            "ERROR: General     f:0, t:3> boom",
            "\tat zombie.Foo.bar(Foo.java:10)",
            "\t... 5 more",
            "LOG  : General     f:0, t:4> next",
            "java.lang.NullPointerException: x",
            "Caused by: java.io.IOException",
            "    at a.b.C.d(C.java:1)",
        ]);
        assert_eq!(r, [Info, Warn, Error, Error, Error, Info, Error, Error, Error]);
        // «at …» без отступа — не продолжение стека
        assert_eq!(lv(&["at the end of the day"]), [Info]);
    }

    #[test]
    fn journal_levels() {
        assert_eq!(journal_level("Ошибка записи: нет доступа"), LogLevel::Error);
        assert_eq!(journal_level("Mod removed: x"), LogLevel::Info);
        assert_eq!(journal_level("ini обновлён. Сервер запущен — перезапусти его"), LogLevel::Warn);
    }

    #[test]
    fn level_order_filters_upwards() {
        assert!(LogLevel::Error > LogLevel::Warn && LogLevel::Warn > LogLevel::Info);
        let lines: Vec<String> = ["LOG  : a", "WARN : b", "ERROR: c", "\tat x.Y(Y.java)", "LOG  : d C"].map(String::from).to_vec();
        let lv = log_levels(&lines);
        let pick = |min, q: &str| filter_lines(&lines, &lv, min, q).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(pick(LogLevel::Info, "").len(), 5);
        assert_eq!(pick(LogLevel::Warn, ""), ["WARN : b", "ERROR: c", "\tat x.Y(Y.java)"]);
        assert_eq!(pick(LogLevel::Error, ""), ["ERROR: c", "\tat x.Y(Y.java)"]);
        assert_eq!(pick(LogLevel::Info, "c"), ["ERROR: c", "LOG  : d C"]);
    }

    /// Вкладка рисуется во всех видах без паники: логи читаются из файла, фильтр и журнал работают
    #[test]
    fn problems_tab_renders_every_view() {
        let dir = std::env::temp_dir().join(format!("pzmm-pb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("console.txt");
        std::fs::write(&log, "LOG  : General > hi\nWARN : Lua > careful\nERROR: General > boom\n\tat zombie.A.b(A.java)\n").unwrap();
        let mut a = App::new();
        a.status = "Ошибка записи: нет доступа".into();
        a.record_status();
        a.record_status(); // одно и то же сообщение второй раз в журнал не попадает
        assert_eq!(a.journal.len(), 1);
        a.problems = a.collect_problems();
        a.pb_counts = counts(&a.problems);
        let ctx = egui::Context::default();
        for (view, level) in [(View::Problems, Level::All), (View::Journal, Level::All), (View::Server, Level::All), (View::Client, Level::WarnUp), (View::Server, Level::ErrorsOnly)] {
            a.pb_view = view;
            a.pb_level = level;
            a.pb_polled = None;
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 600.0)));
            let _ = ctx.run_ui(raw, |ui| match view {
                View::Server | View::Client => a.log_view(ui, Some(log.clone())),
                _ => a.tab_problems(ui),
            });
        }
        assert_eq!(a.pb_log.len(), 4, "лог прочитан из файла");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
