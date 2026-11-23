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
            ui.add(egui::TextEdit::singleline(&mut self.sb_filter).hint_text(t("🔍 поиск по ключу или описанию")).desired_width(200.0));
            ui.checkbox(&mut self.sb_only_changed, t("только изменённые"));
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
        let only = self.sb_only_changed;
        let mut cats: Vec<String> = vec![];
        for f in &self.sb_fields {
            if !cats.contains(&f.category) {
                cats.push(f.category.clone());
            }
        }
        let mut reset_cat: Option<String> = None;
        let fields_all = self.sb_fields.clone();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for cat in &cats {
                let fields: Vec<&sandbox::Field> = fields_all
                    .iter()
                    .filter(|f| &f.category == cat)
                    .filter(|f| q.is_empty() || f.key.to_lowercase().contains(&q) || f.desc.to_lowercase().contains(&q))
                    .filter(|f| !only || self.sb_edits.contains_key(&(f.category.clone(), f.key.clone())))
                    .collect();
                if fields.is_empty() {
                    continue;
                }
                let changed_n = fields.iter().filter(|f| self.sb_edits.contains_key(&(f.category.clone(), f.key.clone()))).count();
                let title = if changed_n > 0 { format!("{} ({}) · ✎ {}", t(cat), fields.len(), changed_n) } else { format!("{} ({})", t(cat), fields.len()) };
                let hdr = egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(cat).default_open(cat == sandbox::ROOT || !q.is_empty() || only).show(ui, |ui| {
                    for f in fields {
                        self.sb_row(ui, f);
                        ui.add_space(2.0);
                    }
                });
                hdr.header_response.context_menu(|ui| {
                    if ui.button(t("↺ Сбросить изменения в категории")).clicked() {
                        reset_cat = Some(cat.clone());
                        ui.close();
                    }
                });
            }
        });
        if let Some(c) = reset_cat {
            self.sb_edits.retain(|(cat, _), _| *cat != c);
        }
    }

    /// Одна настройка: подпись, подходящий регулятор, сброс; правый клик — меню
    fn sb_row(&mut self, ui: &mut egui::Ui, f: &sandbox::Field) {
        let id = (f.category.clone(), f.key.clone());
        let changed = self.sb_edits.contains_key(&id);
        let mut cur = self.sb_edits.get(&id).cloned().unwrap_or_else(|| f.value.clone());
        let before = cur.clone();
        let warn = Color32::from_rgb(230, 180, 70);
        let resp = egui::Frame::new()
            .fill(if changed { Color32::from_rgb(44, 40, 28) } else { Color32::from_rgb(30, 33, 30) })
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let lab = ui.add(egui::Label::new(if changed { RichText::new(&f.key).strong().color(warn) } else { RichText::new(&f.key).strong() }).sense(egui::Sense::click()));
                    if !f.desc.is_empty() {
                        lab.on_hover_text(&f.desc);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let reset = ui.add_enabled(changed, egui::Button::new("↺").small()).on_hover_text(t("Вернуть как в файле"));
                        if reset.clicked() {
                            cur = f.value.clone();
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| sb_control(ui, f, &mut cur));
                    });
                });
                if !f.desc.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&f.desc).small().weak()).wrap());
                }
            })
            .response;
        resp.context_menu(|ui| {
            if ui.add_enabled(changed, egui::Button::new(t("↺ Вернуть как в файле"))).clicked() {
                cur = f.value.clone();
                ui.close();
            }
            if let Some(d) = &f.default {
                let label = f.options.iter().find(|(n, _)| n == d).map(|(_, l)| format!("{d} — {l}")).unwrap_or_else(|| d.clone());
                if ui.button(tf!("⟲ По умолчанию: {}", label)).clicked() {
                    cur = d.clone();
                    ui.close();
                }
            }
            if f.kind == Kind::Bool && ui.button(t("⇄ Переключить")).clicked() {
                cur = (cur != "true").to_string();
                ui.close();
            }
            ui.separator();
            if ui.button(t("📋 Копировать ключ")).clicked() {
                ui.ctx().copy_text(f.key.clone());
                ui.close();
            }
            if ui.button(t("📋 Копировать значение")).clicked() {
                ui.ctx().copy_text(cur.clone());
                ui.close();
            }
            if !f.desc.is_empty() && ui.button(t("📋 Копировать описание")).clicked() {
                ui.ctx().copy_text(f.desc.clone());
                ui.close();
            }
        });
        if cur != before {
            if cur == f.value {
                self.sb_edits.remove(&id);
            } else {
                self.sb_edits.insert(id, cur);
            }
        }
    }
}

/// Регулятор под тип настройки: переключатель, список вариантов, ползунок или поле
fn sb_control(ui: &mut egui::Ui, f: &sandbox::Field, cur: &mut String) {
    match f.kind {
        Kind::Bool => {
            let mut b = cur == "true";
            if toggle(ui, &mut b).changed() {
                *cur = b.to_string();
            }
            ui.label(if b { t("включено") } else { t("выключено") });
        }
        _ if !f.options.is_empty() => {
            let shown = f.options.iter().find(|(n, _)| n == cur).map(|(n, l)| format!("{n} — {l}")).unwrap_or_else(|| cur.clone());
            egui::ComboBox::from_id_salt((&f.category, &f.key)).width(260.0).selected_text(shown).show_ui(ui, |ui| {
                for (n, l) in &f.options {
                    ui.selectable_value(cur, n.clone(), format!("{n} — {l}"));
                }
            });
        }
        Kind::Number => {
            let is_float = cur.contains('.') || f.min.is_some_and(|m| m.fract() != 0.0) || f.max.is_some_and(|m| m.fract() != 0.0);
            let mut v: f64 = cur.parse().unwrap_or(0.0);
            let (lo, hi) = match (f.min, f.max) {
                (Some(a), Some(b)) if b > a => (a.min(v), b.max(v)),
                _ => (v.min(0.0), (v.abs() * 4.0).max(10.0)),
            };
            let wide = hi - lo > 1.0e6;
            let r = if wide {
                ui.add(egui::DragValue::new(&mut v).speed(((hi - lo) / 1000.0).max(1.0)))
            } else {
                ui.add_sized([260.0, 18.0], egui::Slider::new(&mut v, lo..=hi).clamping(egui::SliderClamping::Never))
            };
            if r.changed() {
                *cur = if is_float { format_float(v) } else { format!("{}", v.round() as i64) };
            }
            if let Some(d) = &f.default {
                ui.weak(tf!("по умолч. {}", d));
            }
        }
        Kind::Text => {
            ui.add(egui::TextEdit::singleline(cur).desired_width(260.0));
        }
    }
}

fn format_float(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0');
    if s.ends_with('.') { format!("{s}0") } else { s.to_string() }
}

/// Переключатель «рычаг»
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(38.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = if *on { Color32::from_rgb(70, 140, 80) } else { Color32::from_rgb(70, 70, 70) };
    let r = rect.height() / 2.0;
    ui.painter().rect_filled(rect, r, bg);
    let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter().circle_filled(egui::pos2(cx, rect.center().y), r - 3.0, Color32::WHITE);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
