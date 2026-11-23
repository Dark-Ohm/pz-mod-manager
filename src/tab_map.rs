//! Вкладка «Карта»: сетка клеток мира, игроки, метки. Масштаб колёсиком, перетаскивание мышью,
//! правый клик — меню. Фон — необязательная картинка (онлайн-тайлы pzmap.org закрыты Cloudflare).
use crate::i18n::t;
use crate::config::Marker;
use crate::{App, map};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke};

/// Размер плитки пирамиды в пикселях; уровень L: 1 пиксель = 2^L тайлов мира
const TILE: f32 = 256.0;
/// Границы карты мира (клетки) из pyramid.txt ванильной Muldraugh, KY
const TERRAIN_BOUNDS: [i32; 4] = [0, 0, 19968, 16128];

/// pyramid.zip карты, которую игра показывает на клавише M (Linux: .../projectzomboid/media, Windows: .../media)
fn find_pyramid(st: &crate::config::Settings) -> Option<std::path::PathBuf> {
    let g = &st.client_game_dir;
    ["projectzomboid/media/maps/Muldraugh, KY/pyramid.zip", "media/maps/Muldraugh, KY/pyramid.zip"].iter().map(|r| g.join(r)).find(|p| p.is_file())
}

/// Плитка пирамиды из zip → текстура
fn load_tile(zip: &mut zip::ZipArchive<std::fs::File>, ctx: &egui::Context, key: (u8, u32, u32)) -> Option<egui::TextureHandle> {
    use std::io::Read;
    let mut f = zip.by_name(&format!("{}/tile{}x{}.png", key.0, key.1, key.2)).ok()?;
    let mut buf = vec![];
    f.read_to_end(&mut buf).ok()?;
    let img = image::load_from_memory(&buf).ok()?.to_rgba8();
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    Some(ctx.load_texture(format!("tile{}_{}_{}", key.0, key.1, key.2), ci, egui::TextureOptions::LINEAR))
}

/// POIData.lua карты ПДА (BetterWorldMap, клавиша M) из модов окружения
fn find_poi_file(srv: &crate::config::ServerCfg, st: &crate::config::Settings) -> Option<std::path::PathBuf> {
    for root in [srv.mods_dir(), srv.client_mods_dir(st)] {
        for e in walkdir::WalkDir::new(&root).max_depth(8).into_iter().filter_map(|e| e.ok()) {
            if e.file_name() == "POIData.lua" && e.path().to_string_lossy().contains("BetterWorldMap") {
                return Some(e.path().to_path_buf());
            }
        }
    }
    None
}

