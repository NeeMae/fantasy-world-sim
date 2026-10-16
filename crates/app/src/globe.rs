//! The world on a globe.
//!
//! The flat map is drawn once into an image, then wrapped onto a sphere
//! pixel by pixel on the CPU: each pixel of the globe finds the latitude and
//! longitude under it and takes the map's colour there. Pixel art stays
//! crisp, and hovering or clicking finds the hex under the cursor the same
//! way. A wrapping world covers the whole globe (as an equirectangular map:
//! its rows are latitudes, its columns longitudes); a flat region map is a
//! patch on the front of a larger, unexplored world.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;
use bevy_egui::egui;
use content::Registry;
use map_raster::{HexLayout, MapMode, RenderOptions};
use rayon::prelude::*;
use sim_core::{HexId, Topology, World, Wrap};

/// Widest the map image drawn for the globe gets, in pixels.
const SOURCE_WIDTH: f32 = 2400.0;
/// Latitude span of a flat region map on the globe.
const REGION_LATITUDES: f32 = 60.0 * PI / 180.0;
/// Where the light comes from (view space: x right, y up, z out of the screen).
const LIGHT: [f32; 3] = [-0.45, 0.5, 0.74];
const UNEXPLORED: [u8; 3] = [74, 82, 100];
const ATMOSPHERE: [f32; 3] = [120.0, 170.0, 235.0];

/// The map image the globe samples, and how to find hexes in it.
struct Source {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    layout: HexLayout,
    topology: Topology,
    /// For flat maps, half the longitude and latitude spans they cover.
    patch: Option<(f32, f32)>,
    /// What it was drawn from: the world (by its shared allocation) and view.
    key: (usize, MapMode),
}

#[derive(Resource)]
pub struct GlobeView {
    pub enabled: bool,
    /// Longitude and latitude at the centre of the view, in radians.
    yaw: f32,
    pitch: f32,
    /// 1 fits the globe to the panel; more zooms in.
    zoom: f32,
    source: Option<Source>,
    texture: Option<egui::TextureHandle>,
    /// What the texture shows: view angles, zoom, size and source.
    drawn: Option<(f32, f32, f32, [u32; 2], usize, MapMode)>,
}

impl Default for GlobeView {
    fn default() -> Self {
        GlobeView {
            enabled: false,
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            source: None,
            texture: None,
            drawn: None,
        }
    }
}

/// What happened on the globe this frame.
#[derive(Default)]
pub struct GlobeInput {
    /// The hex under the cursor.
    pub hover: Option<HexId>,
    /// The hex clicked, if any.
    pub clicked: Option<HexId>,
}

impl GlobeView {
    /// Draws the globe filling `ui`, handling drag (rotate) and scroll (zoom).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        world: &std::sync::Arc<World>,
        registry: &Registry,
        mode: MapMode,
    ) -> GlobeInput {
        let key = (std::sync::Arc::as_ptr(world) as usize, mode);
        if self.source.as_ref().is_none_or(|s| s.key != key) {
            self.source = Some(draw_source(world, registry, mode, key));
        }
        let source = self.source.as_ref().expect("just drawn");

        let available = ui.available_size().max(egui::vec2(64.0, 64.0));
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        // The globe fits the panel's shorter side; zoomed in it fills it all.
        let side = rect.width().min(rect.height());
        let ppp = ui.ctx().pixels_per_point();
        let pixels = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let square = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));

        // Drag turns the globe under the cursor; scroll zooms.
        let radius_points = side / 2.0 * 0.92 * self.zoom;
        if response.dragged() {
            let d = response.drag_delta();
            self.yaw -= d.x / radius_points;
            self.pitch = (self.pitch + d.y / radius_points).clamp(-FRAC_PI_2 + 0.05, FRAC_PI_2 - 0.05);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(0.6, 8.0);
            }
        }
        self.yaw = self.yaw.rem_euclid(TAU);

        let drawn = (self.yaw, self.pitch, self.zoom, pixels, key.0, key.1);
        if self.drawn != Some(drawn) || self.texture.is_none() {
            let rgba = render(source, pixels, self.yaw, self.pitch, self.zoom);
            let image =
                egui::ColorImage::from_rgba_unmultiplied([pixels[0] as usize, pixels[1] as usize], &rgba);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.texture = Some(ui.ctx().load_texture("globe", image, egui::TextureOptions::NEAREST))
                }
            }
            self.drawn = Some(drawn);
        }
        if let Some(texture) = &self.texture {
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        let hex_under = |pos: egui::Pos2| {
            let (nx, ny) =
                (2.0 * (pos.x - square.min.x) / side - 1.0, 1.0 - 2.0 * (pos.y - square.min.y) / side);
            let (lat, lon) = view_to_globe(nx, ny, self.yaw, self.pitch, self.zoom)?;
            let (x, y) = map_pixel(source, lat, lon)?;
            source.layout.hex_at(&source.topology, x, y)
        };
        let hover = response.hover_pos().and_then(hex_under);
        let clicked =
            if response.clicked() { response.interact_pointer_pos().and_then(hex_under) } else { None };
        GlobeInput { hover, clicked }
    }
}

