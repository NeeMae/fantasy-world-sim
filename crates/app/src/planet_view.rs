//! The planet on screen: a globe you can turn, or a flat map you can pan,
//! drawn by `map-raster` into a texture whenever the view or the world
//! changes, with hex outlines (hover, selection, brush) drawn over it.

use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::Arc;

use bevy::prelude::*;
use bevy_egui::egui;
use content::Registry;
use map_raster::{MapMode, Projection, RenderOptions, View};
use sim_core::{HexId, Topology, World};

/// Most image pixels between neighbouring hex centres.
const PIXELS_PER_HEX: f32 = 7.0;

/// How the planet is shown, and where the view looks.
#[derive(Resource)]
pub struct PlanetView {
    /// A globe, or a flat map.
    pub globe: bool,
    globe_view: View,
    map_view: View,
    /// Draw faint lines between hexes when they're big enough.
    pub grid: bool,
    texture: Option<egui::TextureHandle>,
    drawn: Option<Drawn>,
    /// Image size last drawn, in pixels, and the panel it filled.
    size: (u32, u32),
    rect: egui::Rect,
}

#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    world: usize,
    mode: MapMode,
    grid: bool,
    projection: Projection,
    size: (u32, u32),
}

impl Default for PlanetView {
    fn default() -> Self {
        PlanetView {
            globe: true,
            globe_view: View { lon: 0.0, lat: 0.35, zoom: 1.0 },
            map_view: View { lon: 0.0, lat: 0.0, zoom: 1.0 },
            grid: false,
            texture: None,
            drawn: None,
            size: (1, 1),
            rect: egui::Rect::NOTHING,
        }
    }
}

/// What the pointer did over the planet this frame.
#[derive(Default)]
pub struct Pointer {
    /// The hex under the pointer.
    pub hover: Option<HexId>,
    /// The primary button went down over the planet.
    pub pressed: bool,
    /// The primary button is held (after going down over the planet).
    pub held: bool,
    /// A primary click (press and release without dragging).
    pub clicked: bool,
}

/// Hexes to outline over the planet.
pub struct Outline<'a> {
    pub hexes: &'a [HexId],
    pub color: egui::Color32,
    pub width: f32,
}

impl PlanetView {
    pub fn projection(&self) -> Projection {
        if self.globe { Projection::Globe(self.globe_view) } else { Projection::Map(self.map_view) }
    }

    fn view_mut(&mut self) -> &mut View {
        if self.globe { &mut self.globe_view } else { &mut self.map_view }
    }

    /// Radians the view turns per screen point at the current zoom.
    fn angle_per_point(&self) -> f64 {
        let (w, h) = (self.rect.width().max(1.0) as u32, self.rect.height().max(1.0) as u32);
        match self.projection() {
            Projection::Globe(v) => 1.0 / v.radius_px(w, h),
            Projection::Map(v) => v.map_angle(w, h),
        }
    }