/// Строки вида `{ "bank", 2428, 13974 },`
fn parse_poi(text: &str) -> Vec<(String, i32, i32)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix('{')?.trim_end_matches(',').trim_end_matches('}');
            let mut it = l.split(',').map(str::trim);
            let cat = it.next()?.trim_matches('"').to_string();
            Some((cat, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

fn poi_color(cat: &str) -> Color32 {
    let h = cat.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let hue = (h % 360) as f32 / 360.0;
    let c = egui::ecolor::Hsva::new(hue, 0.55, 0.95, 1.0);
    Color32::from(c)
}

impl App {
    pub fn tab_map(&mut self, ui: &mut egui::Ui) {
        self.refresh_players();
        let srv = self.store.settings.server().clone();
        let want = find_poi_file(&srv, &self.store.settings);
        if self.map_poi.as_ref().map(|(p, _)| p) != want.as_ref() {
            self.map_poi = want.map(|p| {
                let pts = std::fs::read_to_string(&p).map(|t| parse_poi(&t)).unwrap_or_default();
                (p, pts)
            });
        }
        let pyramid = find_pyramid(&self.store.settings);
        let use_terrain = self.map_terrain_on && pyramid.is_some() && srv.map_image.as_os_str().is_empty();
        let [bx0, by0, bx1, by1] = if use_terrain { TERRAIN_BOUNDS } else { srv.map_bounds };
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
            ui.heading(t("Карта ПДА (M)"));
            if pyramid.is_some() {
                if ui.checkbox(&mut self.map_terrain_on, t("Карта мира (как на клавише M)")).changed() {
                    self.map_fit = true;
                }
            } else {
                ui.weak(t("pyramid.zip игры не найден: проверь папку клиента игры в настройках"));
            }
            match &self.map_poi {
                Some((p, pts)) => {
                    ui.checkbox(&mut self.map_poi_on, tf!("Точки интереса: {}", pts.len())).on_hover_text(p.display().to_string());
                    let mut cats: Vec<&str> = pts.iter().map(|(c, _, _)| c.as_str()).collect();
                    cats.sort_unstable();
                    cats.dedup();
                    egui::CollapsingHeader::new(t("Категории")).id_salt("poicats").show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            for c in cats {
                                let mut on = !self.map_poi_hidden.contains(c);
                                ui.horizontal(|ui| {
                                    ui.colored_label(poi_color(c), "●");
                                    if ui.checkbox(&mut on, c).changed() {
                                        if on {
                                            self.map_poi_hidden.remove(c);
                                        } else {
                                            self.map_poi_hidden.insert(c.to_string());
                                        }
                                    }
                                });
                            }
                        });
                    });
                }
                None => {
                    ui.weak(t("Мод карты ПДА (BetterWorldMap) не найден в модах окружения"));
                }
            }
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

            // карта мира: пирамида плиток игры
            if use_terrain {
                if let Some(pz) = &pyramid {
                    if self.map_zip.as_ref().map(|(p, _)| p) != Some(pz) {
                        self.map_zip = std::fs::File::open(pz).ok().and_then(|f| zip::ZipArchive::new(f).ok()).map(|z| (pz.clone(), z));
                        self.map_tiles.clear();
                    }
                    let level = ((1.0 / v.scale.max(0.001)).log2().floor().max(0.0) as u8).min(4);
                    let tw = TILE * (1u32 << level) as f32;
                    let (wx0, wy0) = v.to_world(rect, rect.min);
                    let (wx1, wy1) = v.to_world(rect, rect.max);
                    let (tx0, ty0) = ((wx0 / tw).floor().max(0.0) as u32, (wy0 / tw).floor().max(0.0) as u32);
                    let (tx1, ty1) = ((wx1 / tw).floor().max(0.0) as u32, (wy1 / tw).floor().max(0.0) as u32);
                    if self.map_tiles.len() > 600 {
                        self.map_tiles.clear();
                    }
                    let mut loaded = 0;
                    let mut pending = false;
                    for ty in ty0..=ty1.min(63 >> level) {
                        for tx in tx0..=tx1.min(77 >> level) {
                            let key = (level, tx, ty);
                            if !self.map_tiles.contains_key(&key) {
                                if loaded >= 8 {
                                    pending = true;
                                    continue;
                                }
                                let tex = self.map_zip.as_mut().and_then(|(_, z)| load_tile(z, ui.ctx(), key));
                                self.map_tiles.insert(key, tex);
                                loaded += 1;
                            }
                            if let Some(Some(tex)) = self.map_tiles.get(&key) {
                                let r = egui::Rect::from_two_pos(v.to_screen(rect, tx as f32 * tw, ty as f32 * tw), v.to_screen(rect, (tx + 1) as f32 * tw, (ty + 1) as f32 * tw));
                                clip.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                        }
                    }
                    if pending {
                        ui.ctx().request_repaint();
                    }
                }
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

            // точки интереса карты ПДА
            if self.map_poi_on {
                if let Some((_, pts)) = &self.map_poi {
                    let labels = v.scale > 0.12;
                    for (cat, x, y) in pts {
                        if self.map_poi_hidden.contains(cat) {
                            continue;
                        }
                        let p = v.to_screen(rect, *x as f32, *y as f32);
                        if !rect.expand(20.0).contains(p) {
                            continue;
                        }
                        clip.circle_filled(p, 3.0, poi_color(cat));
                        if labels {
                            clip.text(p + egui::vec2(5.0, -1.0), Align2::LEFT_CENTER, cat, FontId::proportional(10.0), Color32::from_gray(190));
                        }
                    }
                }
            }

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
