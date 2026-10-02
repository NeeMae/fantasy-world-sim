//! Turns the hex world into pixels.
//!
//! Every pixel is assigned to exactly one hex by geometry (so hexes tile with
//! no gaps or overlaps at any size), then coloured with simple pixel-art
//! treatment: dithered biome colours, shaded relief and outlined coasts.
//!
//! This is shared by the desktop app (as a texture) and the CLI (as a PNG),
//! and is the placeholder until real tilesets arrive via content packs.

use content::{Registry, Rgb};
use rayon::prelude::*;
use sim_core::{Axial, HexId, Topology, World, Wrap};

const SQRT3: f32 = 1.732_050_8;

/// Pixel geometry of flat-topped hexes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HexLayout {
    /// Centre-to-corner distance in pixels; a hex is `2 * size` pixels wide.
    pub size: f32,
}

impl Default for HexLayout {
    fn default() -> Self {
        HexLayout { size: 4.0 }
    }
}

impl HexLayout {
    /// Image size needed to draw the whole map. On wrapping maps the width
    /// tiles seamlessly, so the image can be repeated side by side.
    pub fn image_size(&self, topo: &Topology) -> (u32, u32) {
        let s = self.size;
        let w = match topo.wrap() {
            Wrap::X => s * 1.5 * topo.width() as f32,
            Wrap::None => s * 1.5 * (topo.width() - 1) as f32 + 2.0 * s,
        };
        let h = s * SQRT3 * (topo.height() as f32 + 0.5);
        (w.ceil() as u32, h.ceil() as u32)
    }

    /// Pixel position of a hex's centre.
    pub fn center(&self, topo: &Topology, hex: HexId) -> (f32, f32) {
        let a = topo.axial(hex);
        let s = self.size;
        (s + s * 1.5 * a.q as f32, s * SQRT3 / 2.0 + s * SQRT3 * (a.r as f32 + a.q as f32 / 2.0))
    }

    /// The hex under a pixel position, if any.
    pub fn hex_at(&self, topo: &Topology, x: f32, y: f32) -> Option<HexId> {
        let s = self.size;
        let (x, y) = (x - s, y - s * SQRT3 / 2.0);
        let q = (2.0 / 3.0 * x) / s;
        let r = (-1.0 / 3.0 * x + SQRT3 / 3.0 * y) / s;
        topo.at_axial(round_axial(q, r))
    }
}

/// Rounds fractional axial coordinates to the containing hex.
fn round_axial(q: f32, r: f32) -> Axial {
    let s = -q - r;
    let (mut rq, mut rr, rs) = (q.round(), r.round(), s.round());
    let (dq, dr, ds) = ((rq - q).abs(), (rr - r).abs(), (rs - s).abs());
    if dq > dr && dq > ds {
        rq = -rr - rs;
    } else if dr > ds {
        rr = -rq - rs;
    }
    Axial::new(rq as i32, rr as i32)
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    pub layout: HexLayout,
    /// Draw faint outlines between hexes.
    pub grid: bool,
    pub background: Rgb,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { layout: HexLayout::default(), grid: false, background: Rgb(12, 14, 22) }
    }
}

/// An RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const NONE: u32 = u32::MAX;

pub fn render(world: &World, registry: &Registry, opts: &RenderOptions) -> Image {
    let topo = &world.topology;
    let (width, height) = opts.layout.image_size(topo);
    let (w, h) = (width as usize, height as usize);

    // Pass 1: which hex owns each pixel.
    let owner: Vec<u32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            opts.layout.hex_at(topo, x, y).map_or(NONE, |id| id.0)
        })
        .collect();

    let wraps = topo.wrap() == Wrap::X;
    let owner_at = |x: isize, y: isize| -> u32 {
        if y < 0 || y >= h as isize {
            return NONE;
        }
        let x = if wraps { x.rem_euclid(w as isize) } else { x };
        if x < 0 || x >= w as isize { NONE } else { owner[y as usize * w + x as usize] }
    };
    let is_water = |id: u32| id != NONE && registry.biome(world.terrain.biome[id as usize]).water;

    // Pass 2: colour.
    let mut rgba = vec![0u8; w * h * 4];
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let id = owner[y * w + x];
            let px = &mut row[x * 4..x * 4 + 4];
            if id == NONE {
                px.copy_from_slice(&[opts.background.0, opts.background.1, opts.background.2, 255]);
                continue;
            }
            let (xi, yi) = (x as isize, y as isize);
            let biome = registry.biome(world.terrain.biome[id as usize]);
            let elevation = world.terrain.elevation[id as usize];

            // Ordered dither gives flat colours a hand-placed pixel texture.
            let mut shade = 1.0 + (BAYER[y % 4][x % 4] - 0.5) * 0.10;
            if biome.water {
                shade *= 0.80 + elevation * 0.4; // deeper water is darker
            } else {
                shade *= 0.88 + elevation * 0.24;
            }

            let neighbours = [(1, 0), (-1, 0), (0, 1), (0, -1)].map(|(dx, dy)| owner_at(xi + dx, yi + dy));
            let here_water = biome.water;
            if neighbours.iter().any(|&n| n != NONE && is_water(n) != here_water) {
                // Coastline: dark edge on land, pale surf on water.
                shade = if here_water { 1.35 } else { 0.62 };
            } else if opts.grid && neighbours.iter().any(|&n| n != NONE && n != id) {
                shade *= 0.85;
            }

            let c = biome.color.scale(shade);
            px.copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
    });

    Image { width, height, rgba }
}

/// 4×4 Bayer matrix, normalised to `0.0..1.0`.
const BAYER: [[f32; 4]; 4] = [
    [0.0 / 16.0, 8.0 / 16.0, 2.0 / 16.0, 10.0 / 16.0],
    [12.0 / 16.0, 4.0 / 16.0, 14.0 / 16.0, 6.0 / 16.0],
    [3.0 / 16.0, 11.0 / 16.0, 1.0 / 16.0, 9.0 / 16.0],
    [15.0 / 16.0, 7.0 / 16.0, 13.0 / 16.0, 5.0 / 16.0],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_centres_map_back_to_their_hex() {
        for wrap in [Wrap::None, Wrap::X] {
            let topo = Topology::new(20, 12, wrap);
            for size in [3.0, 4.0, 7.5] {
                let layout = HexLayout { size };
                for id in topo.ids() {
                    let (x, y) = layout.center(&topo, id);
                    assert_eq!(layout.hex_at(&topo, x, y), Some(id));
                }
            }
        }
    }

    #[test]
    fn every_hex_owns_pixels_and_corners_are_off_map() {
        let topo = Topology::new(16, 10, Wrap::None);
        let layout = HexLayout::default();
        let (w, h) = layout.image_size(&topo);
        let mut seen = vec![false; topo.len()];
        for y in 0..h {
            for x in 0..w {
                if let Some(id) = layout.hex_at(&topo, x as f32 + 0.5, y as f32 + 0.5) {
                    seen[id.index()] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(layout.hex_at(&topo, -10.0, -10.0), None);
    }

    #[test]
    fn wrapping_image_tiles_seamlessly() {
        let topo = Topology::new(16, 10, Wrap::X);
        let layout = HexLayout::default();
        let (w, _) = layout.image_size(&topo);
        for y in [5.5, 20.5, 40.5] {
            assert_eq!(layout.hex_at(&topo, 0.5, y), layout.hex_at(&topo, w as f32 + 0.5, y));
        }
    }
}