    /// Turns a globe (or pans a map) by a screen-space amount in points.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let a = self.angle_per_point();
        let globe = self.globe;
        let v = self.view_mut();
        v.lon = (v.lon - dx as f64 * a).rem_euclid(TAU);
        let limit = if globe { FRAC_PI_2 - 0.02 } else { FRAC_PI_2 };
        v.lat = (v.lat + dy as f64 * a).clamp(-limit, limit);
    }

    /// Zooms by a factor about the centre of the view.
    pub fn zoom_by(&mut self, factor: f64) {
        let min = if self.globe { 0.8 } else { 1.0 };
        let v = self.view_mut();
        v.zoom = (v.zoom * factor).clamp(min, 60.0);
    }

    /// Recentres the view on the whole planet.
    pub fn reset(&mut self) {
        *self.view_mut() = if self.globe {
            View { lon: 0.0, lat: 0.35, zoom: 1.0 }
        } else {
            View { lon: 0.0, lat: 0.0, zoom: 1.0 }
        };
    }

    /// Draws the planet filling `ui`, with outlines over it. Secondary or
    /// middle drag turns (or pans) it; the scroll wheel zooms.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        world: &Arc<World>,
        registry: &Registry,
        mode: MapMode,
        outlines: &[Outline],
    ) -> Pointer {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.rect = rect;
        let ppp = ui.ctx().pixels_per_point();

        // Turning and zooming.
        let turning = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle);
        if turning {
            let d = response.drag_delta();
            self.pan(d.x, d.y);
        }
        let scroll = if response.hovered() { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        if scroll != 0.0 {
            self.zoom_by((scroll as f64 * 0.003).exp());
        }

        // Pixel art: never more than PIXELS_PER_HEX image pixels between
        // hex centres, so zooming in enlarges the pixels (as the old flat map
        // did) rather than drawing ever finer. Drawn at half that while
        // moving, so turning stays smooth.
        let hex_points = (world.topology.spacing() / self.angle_per_point()) as f32;
        let detail = (PIXELS_PER_HEX / (hex_points * ppp)).min(1.0);
        let moving = turning || scroll != 0.0;
        let scale = detail * if moving { 0.5 } else { 1.0 };
        let size = (
            ((rect.width() * ppp * scale).round() as u32).max(1),
            ((rect.height() * ppp * scale).round() as u32).max(1),
        );
        let drawn = Drawn {
            world: Arc::as_ptr(world) as usize,
            mode,
            grid: self.grid,
            projection: self.projection(),
            size,
        };
        if self.drawn != Some(drawn) || self.texture.is_none() {
            let opts = RenderOptions { mode, grid: self.grid, lighting: self.globe, ..Default::default() };
            let image = map_raster::render(world, registry, &opts, self.projection(), size.0, size.1);
            let image =
                egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &image.rgba);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.texture = Some(ui.ctx().load_texture("planet", image, egui::TextureOptions::NEAREST))
                }
            }
            self.drawn = Some(drawn);
            self.size = size;
        }
        if let Some(texture) = &self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(texture.id(), rect, uv, egui::Color32::WHITE);
        }

        let painter = ui.painter_at(rect);
        for outline in outlines {
            self.draw_outline(&painter, &world.topology, outline);
        }
        if mode == MapMode::Plates {
            self.draw_plate_motion(&painter, world);
        }

        let hover = response.hover_pos().and_then(|pos| self.hex_at(&world.topology, pos));
        let primary_down = ui.input(|i| i.pointer.primary_down());
        Pointer {
            hover,
            pressed: response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()),
            held: primary_down
                && (response.dragged_by(egui::PointerButton::Primary)
                    || response.is_pointer_button_down_on()),
            clicked: response.clicked(),
        }
    }

    /// The hex under a screen position.
    pub fn hex_at(&self, topo: &Topology, pos: egui::Pos2) -> Option<HexId> {
        let (w, h) = (self.rect.width() as f64, self.rect.height() as f64);
        // Points, not pixels: projections only care about proportions.
        let (scale_w, scale_h) = (1000.0, 1000.0 * h / w.max(1.0));
        let (x, y) =
            ((pos.x - self.rect.min.x) as f64 / w * scale_w, (pos.y - self.rect.min.y) as f64 / h * scale_h);
        map_raster::cell_at(topo, self.projection(), scale_w as u32, scale_h as u32, x, y)
    }

    /// Where a point of the planet is on screen.
    fn screen(&self, p: [f64; 3]) -> Option<egui::Pos2> {
        let (w, h) = (self.rect.width() as f64, self.rect.height() as f64);
        let (sw, sh) = (1000.0, 1000.0 * h / w.max(1.0));
        let (x, y) = map_raster::screen_of(self.projection(), sw as u32, sh as u32, p)?;
        Some(egui::pos2(self.rect.min.x + (x / sw * w) as f32, self.rect.min.y + (y / sh * h) as f32))
    }

    /// An arrow on each plate showing which way it drifts, and its number.
    fn draw_plate_motion(&self, painter: &egui::Painter, world: &World) {
        let color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220);
        let shadow = egui::Color32::from_black_alpha(160);
        for (id, plate) in world.geology.plates.iter().enumerate() {
            let c = plate.centre.map(|v| v as f64);
            let spin = plate.spin.map(|v| v as f64);
            let v = sim_core::cross(spin, c);
            // A long arrow reads better than a true-to-scale one.
            let tip = sim_core::normalize([0, 1, 2].map(|k| c[k] + v[k] * 0.18));
            let (Some(a), Some(b)) = (self.screen(c), self.screen(tip)) else {
                continue;
            };
            if (a.x - b.x).abs() > self.rect.width() * 0.5 {
                continue;
            }
            let font = egui::FontId::proportional(if plate.micro { 10.0 } else { 12.0 });
            painter.text(
                a + egui::vec2(1.0, 1.0),
                egui::Align2::CENTER_BOTTOM,
                format!("#{id}"),
                font.clone(),
                shadow,
            );
            painter.text(a, egui::Align2::CENTER_BOTTOM, format!("#{id}"), font, color);
            let d = b - a;
            if d.length() < 4.0 {
                painter.circle_filled(a, 3.0, color);
                continue;
            }
            // A dark outline under a light arrow, so it shows on any colour.
            for stroke in [egui::Stroke::new(3.5, shadow), egui::Stroke::new(2.0, color)] {
                painter.line_segment([a, b], stroke);
                let dir = d.normalized();
                let side = egui::vec2(-dir.y, dir.x);
                let head = 7.0;
                painter.line_segment([b, b - dir * head + side * head * 0.6], stroke);
                painter.line_segment([b, b - dir * head - side * head * 0.6], stroke);
            }
        }
    }

    /// Traces the outer edge of a set of hexes.
    fn draw_outline(&self, painter: &egui::Painter, topo: &Topology, outline: &Outline) {
        let set: std::collections::HashSet<HexId> = outline.hexes.iter().copied().collect();
        let stroke = egui::Stroke::new(outline.width, outline.color);
        let wrap_jump = self.rect.width() * 0.5;
        for &hex in outline.hexes {
            let corners = topo.corners(hex);
            let neighbours: Vec<HexId> = topo.neighbors(hex).collect();
            // Corner k sits between neighbours k and k+1, so the side shared
            // with neighbour k runs from corner k−1 to corner k.
            for (k, n) in neighbours.iter().enumerate() {
                if set.contains(n) {
                    continue;
                }
                let a = corners[(k + corners.len() - 1) % corners.len()];
                let b = corners[k];
                if let (Some(a), Some(b)) = (self.screen(a), self.screen(b))
                    && (a.x - b.x).abs() < wrap_jump
                {
                    painter.line_segment([a, b], stroke);
                }
            }
        }
    }
}

/// Latitude and longitude of a hex, as text.
pub fn describe_position(topo: &Topology, hex: HexId) -> String {
    let (lat, lon) = topo.lat_lon(hex);
    let (lat, lon) = (lat.to_degrees(), lon.to_degrees());
    let ns = if lat >= 0.0 { "N" } else { "S" };
    let ew = if lon >= 0.0 { "E" } else { "W" };
    format!("{:.1}°{ns} {:.1}°{ew}", lat.abs(), lon.abs())
}
