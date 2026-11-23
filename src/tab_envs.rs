//! Вкладка «Окружения»: список, создание (с папками), изменение, удаление.
//! Кнопки запуска строятся из списка окружений сами (вкладка «Управление», меню запуска).
use crate::config::{self, ServerCfg};
use crate::i18n::t;
use crate::{App, path_row, platform, sync};
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct NewEnv {
    pub label: String,
    pub id: String,
    pub base: PathBuf,
    /// скопировать настройки (игра, режим, аргументы) с этого окружения
    pub copy_from: Option<usize>,
    pub nosteam: bool,
}

impl Default for NewEnv {
    fn default() -> Self {
        Self { label: String::new(), id: String::new(), base: platform::home().join("Projects/project-zomboid"), copy_from: None, nosteam: true }
    }
}

/// id из названия: латиница, цифры и «-»
fn slug(s: &str) -> String {
    let v: String = s.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    v.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

fn dirs_of(base: &Path, id: &str) -> (PathBuf, PathBuf) {
    (base.join(format!("pz-{id}-server")), base.join(format!("pz-{id}-client")))
}

/// Создаёт папки окружения: сервер (Server, mods, Saves) и клиент (mods + пустой default.txt)
pub fn create_folders(c: &ServerCfg, s: &config::Settings) -> std::io::Result<()> {
    for d in ["Server", "mods", "Saves"] {
        std::fs::create_dir_all(c.cache_dir.join(d))?;
    }
    let cm = c.client_mods_dir(s);
    std::fs::create_dir_all(&cm)?;
    let f = cm.join("default.txt");
    if !f.exists() {
        std::fs::write(f, sync::client_list_text(&[]))?;
    }
    Ok(())
}

impl App {
    pub fn tab_envs(&mut self, ui: &mut egui::Ui) {
        let n = self.store.settings.servers.len();
        self.env_sel = self.env_sel.min(n.saturating_sub(1));
        let mut changed = false;
        let mut make_active: Option<usize> = None;
        let mut delete = false;

        ui.horizontal(|ui| {
            ui.heading(t("Окружения"));
            if ui.button(t("＋ Новое окружение")).clicked() {
                self.env_new = NewEnv::default();
                self.env_new_open = true;
            }
            if ui.button(t("⧉ Дублировать")).on_hover_text(t("Новое окружение с настройками выбранного (папки создаются заново)")).clicked() {
                self.env_new = NewEnv { copy_from: Some(self.env_sel), nosteam: self.store.settings.servers[self.env_sel].nosteam, ..NewEnv::default() };
                self.env_new_open = true;
            }
        });
        ui.weak(t("Окружение — сервер и клиент со своими папками данных, модами и миром. Кнопки запуска появляются на вкладке «Управление» сами."));
        ui.add_space(4.0);

        let running_any = self.env_status.get(self.env_sel).is_some_and(|x| x.0.running || x.1.running);
        egui::Panel::left("envlist").resizable(false).exact_size(210.0).show_inside(ui, |ui| {
            let active = self.store.settings.active_server;
            for (i, c) in self.store.settings.servers.iter().enumerate() {
                let running = self.env_status.get(i).is_some_and(|x| x.0.running);
                let txt = format!("{}{}  ·  {}", if running { "● " } else { "" }, c.label, c.key());
                let r = ui.selectable_label(i == self.env_sel, if i == active { RichText::new(txt).strong() } else { RichText::new(txt) });
                if r.clicked() {
                    self.env_sel = i;
                }
                r.context_menu(|ui| {
                    if ui.add_enabled(i != active, egui::Button::new(t("★ Сделать активным"))).clicked() {
                        make_active = Some(i);
                        ui.close();
                    }
                    if ui.button(t("⧉ Дублировать")).clicked() {
                        self.env_new = NewEnv { copy_from: Some(i), nosteam: c.nosteam, ..NewEnv::default() };
                        self.env_new_open = true;
                        ui.close();
                    }
                });
            }
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                let sel = self.env_sel;
                let active = self.store.settings.active_server == sel;
                ui.horizontal(|ui| {
                    if active {
                        ui.colored_label(crate::ACCENT, t("★ активное окружение"));
                    } else if ui.button(t("★ Сделать активным")).clicked() {
                        make_active = Some(sel);
                    }
                    if n > 1 && ui.add_enabled(!running_any, egui::Button::new(t("🗑 Удалить…"))).on_hover_text(t("Убирает только запись из настроек, файлы остаются")).clicked() {
                        delete = true;
                    }
                    if running_any {
                        ui.weak(t("Остановите сервер и клиент этого окружения"));
                    }
                });
                ui.add_space(6.0);
                let s_snapshot = self.store.settings.clone();
                let c = &mut self.store.settings.servers[sel];
                ui.label(t("Название"));
                changed |= ui.add(egui::TextEdit::singleline(&mut c.label).desired_width(300.0)).changed();
                ui.label(t("Id окружения (для CLI и агентов)"));
                let hint = c.key();
                changed |= ui.add(egui::TextEdit::singleline(&mut c.id).desired_width(200.0).hint_text(hint)).changed();
                changed |= path_row(ui, t("Папка данных сервера (-cachedir, внутри Server/ и mods/)"), &mut c.cache_dir, true);
                changed |= path_row(ui, t("Папка данных клиента этого окружения (-cachedir клиента; пусто — общие моды клиента)"), &mut c.client_cache_dir, true);
                changed |= path_row(ui, t("Папка игры / сервера (start-server.sh, StartServer64_nosteam.bat)"), &mut c.game_dir, true);
                if !c.nosteam {
                    changed |= path_row(ui, t("Воркшоп сервера (steamapps/workshop/content/108600)"), &mut c.workshop_dir, true);
                }
                ui.horizontal(|ui| {
                    ui.label(t("Имя сервера:"));
                    let names = c.list_names();
                    if names.is_empty() {
                        changed |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).changed();
                        ui.weak(t("ini появится после первого запуска"));
                    } else {
                        egui::ComboBox::from_id_salt("envsrvname").selected_text(c.name.clone()).show_ui(ui, |ui| {
                            for nm in &names {
                                changed |= ui.selectable_value(&mut c.name, nm.clone(), nm).changed();
                            }
                        });
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(t("Память Java (МБ):"));
                    changed |= ui.add(egui::DragValue::new(&mut c.xmx_mb).range(1024..=65536).speed(256)).changed();
                    ui.label(t("Бэкапов хранить:"));
                    changed |= ui.add(egui::DragValue::new(&mut c.backup_keep).range(1..=200)).changed();
                });
                changed |= ui.checkbox(&mut c.nosteam, t("No-steam (-nosteam): без Steam, моды только из локальной папки")).changed();
                ui.label(t("Доп. аргументы запуска"));
                changed |= ui.add(egui::TextEdit::singleline(&mut c.extra_args).desired_width(f32::INFINITY).hint_text(t("например: -adminpassword …"))).changed();
                ui.add_space(6.0);

                // что уже есть на диске
                let rows = [
                    (t("Сервер: Server/ (ini)"), c.cache_dir.join("Server")),
                    (t("Сервер: mods/"), c.mods_dir()),
                    (t("Клиент: mods/"), c.client_mods_dir(&s_snapshot)),
                    (t("Клиент: default.txt"), c.client_mods_dir(&s_snapshot).join("default.txt")),
                ];
                let missing = rows.iter().filter(|(_, p)| !p.exists()).count();
                for (label, p) in &rows {
                    ui.horizontal(|ui| {
                        if p.exists() {
                            ui.colored_label(crate::ACCENT, "✔");
                        } else {
                            ui.colored_label(crate::WARN, "✘");
                        }
                        ui.label(label.as_str());
                        ui.weak(platform::short(p));
                    });
                }
                if missing > 0 && ui.button(t("📁 Создать недостающие папки")).clicked() {
                    let r = create_folders(c, &s_snapshot);
                    self.status = match r {
                        Ok(_) => t("Папки созданы").to_string(),
                        Err(e) => tf!("Ошибка: {}", e),
                    };
                }
                ui.horizontal(|ui| {
                    if ui.button(t("📂 Папка сервера")).clicked() {
                        let _ = open::that(&c.cache_dir);
                    }
                    if ui.button(t("📂 Папка клиента")).clicked() {
                        let _ = open::that(c.client_cache_dir(&s_snapshot));
                    }
                });
                if let Err(e) = config::check_dirs_unique(&s_snapshot) {
                    ui.colored_label(crate::ERR, e);
                }
            });
        });

        if changed {
            self.save();
            self.rescan();
            self.rt_polled = None;
            self.dash_at = None;
        }
        if let Some(i) = make_active {
            self.switch_env(i);
        }
        if delete {
            self.env_del_confirm = true;
        }
        self.env_new_window(ui.ctx());
        self.env_delete_window(ui.ctx());
    }

    fn env_new_window(&mut self, ctx: &egui::Context) {
        if !self.env_new_open {
            return;
        }
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        let labels: Vec<String> = self.store.settings.servers.iter().map(|c| c.label.clone()).collect();
        egui::Window::new(t("Новое окружение")).open(&mut open).collapsible(false).default_width(520.0).show(ctx, |ui| {
            let e = &mut self.env_new;
            ui.label(t("Название"));
            if ui.add(egui::TextEdit::singleline(&mut e.label).desired_width(f32::INFINITY).hint_text(t("например: Тест карты"))).changed() {
                e.id = slug(&e.label);
            }
            ui.label(t("Id (латиница, для CLI и агентов)"));
            ui.add(egui::TextEdit::singleline(&mut e.id).desired_width(200.0));
            path_row(ui, t("Где создать папки"), &mut e.base, true);
            ui.horizontal(|ui| {
                ui.label(t("Взять настройки с:"));
                let cur = e.copy_from.and_then(|i| labels.get(i).cloned()).unwrap_or_else(|| t("ничего (по умолчанию)").to_string());
                egui::ComboBox::from_id_salt("envcopy").selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.copy_from, None, t("ничего (по умолчанию)"));
                    for (i, l) in labels.iter().enumerate() {
                        ui.selectable_value(&mut e.copy_from, Some(i), l);
                    }
                });
            });
            ui.checkbox(&mut e.nosteam, t("No-steam (-nosteam)"));
            let id = slug(&e.id);
            let (sd, cd) = dirs_of(&e.base, &id);
            ui.add_space(4.0);
            ui.weak(t("Будут созданы:"));
            ui.label(RichText::new(format!("{}  (Server, mods, Saves)", platform::short(&sd))).monospace().size(12.0));
            ui.label(RichText::new(format!("{}  (mods, default.txt)", platform::short(&cd))).monospace().size(12.0));
            let taken = self.store.settings.servers.iter().any(|c| c.key() == id) || sd.exists() && std::fs::read_dir(&sd).is_ok_and(|mut r| r.next().is_some());
            if taken {
                ui.colored_label(crate::WARN, t("Такой id уже занят или папка не пуста"));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!id.is_empty() && !e.label.trim().is_empty() && !taken, egui::Button::new(t("Создать"))).clicked() {
                    create = true;
                }
                if ui.button(t("Отмена")).clicked() {
                    cancel = true;
                }
            });
        });
        if create {
            let e = self.env_new.clone();
            let id = slug(&e.id);
            let (sd, cd) = dirs_of(&e.base, &id);
            let mut c = match e.copy_from.and_then(|i| self.store.settings.servers.get(i)) {
                Some(src) => ServerCfg { markers: vec![], map_image: PathBuf::new(), ..src.clone() },
                None => ServerCfg::default(),
            };
            c.id = id.clone();
            c.label = e.label.trim().to_string();
            c.cache_dir = sd;
            c.client_cache_dir = cd;
            c.nosteam = e.nosteam;
            if e.copy_from.is_none() {
                c.name = format!("{id}test");
            }
            let mut s2 = self.store.settings.clone();
            s2.servers.push(c.clone());
            match config::check_dirs_unique(&s2).map_err(anyhow::Error::msg).and_then(|_| create_folders(&c, &s2).map_err(Into::into)) {
                Ok(_) => {
                    self.store.settings.servers.push(c);
                    self.env_sel = self.store.settings.servers.len() - 1;
                    self.save();
                    self.status = tf!("Окружение «{}» создано. Первый запуск сервера — на вкладке «Управление» (нужен пароль админа).", e.label.trim());
                    self.env_new_open = false;
                }
                Err(err) => self.status = tf!("Ошибка: {}", format!("{err:#}")),
            }
        }
        if cancel || !open {
            self.env_new_open = false;
        }
    }

    fn env_delete_window(&mut self, ctx: &egui::Context) {
        if !self.env_del_confirm {
            return;
        }
        let i = self.env_sel;
        let Some(c) = self.store.settings.servers.get(i).cloned() else {
            self.env_del_confirm = false;
            return;
        };
        let mut go = false;
        let mut cancel = false;
        egui::Window::new(t("Удалить окружение")).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(RichText::new(tf!("Убрать «{}» из менеджера?", c.label)).strong());
            ui.weak(t("Удаляется только запись в настройках. Папки с миром и модами остаются на диске:"));
            ui.label(RichText::new(platform::short(&c.cache_dir)).monospace().size(12.0));
            ui.label(RichText::new(platform::short(&c.client_cache_dir(&self.store.settings))).monospace().size(12.0));
            ui.horizontal(|ui| {
                if ui.button(t("🗑 Убрать")).clicked() {
                    go = true;
                }
                if ui.button(t("Отмена")).clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            let s = &mut self.store.settings;
            s.servers.remove(i);
            if s.active_server >= s.servers.len() || s.active_server == i {
                s.active_server = 0;
            } else if s.active_server > i {
                s.active_server -= 1;
            }
            self.env_sel = 0;
            self.save();
            self.rescan();
            self.rt_polled = None;
            self.dash_at = None;
            self.status = tf!("Окружение «{}» убрано из менеджера, файлы не тронуты", c.label);
        }
        if go || cancel {
            self.env_del_confirm = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_dirs() {
        assert_eq!(slug("  Тест Карты 2 "), "2");
        assert_eq!(slug("My Env_1"), "my-env-1");
        let (s, c) = dirs_of(Path::new("/x"), "lab");
        assert_eq!(s, Path::new("/x/pz-lab-server"));
        assert_eq!(c, Path::new("/x/pz-lab-client"));
    }

    #[test]
    fn creates_folders() {
        let d = std::env::temp_dir().join(format!("pzmm-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (sd, cd) = dirs_of(&d, "t");
        let mut c = ServerCfg::default();
        c.cache_dir = sd.clone();
        c.client_cache_dir = cd.clone();
        create_folders(&c, &config::Settings::default()).unwrap();
        assert!(sd.join("Server").is_dir() && sd.join("mods").is_dir() && sd.join("Saves").is_dir());
        assert!(cd.join("mods/default.txt").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }
}
