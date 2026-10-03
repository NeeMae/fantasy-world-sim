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
use sim_core::{HexId, Topology, World, Wrap, round_axial};

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
        topo.at_axial(round_axial(q as f64, r as f64))
    }
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

/// A rectangle of pixels in map-image coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Renders the whole map.
pub fn render(world: &World, registry: &Registry, opts: &RenderOptions) -> Image {
    let (width, height) = opts.layout.image_size(&world.topology);
    let rgba = render_rect(world, registry, opts, PixelRect { x: 0, y: 0, width, height });
    Image { width, height, rgba }
}

/// Renders one rectangle of the map as RGBA8, row-major. Pixels outside the
/// map get the background colour. Rendering a rectangle gives exactly the
/// same pixels as the same area of a full [`render`].
pub fn render_rect(world: &World, registry: &Registry, opts: &RenderOptions, rect: PixelRect) -> Vec<u8> {
    let topo = &world.topology;
    let (map_w, _) = opts.layout.image_size(topo);
    let wraps = topo.wrap() == Wrap::X;

    // Pass 1: which hex owns each pixel, with a 1px margin so edge effects
    // (coastlines) match across rectangle boundaries.
    let (ow, oh) = (rect.width as usize + 2, rect.height as usize + 2);
    let (ox, oy) = (rect.x as isize - 1, rect.y as isize - 1);
    let owner: Vec<u32> = (0..ow * oh)
        .into_par_iter()
        .map(|i| {
            let mut x = ox + (i % ow) as isize;
            let y = oy + (i / ow) as isize;
            if wraps {
                x = x.rem_euclid(map_w as isize);
            }
            opts.layout.hex_at(topo, x as f32 + 0.5, y as f32 + 0.5).map_or(NONE, |id| id.0)
        })
        .collect();
    let is_water = |id: u32| id != NONE && registry.biome(world.terrain.biome[id as usize]).water;

    // Pass 2: colour.
    let (w, h) = (rect.width as usize, rect.height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    if w == 0 {
        return rgba;
    }
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(ry, row)| {
        for rx in 0..w {
            let (lx, ly) = (rx + 1, ry + 1); // position in `owner`
            let id = owner[ly * ow + lx];
            let px = &mut row[rx * 4..rx * 4 + 4];
            if id == NONE {
                px.copy_from_slice(&[opts.background.0, opts.background.1, opts.background.2, 255]);
                continue;
            }
            let (x, y) = (rect.x as usize + rx, rect.y as usize + ry);
            let biome = registry.biome(world.terrain.biome[id as usize]);
            let elevation = world.terrain.elevation[id as usize];

            // Ordered dither gives flat colours a hand-placed pixel texture.
            let mut shade = 1.0 + (BAYER[y % 4][x % 4] - 0.5) * 0.10;
            if biome.water {
                shade *= 0.80 + elevation * 0.4; // deeper water is darker
            } else {
                shade *= 0.88 + elevation * 0.24;
            }

            let neighbours =
                [(lx + 1, ly), (lx - 1, ly), (lx, ly + 1), (lx, ly - 1)].map(|(nx, ny)| owner[ny * ow + nx]);
            if neighbours.iter().any(|&n| n != NONE && is_water(n) != biome.water) {
                // Coastline: dark edge on land, pale surf on water.
                shade = if biome.water { 1.35 } else { 0.62 };
            } else if opts.grid && neighbours.iter().any(|&n| n != NONE && n != id) {
                shade *= 0.85;
            }

            let c = biome.color.scale(shade);
            px.copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
    });
    rgba
}

/// The pixel rectangle covering `hexes`, clamped to the map image (with a
/// little margin for edge effects that reach into neighbouring pixels).
pub fn hexes_bounds(topo: &Topology, layout: &HexLayout, hexes: &[HexId]) -> Option<PixelRect> {
    let (map_w, map_h) = layout.image_size(topo);
    let reach = layout.size + 2.0;
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for &hex in hexes {
        let (x, y) = layout.center(topo, hex);
        let b = bounds.get_or_insert((x, y, x, y));
        *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    }
    let (x0, y0, x1, y1) = bounds?;
    let x0 = (x0 - reach).floor().max(0.0) as u32;
    let y0 = (y0 - reach).floor().max(0.0) as u32;
    let x1 = ((x1 + reach).ceil() as u32).min(map_w);
    let y1 = ((y1 + reach).ceil() as u32).min(map_h);
    Some(PixelRect { x: x0, y: y0, width: x1.saturating_sub(x0), height: y1.saturating_sub(y0) })
}

/// A transparent RGBA8 overlay tracing the outer edge of a set of hexes,
/// positioned at `rect` in map-image coordinates.
pub struct Outline {
    pub rect: PixelRect,
    pub rgba: Vec<u8>,
}

/// Traces the boundary of `hexes` with a line `thickness` pixels wide, drawn
/// just inside the hexes' pixels so it lines up exactly with the map.
pub fn outline(
    topo: &Topology,
    layout: &HexLayout,
    hexes: &[HexId],
    thickness: u32,
    color: [u8; 4],
) -> Option<Outline> {
    let rect = hexes_bounds(topo, layout, hexes)?;
    let (w, h) = (rect.width as usize, rect.height as usize);
    let mut member = vec![false; topo.len()];
    for &hex in hexes {
        member[hex.index()] = true;
    }
    let inside: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = ((rect.x as usize + i % w) as f32 + 0.5, (rect.y as usize + i / w) as f32 + 0.5);
            layout.hex_at(topo, x, y).is_some_and(|id| member[id.index()])
        })
        .collect();
    let t = thickness.max(1) as isize;
    let is_inside = |x: isize, y: isize| {
        x >= 0 && y >= 0 && x < w as isize && y < h as isize && inside[y as usize * w + x as usize]
    };
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h as isize {
        for x in 0..w as isize {
            if !is_inside(x, y) {
                continue;
            }
            let edge = (-t..=t)
                .any(|dy| (-t..=t).any(|dx| (dx.abs() + dy.abs() <= t) && !is_inside(x + dx, y + dy)));
            if edge {
                let i = (y as usize * w + x as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&color);
            }
        }
    }
    Some(Outline { rect, rgba })
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
    fn rect_render_matches_full_render() {
        let registry = content::load_str(
            r##"(biomes: [(id: "sea", water: true, color: "#0000ff"), (id: "land", color: "#00ff00")])"##,
        )
        .unwrap();
        let topo = Topology::new(24, 16, Wrap::None);
        let mut terrain = sim_core::Terrain::new(topo.len());
        for (i, b) in terrain.biome.iter_mut().enumerate() {
            *b = content::BiomeId(((i * 7) % 3 == 0) as u16);
        }
        let world = World::new(1, topo, terrain);
        let opts = RenderOptions::default();
        let full = render(&world, &registry, &opts);
        let rect = PixelRect { x: 17, y: 9, width: 40, height: 23 };
        let part = render_rect(&world, &registry, &opts, rect);
        for y in 0..rect.height as usize {
            let fy = rect.y as usize + y;
            let full_row =
                &full.rgba[(fy * full.width as usize + rect.x as usize) * 4..][..rect.width as usize * 4];
            assert_eq!(&part[y * rect.width as usize * 4..][..rect.width as usize * 4], full_row);
        }
    }

    #[test]
    fn outline_traces_only_the_selected_hex() {
        let topo = Topology::new(10, 10, Wrap::None);
        let layout = HexLayout { size: 6.0 };
        let hex = topo.at(4, 4).unwrap();
        let o = outline(&topo, &layout, &[hex], 1, [255, 255, 0, 255]).unwrap();
        let mut drawn = 0;
        for i in 0..(o.rect.width * o.rect.height) as usize {
            if o.rgba[i * 4 + 3] != 0 {
                drawn += 1;
                let x = (o.rect.x + i as u32 % o.rect.width) as f32 + 0.5;
                let y = (o.rect.y + i as u32 / o.rect.width) as f32 + 0.5;
                assert_eq!(layout.hex_at(&topo, x, y), Some(hex));
            }
        }
        assert!(drawn > 20, "an outline was drawn ({drawn} px)");
        let (cx, cy) = layout.center(&topo, hex);
        let (lx, ly) = (cx as u32 - o.rect.x, cy as u32 - o.rect.y);
        assert_eq!(o.rgba[((ly * o.rect.width + lx) * 4 + 3) as usize], 0, "the middle is see-through");
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
