//! Вкладка «Карта»: сетка клеток мира, игроки, метки. Масштаб колёсиком, перетаскивание мышью,
//! правый клик — меню. Фон — необязательная картинка (онлайн-тайлы pzmap.org закрыты Cloudflare).
use crate::i18n::t;
use crate::config::Marker;
use crate::{App, map};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke};

impl App {
    pub fn tab_map(&mut self, ui: &mut egui::Ui) {
        self.refresh_players();
        let srv = self.store.settings.server().clone();
        let [bx0, by0, bx1, by1] = srv.map_bounds;
        let mut center: Option<(f32, f32)> = None;
        let mut add_marker: Option<(i32, i32)> = None;
        let mut del_marker: Option<usize> = None;

        egui::Panel::right("mapside").default_size(270.0).show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(t("Игроки"));
            for p in &self.players {
                if let Some((x, y, _)) = p.pos {
                    let t = RichText::new(format!("{} {}", if p.online { "●" } else { "○" }, p.name));
                    if ui.selectable_label(self.pl_sel.as_deref() == Some(p.name.as_str()), t).clicked() {
                        self.pl_sel = Some(p.name.clone());
                        center = Some((x as f32, y as f32));
                    }
                }
            }
            ui.separator();
            ui.heading(t("Метки"));
            for (i, m) in srv.markers.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.link(format!("{} ({}, {})", m.name, m.x, m.y)).clicked() {
                        center = Some((m.x as f32, m.y as f32));
                    }
                    if ui.small_button("✕").clicked() {
                        del_marker = Some(i);
                    }
                });
            }
            ui.add(egui::TextEdit::singleline(&mut self.marker_name).hint_text(t("имя новой метки")).desired_width(f32::INFINITY));
            ui.separator();
            ui.heading(t("Фон карты"));
            let mut cfg_changed = false;
            let c = self.store.settings.server_mut();
            ui.horizontal(|ui| {
                ui.label(if c.map_image.as_os_str().is_empty() { t("нет").to_string() } else { c.map_image.display().to_string() });
            });
            ui.horizontal(|ui| {
                if ui.button(t("Выбрать PNG/JPG…")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter(t("картинка"), &["png", "jpg", "jpeg"]).pick_file() {
                        c.map_image = f;
                        cfg_changed = true;
                    }
                }
                if ui.button(t("Убрать")).clicked() {
                    c.map_image = Default::default();
                    cfg_changed = true;
                }
            });
            ui.label(t("Границы картинки в клетках мира:"));
            for (i, name) in ["x0", "y0", "x1", "y1"].iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(*name);
                    cfg_changed |= ui.add(egui::DragValue::new(&mut c.map_bounds[i]).speed(50)).changed();
                });
            }
            if cfg_changed {
                self.save();
                self.map_fit = true;
            }
            ui.separator();
            ui.weak(t("Колёсико — масштаб, перетаскивание — сдвиг, правый клик — меню. Телепорт по координатам из консоли сервера недоступен (в игре есть /teleportto только для самого админа)."));
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
            let rect = resp.rect;
            if self.map_fit && rect.width() > 10.0 {
                let w = (bx1 - bx0).max(1) as f32;
                let h = (by1 - by0).max(1) as f32;
                self.map_view.scale = (rect.width() / w).min(rect.height() / h) * 0.95;
                self.map_view.origin = (bx0 as f32 - w * 0.02, by0 as f32 - h * 0.02);
                self.map_fit = false;
            }
            if let Some((x, y)) = center {
                let v = &mut self.map_view;
                v.origin = (x - rect.width() / 2.0 / v.scale, y - rect.height() / 2.0 / v.scale);
            }
            if resp.dragged() {
                self.map_view.pan(resp.drag_delta());
            }
            if let Some(p) = resp.hover_pos() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.map_view.zoom_at(rect, p, (scroll * 0.002).exp());
                }
            }
            let v = self.map_view;
            painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 24, 20));
            let clip = painter.with_clip_rect(rect);

            // фоновая картинка
            if !srv.map_image.as_os_str().is_empty() {
                let uri = crate::platform::file_uri(&srv.map_image.to_string_lossy());
                let r = egui::Rect::from_two_pos(v.to_screen(rect, bx0 as f32, by0 as f32), v.to_screen(rect, bx1 as f32, by1 as f32));
                egui::Image::new(uri).paint_at(ui, r.intersect(rect.expand(4000.0)));
            }

            // сетка клеток
            let step = v.grid_step();
            let (wx0, wy0) = v.to_world(rect, rect.min);
            let (wx1, wy1) = v.to_world(rect, rect.max);
            let first = |w: f32| (w / step as f32).floor() as i32 * step;
            let grid = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 150, 120, 40));
            let major = Stroke::new(1.0, Color32::from_rgba_unmultiplied(140, 190, 140, 90));
            let mut gx = first(wx0);
            while (gx as f32) < wx1 {
                let x = v.to_screen(rect, gx as f32, 0.0).x;
                clip.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], if gx % (map::CELL * 10) == 0 { major } else { grid });
                if gx >= 0 {
                    clip.text(egui::pos2(x + 3.0, rect.top() + 2.0), Align2::LEFT_TOP, gx.to_string(), FontId::monospace(10.0), Color32::from_gray(120));
                }
                gx += step;
            }
            let mut gy = first(wy0);
            while (gy as f32) < wy1 {
                let y = v.to_screen(rect, 0.0, gy as f32).y;
                clip.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], if gy % (map::CELL * 10) == 0 { major } else { grid });
                if gy >= 0 {
                    clip.text(egui::pos2(rect.left() + 3.0, y + 2.0), Align2::LEFT_TOP, gy.to_string(), FontId::monospace(10.0), Color32::from_gray(120));
                }
                gy += step;
            }
            clip.rect_stroke(egui::Rect::from_two_pos(v.to_screen(rect, bx0 as f32, by0 as f32), v.to_screen(rect, bx1 as f32, by1 as f32)), 0.0, Stroke::new(1.5, Color32::from_rgb(90, 130, 90)), egui::StrokeKind::Middle);

            // метки
            for m in &srv.markers {
                let p = v.to_screen(rect, m.x as f32, m.y as f32);
                clip.circle_filled(p, 5.0, Color32::from_rgb(230, 180, 70));
                clip.text(p + egui::vec2(8.0, -2.0), Align2::LEFT_CENTER, &m.name, FontId::proportional(12.0), Color32::from_rgb(230, 200, 120));
            }
            // игроки
            let mut nearest: Option<(f32, String)> = None;
            let hover = resp.hover_pos();
            for pl in &self.players {
                let Some((x, y, _)) = pl.pos else { continue };
                let p = v.to_screen(rect, x as f32, y as f32);
                let sel = self.pl_sel.as_deref() == Some(pl.name.as_str());
                let col = if pl.online { Color32::from_rgb(120, 210, 90) } else { Color32::from_gray(130) };
                if sel {
                    clip.circle_stroke(p, 9.0, Stroke::new(2.0, Color32::WHITE));
                }
                clip.circle_filled(p, 5.0, col);
                clip.text(p + egui::vec2(8.0, -2.0), Align2::LEFT_CENTER, &pl.name, FontId::proportional(13.0), Color32::WHITE);
                if let Some(h) = hover {
                    let d = h.distance(p);
                    if d < 14.0 && nearest.as_ref().is_none_or(|(b, _)| d < *b) {
                        nearest = Some((d, pl.name.clone()));
                    }
                }
            }
            if resp.clicked() {
                self.pl_sel = nearest.map(|(_, n)| n);
            }
            if resp.secondary_clicked() {
                self.map_ctx = resp.hover_pos().map(|p| v.to_world(rect, p));
            }
            if let Some(h) = hover {
                let (x, y) = v.to_world(rect, h);
                clip.text(rect.left_bottom() + egui::vec2(8.0, -8.0), Align2::LEFT_BOTTOM, tf!("x {}  y {}   клетка {}_{}", x as i32, y as i32, x.div_euclid(map::CELL as f32) as i32, y.div_euclid(map::CELL as f32) as i32), FontId::monospace(12.0), Color32::from_gray(200));
            }
            let ctx = self.map_ctx;
            let name = self.marker_name.clone();
            resp.context_menu(|ui| {
                if let Some((x, y)) = ctx {
                    let (xi, yi) = (x as i32, y as i32);
                    ui.label(format!("{xi}, {yi}"));
                    if ui.button(t("Копировать координаты")).clicked() {
                        ui.ctx().copy_text(format!("{xi},{yi},0"));
                        ui.close();
                    }
                    if ui.button(t("Добавить метку здесь")).clicked() {
                        add_marker = Some((xi, yi));
                        ui.close();
                    }
                }
            });
            if let Some((x, y)) = add_marker.take() {
                let c = self.store.settings.server_mut();
                let n = if name.trim().is_empty() { tf!("Метка {}", c.markers.len() + 1) } else { name };
                c.markers.push(Marker { name: n, x, y });
                self.marker_name.clear();
                self.save();
            }
        });
        if let Some(i) = del_marker {
            self.store.settings.server_mut().markers.remove(i);
            self.save();
        }
    }
}