fn draw_source(world: &World, registry: &Registry, mode: MapMode, key: (usize, MapMode)) -> Source {
    let topology = world.topology;
    let size = (SOURCE_WIDTH / (1.5 * topology.width().max(1) as f32)).clamp(0.5, 6.0);
    let layout = HexLayout { size };
    let image = map_raster::render(world, registry, &RenderOptions { layout, mode, ..Default::default() });
    let patch = (topology.wrap() == Wrap::None).then(|| {
        let half_lat = REGION_LATITUDES / 2.0;
        let aspect = image.width as f32 / image.height.max(1) as f32;
        ((half_lat * aspect).min(PI * 0.95), half_lat)
    });
    Source { width: image.width, height: image.height, rgba: image.rgba, layout, topology, patch, key }
}

/// Latitude and longitude under a point of the view, if it's on the globe.
/// `nx`, `ny` run from -1 to 1 across the square the globe is drawn in.
fn view_to_globe(nx: f32, ny: f32, yaw: f32, pitch: f32, zoom: f32) -> Option<(f32, f32)> {
    let r = 0.92 * zoom;
    let (x, y) = (nx / r, ny / r);
    let r2 = x * x + y * y;
    if r2 > 1.0 {
        return None;
    }
    let z = (1.0 - r2).sqrt();
    let (s, c) = pitch.sin_cos();
    let (y1, z1) = (y * c + z * s, -y * s + z * c);
    Some((y1.clamp(-1.0, 1.0).asin(), x.atan2(z1) + yaw))
}

/// Where a latitude and longitude fall on the map image, if they're on it.
fn map_pixel(source: &Source, lat: f32, lon: f32) -> Option<(f32, f32)> {
    let (w, h) = (source.width as f32, source.height as f32);
    match source.patch {
        None => {
            let u = (lon / TAU).rem_euclid(1.0);
            let v = 0.5 - lat / PI;
            Some((u * w, (v * h).clamp(0.0, h - 1.0)))
        }
        Some((half_lon, half_lat)) => {
            let lon = (lon + PI).rem_euclid(TAU) - PI;
            if lon.abs() > half_lon || lat.abs() > half_lat {
                return None;
            }
            Some(((lon + half_lon) / (2.0 * half_lon) * w, (half_lat - lat) / (2.0 * half_lat) * h))
        }
    }
}

/// Draws the globe as a `width`×`height` RGBA image, the globe fitting its
/// shorter side at zoom 1.
fn render(source: &Source, [width, height]: [u32; 2], yaw: f32, pitch: f32, zoom: f32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let side = width.min(height).max(1) as f32;
    let len = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
    let light = LIGHT.map(|v| v / len);
    let mut out = vec![0u8; w * h * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(row, line)| {
        let ny = (h as f32 - 2.0 * (row as f32 + 0.5)) / side;
        for col in 0..w {
            let nx = (2.0 * (col as f32 + 0.5) - w as f32) / side;
            let px = &mut line[col * 4..col * 4 + 4];
            let r = 0.92 * zoom;
            let d = (nx * nx + ny * ny).sqrt() / r;
            let Some((lat, lon)) = view_to_globe(nx, ny, yaw, pitch, zoom) else {
                // A thin glow of atmosphere round the edge.
                if d < 1.05 {
                    let a = (1.0 - (d - 1.0) / 0.05).clamp(0.0, 1.0);
                    px.copy_from_slice(&[
                        ATMOSPHERE[0] as u8,
                        ATMOSPHERE[1] as u8,
                        ATMOSPHERE[2] as u8,
                        (a * a * 140.0) as u8,
                    ]);
                }
                continue;
            };
            let base = match map_pixel(source, lat, lon) {
                Some((x, y)) => {
                    let (x, y) = ((x as u32).min(source.width - 1), (y as u32).min(source.height - 1));
                    let i = ((y * source.width + x) * 4) as usize;
                    [source.rgba[i], source.rgba[i + 1], source.rgba[i + 2]]
                }
                None => UNEXPLORED,
            };
            // Light and atmosphere, from the point's normal in view space.
            let (x, y) = (nx / r, ny / r);
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            let lambert = (x * light[0] + y * light[1] + z * light[2]).max(0.0);
            let shade = 0.45 + 0.6 * lambert;
            let haze = (1.0 - z).powi(3) * 0.6;
            for c in 0..3 {
                let lit = base[c] as f32 * shade;
                px[c] = (lit * (1.0 - haze) + ATMOSPHERE[c] * haze).clamp(0.0, 255.0) as u8;
            }
            px[3] = 255;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_of_the_view_is_the_point_looked_at() {
        let (lat, lon) = view_to_globe(0.0, 0.0, 1.0, 0.4, 1.0).unwrap();
        assert!((lat - 0.4).abs() < 1e-5 && (lon - 1.0).abs() < 1e-5);
        assert!(view_to_globe(0.99, 0.99, 0.0, 0.0, 1.0).is_none(), "corners are off the globe");
        // Up the screen is north.
        let (north, _) = view_to_globe(0.0, 0.5, 0.0, 0.0, 1.0).unwrap();
        assert!(north > 0.0);
    }
}
