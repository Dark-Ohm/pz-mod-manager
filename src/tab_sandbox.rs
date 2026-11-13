//! Вкладка «Песочница»: шаблоны серверов и редактор SandboxVars.lua.
use crate::i18n::t;
use crate::sandbox::{self, Kind};
use crate::{App, templates};
use eframe::egui::{self, Color32, RichText};

impl App {
    fn sb_load(&mut self) {
        let path = sandbox::path(self.store.settings.server());
        self.sb_fields = std::fs::read_to_string(&path).map(|t| sandbox::parse(&t)).unwrap_or_default();
        self.sb_edits.clear();
        self.sb_status = if path.exists() { tf!("{} полей · {}", self.sb_fields.len(), path.display()) } else { tf!("Нет файла {}", path.display()) };
        self.sb_for = path;
    }

    pub fn tab_sandbox(&mut self, ui: &mut egui::Ui) {
        let path = sandbox::path(self.store.settings.server());
        if self.sb_for != path {
            self.sb_load();
        }
        let running = self.server_running();

        ui.heading(t("Шаблоны сервера"));
        ui.weak(t("Меняют только уже существующие ключи в ini и SandboxVars. Пароли, порты и имя сервера не трогаются. Перед записью делается копия .bak-<время>."));
        let tpls = templates::all();
        let mut apply: Option<usize> = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in tpls.iter().enumerate() {
                let r = ui.add_enabled(!running, egui::Button::new(&t.name)).on_hover_text(format!("{}\n[{}]", t.description, t.tags.join(", ")));
                if r.clicked() {
                    self.tpl_confirm = Some(i);
                }
            }
        });
        if running {
            ui.colored_label(Color32::from_rgb(230, 180, 70), t("Сервер запущен — останови его, чтобы применять шаблоны и сохранять песочницу."));
        }
        if let Some(i) = self.tpl_confirm {
            let tpl = &tpls[i];
            egui::Frame::new().fill(Color32::from_rgb(40, 38, 28)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.label(RichText::new(tf!("Применить «{}» к серверу «{}»?", tpl.name, self.store.settings.server().name)).strong());
                ui.label(&tpl.description);
                ui.horizontal(|ui| {
                    if ui.button(t("Применить")).clicked() {
                        apply = Some(i);
                    }
                    if ui.button(t("Отмена")).clicked() {
                        self.tpl_confirm = None;
                    }
                });
            });
        }
        if let Some(i) = apply {
            self.tpl_confirm = None;
            let srv = self.store.settings.server().clone();
            self.status = match templates::apply(&srv, running, &tpls[i]) {
                Ok(r) => tf!("Шаблон «{}»: ini {} ключей, песочница {} из {}", tpls[i].name, r.ini_applied.len(), r.sandbox_applied, r.sandbox_total),
                Err(e) => tf!("Шаблон не применён: {}", e),
            };
            self.sb_load();
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.heading(t("Песочница"));
            ui.add(egui::TextEdit::singleline(&mut self.sb_filter).hint_text(t("🔍 поиск по ключу")).desired_width(200.0));
            if ui.button(t("⟳ Перечитать")).clicked() {
                self.sb_load();
            }
            let n = self.sb_edits.len();
            if ui.add_enabled(n > 0 && !running, egui::Button::new(tf!("💾 Сохранить ({})", n))).clicked() {
                let changes: Vec<(String, String, String)> = self.sb_edits.iter().map(|((c, k), v)| (c.clone(), k.clone(), v.clone())).collect();
                self.status = match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let (new, done) = sandbox::update(&text, &changes);
                        match crate::atomic::write(&path, new.as_bytes()) {
                            Ok(_) => tf!("SandboxVars сохранён: изменено {} из {}", done, n),
                            Err(e) => tf!("Ошибка записи: {}", format!("{e:#}")),
                        }
                    }
                    Err(e) => tf!("Ошибка чтения: {}", e),
                };
                self.sb_load();
            }
        });
        ui.weak(&self.sb_status);

        let q = self.sb_filter.to_lowercase();
        let mut cats: Vec<String> = vec![];
        for f in &self.sb_fields {
            if !cats.contains(&f.category) {
                cats.push(f.category.clone());
            }
        }
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for cat in &cats {
                let fields: Vec<&sandbox::Field> = self.sb_fields.iter().filter(|f| &f.category == cat && (q.is_empty() || f.key.to_lowercase().contains(&q))).collect();
                if fields.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(RichText::new(format!("{} ({})", t(cat), fields.len())).strong()).default_open(cat == sandbox::ROOT || !q.is_empty()).show(ui, |ui| {
                    egui::Grid::new(format!("g_{cat}")).num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                        for f in fields {
                            let id = (f.category.clone(), f.key.clone());
                            let changed = self.sb_edits.contains_key(&id);
                            ui.label(if changed { RichText::new(&f.key).color(Color32::from_rgb(230, 180, 70)) } else { RichText::new(&f.key) });
                            let mut cur = self.sb_edits.get(&id).cloned().unwrap_or_else(|| f.value.clone());
                            let before = cur.clone();
                            match f.kind {
                                Kind::Bool => {
                                    let mut b = cur == "true";
                                    ui.checkbox(&mut b, "");
                                    cur = b.to_string();
                                }
                                _ => {
                                    ui.add(egui::TextEdit::singleline(&mut cur).desired_width(220.0));
                                }
                            }
                            if cur != before {
                                if cur == f.value {
                                    self.sb_edits.remove(&id);
                                } else {
                                    self.sb_edits.insert(id, cur);
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            }
        });
    }
}
