//! Математика карты: мир ↔ экран, шаг сетки.
use eframe::egui::{Pos2, Rect, Vec2};

/// Сторона клетки мира PZ в тайлах
pub const CELL: i32 = 300;

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// мировая точка в левом верхнем углу области
    pub origin: (f32, f32),
    /// пикселей на один тайл
    pub scale: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { origin: (0.0, 0.0), scale: 0.06 }
    }
}

impl View {
    pub fn to_screen(&self, rect: Rect, x: f32, y: f32) -> Pos2 {
        rect.min + Vec2::new((x - self.origin.0) * self.scale, (y - self.origin.1) * self.scale)
    }
    pub fn to_world(&self, rect: Rect, p: Pos2) -> (f32, f32) {
        let d = p - rect.min;
        (self.origin.0 + d.x / self.scale, self.origin.1 + d.y / self.scale)
    }
    /// Масштаб колёсиком вокруг курсора
    pub fn zoom_at(&mut self, rect: Rect, p: Pos2, factor: f32) {
        let (wx, wy) = self.to_world(rect, p);
        self.scale = (self.scale * factor).clamp(0.01, 6.0);
        let d = p - rect.min;
        self.origin = (wx - d.x / self.scale, wy - d.y / self.scale);
    }
    pub fn pan(&mut self, delta: Vec2) {
        self.origin.0 -= delta.x / self.scale;
        self.origin.1 -= delta.y / self.scale;
    }
    /// Шаг линий сетки в тайлах: кратен клетке, не чаще чем раз в ~40 px
    pub fn grid_step(&self) -> i32 {
        let mut step = CELL;
        while (step as f32) * self.scale < 40.0 {
            step *= 2;
        }
        step
    }
}
