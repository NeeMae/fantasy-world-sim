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
    pub mode: MapMode,
    /// Draw faint outlines between hexes.
    pub grid: bool,
    pub background: Rgb,
}

/// What the map shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapMode {
    /// Biomes with relief symbols: the normal map.
    #[default]
    Terrain,
    /// Height as a colour ramp, from deep sea to snowline.
    Elevation,
    /// Tectonic plates, with boundaries tinted red where plates collide and
    /// blue where they pull apart.
    Plates,
    /// Rainfall, from parched to drenched.
    Rainfall,
    /// Temperature, from frozen to tropical.
    Temperature,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            layout: HexLayout::default(),
            mode: MapMode::Terrain,
            grid: false,
            background: Rgb(12, 14, 22),
        }
    }
}

/// An RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const NONE: u32 = u32::MAX;

/// A rectangle of pixels in map-image coordinates. On a wrapping map it may
/// extend past the left or right edge (negative `x`, or beyond the width),
/// meaning it continues on the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
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
            let x = (rect.x as isize + rx as isize).rem_euclid(map_w as isize) as usize;
            let y = (rect.y as isize + ry as isize).max(0) as usize;
            let i = id as usize;
            let biome = registry.biome(world.terrain.biome[i]);
            let elevation = world.terrain.elevation[i];
            let neighbours =
                [(lx + 1, ly), (lx - 1, ly), (lx, ly + 1), (lx, ly - 1)].map(|(nx, ny)| owner[ny * ow + nx]);
            let coast = neighbours.iter().any(|&n| n != NONE && is_water(n) != biome.water);

            // Ordered dither gives flat colours a hand-placed pixel texture.
            let mut shade = 1.0 + (BAYER[y % 4][x % 4] - 0.5) * 0.10;
            let base = match opts.mode {
                MapMode::Terrain => {
                    shade *= if biome.water { 0.80 + elevation * 0.4 } else { 0.88 + elevation * 0.24 };
                    // Rugged ground darkens slightly, so ranges still read
                    // when zoomed out too far for relief symbols.
                    shade *= 1.0 - world.terrain.ruggedness[i] * 0.3;
                    biome.color
                }
                MapMode::Elevation => elevation_color(elevation, biome.water),
                MapMode::Rainfall => {
                    let c = ramp(
                        &[
                            (0.0, Rgb(150, 105, 60)),
                            (0.3, Rgb(215, 195, 120)),
                            (0.55, Rgb(120, 175, 90)),
                            (0.8, Rgb(40, 130, 110)),
                            (1.0, Rgb(30, 70, 150)),
                        ],
                        world.terrain.moisture[i],
                    );
                    if biome.water { c.scale(0.55) } else { c }
                }
                MapMode::Temperature => {
                    let c = ramp(
                        &[
                            (0.0, Rgb(235, 240, 250)),
                            (0.2, Rgb(110, 150, 220)),
                            (0.45, Rgb(110, 190, 120)),
                            (0.7, Rgb(235, 200, 80)),
                            (1.0, Rgb(210, 70, 50)),
                        ],
                        world.terrain.temperature[i],
                    );
                    if biome.water { c.scale(0.55) } else { c }
                }
                MapMode::Plates => {
                    let g = &world.geology;
                    let plate = g.plate[i];
                    let edge = neighbours.iter().any(|&n| n != NONE && g.plate[n as usize] != plate);
                    if edge {
                        shade *= 0.55;
                    }
                    plate_color(plate, g.plates[plate as usize].continental, g.stress[i], biome.water)
                }
            };
            if coast {
                // Coastline: dark edge on land, pale surf on water.
                shade = if biome.water { 1.35 } else { 0.62 };
            } else if opts.grid && neighbours.iter().any(|&n| n != NONE && n != id) {
                shade *= 0.85;
            }
            let mut c = base.scale(shade);

            let symbols =
                opts.mode == MapMode::Terrain && !coast && !biome.water && opts.layout.size >= MIN_GLYPH_SIZE;
            if symbols
                && let Some(ink) = relief_ink(world, registry, &opts.layout, HexId(id), x, y, map_w, wraps)
            {
                c = match ink {
                    Ink::Light => c.scale(1.25),
                    Ink::Shade => c.scale(0.8),
                    Ink::Dark => c.scale(0.45),
                };
            }
            px.copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
    });
    rgba
}

/// The pixel rectangle covering `hexes`, with a little margin for edge
/// effects that reach into neighbouring pixels. Clamped to the map image,
/// except east-west on a wrapping map, where hexes are measured from the
/// copy nearest the first one so a set straddling the seam stays compact.
pub fn hexes_bounds(topo: &Topology, layout: &HexLayout, hexes: &[HexId]) -> Option<PixelRect> {
    let (map_w, map_h) = layout.image_size(topo);
    let wraps = topo.wrap() == Wrap::X;
    let reach = layout.size + 2.0;
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for &hex in hexes {
        let (mut x, y) = layout.center(topo, hex);
        if let (true, Some(b)) = (wraps, bounds) {
            let anchor = (b.0 + b.2) / 2.0;
            x += ((anchor - x) / map_w as f32).round() * map_w as f32;
        }
        let b = bounds.get_or_insert((x, y, x, y));
        *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    }
    let (x0, y0, x1, y1) = bounds?;
    let (mut x0, mut x1) = ((x0 - reach).floor() as i32, (x1 + reach).ceil() as i32);
    if !wraps {
        (x0, x1) = (x0.max(0), x1.min(map_w as i32));
    }
    let y0 = ((y0 - reach).floor() as i32).max(0);
    let y1 = ((y1 + reach).ceil() as i32).min(map_h as i32);
    Some(PixelRect { x: x0, y: y0, width: (x1 - x0).max(0) as u32, height: (y1 - y0).max(0) as u32 })
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
            let x = (rect.x as isize + (i % w) as isize) as f32 + 0.5;
            let y = (rect.y as isize + (i / w) as isize) as f32 + 0.5;
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

/// Height ramp: navy deeps to pale shallows, then green lowlands through
/// tan and brown uplands to white peaks.
fn elevation_color(e: f32, water: bool) -> Rgb {
    let stops: &[(f32, Rgb)] = if water {
        &[(0.0, Rgb(16, 28, 66)), (0.55, Rgb(70, 130, 190))]
    } else {
        &[
            (0.55, Rgb(70, 140, 70)),
            (0.72, Rgb(170, 175, 95)),
            (0.86, Rgb(150, 110, 70)),
            (0.96, Rgb(120, 100, 90)),
            (1.0, Rgb(245, 245, 250)),
        ]
    };
    ramp(stops, e)
}

fn ramp(stops: &[(f32, Rgb)], v: f32) -> Rgb {
    let lerp = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    for pair in stops.windows(2) {
        let ((v0, c0), (v1, c1)) = (pair[0], pair[1]);
        if v <= v1 {
            let t = ((v - v0) / (v1 - v0).max(1e-6)).clamp(0.0, 1.0);
            return Rgb(lerp(c0.0, c1.0, t), lerp(c0.1, c1.1, t), lerp(c0.2, c1.2, t));
        }
    }
    stops.last().map_or(Rgb(0, 0, 0), |s| s.1)
}

/// A distinct colour per plate: bright for continental crust, muted for
/// oceanic, tinted red (colliding) or blue (pulling apart) near boundaries.
fn plate_color(plate: u16, continental: bool, stress: f32, water: bool) -> Rgb {
    // Golden-angle hue spacing keeps neighbouring ids distinct.
    let hue = (plate as f32 * 137.508) % 360.0;
    let (sat, val) = if continental { (0.45, 0.85) } else { (0.35, 0.55) };
    let mut c = hsv(hue, sat, if water { val * 0.8 } else { val });
    let s = stress.clamp(-1.0, 1.0);
    let tint = if s > 0.0 { Rgb(230, 60, 40) } else { Rgb(60, 120, 240) };
    let t = (s.abs() * 1.4).min(0.85);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    c = Rgb(mix(c.0, tint.0), mix(c.1, tint.1), mix(c.2, tint.2));
    c
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(to(r), to(g), to(b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    /// Sunlit face.
    Light,
    /// Shadowed face.
    Shade,
    /// Outline.
    Dark,
}

/// Pixel-art relief symbols: `D` outline, `L` sunlit face, `S` shadowed face.
///
/// One symbol is drawn per seven-hex "flower" (see [`flower_centre`]), at
/// the flower's centre with a small hashed offset: spaced out like a
/// hand-drawn map, a cluster of mountain hexes reads as a range of peaks
/// rather than a texture.
const MOUNTAIN: [&str; 8] = [
    ".......D.......",
    "......DLD......",
    ".....DLLSD.....",
    "....DLLLSSD....",
    "...DLLLLSSSD...",
    "..DLLLLLSSSSD..",
    ".DLLLLLLSSSSSD.",
    "DLLLLLLLSSSSSSD",
];
const HILLS: [&str; 4] = ["...DDD........", ".DDLLSDD..DDD.", "DLLLLSSSDDLLSD", "..............."];

/// Hexes smaller than this (in pixels) are too small for symbols.
const MIN_GLYPH_SIZE: f32 = 3.5;

/// The centre of the seven-hex flower containing `hex`.
///
/// Hexes with `(q + 3r) mod 7 == 0` form a perfect code on the hex grid:
/// every hex is either one of them or adjacent to exactly one. So each hex
/// belongs to exactly one flower, found with a single step.
fn flower_centre(topo: &Topology, hex: HexId) -> Option<HexId> {
    let a = topo.axial(hex);
    let f = (a.q + 3 * a.r).rem_euclid(7);
    if f == 0 {
        return Some(hex);
    }
    let step = sim_core::Axial::DIRECTIONS.into_iter().find(|d| (f + d.q + 3 * d.r).rem_euclid(7) == 0)?;
    topo.at_axial(a + step)
}

/// The relief-symbol ink (if any) at map pixel `(x, y)`, which lies in hex
/// `own`.
#[allow(clippy::too_many_arguments)]
fn relief_ink(
    world: &World,
    registry: &Registry,
    layout: &HexLayout,
    own: HexId,
    x: usize,
    y: usize,
    map_w: u32,
    wraps: bool,
) -> Option<Ink> {
    let topo = &world.topology;
    let centre = flower_centre(topo, own)?;
    if registry.biome(world.terrain.biome[centre.index()]).water {
        return None;
    }
    let glyph = registry.relief(world.terrain.relief[centre.index()]).glyph;
    let (cx, cy) = layout.center(topo, centre);
    // Nudge each symbol a little so ranges don't look stamped on a grid.
    let h = sim_core::rng::mix(centre.0 as u64, 0, 0, 0);
    let scale = (layout.size / 4.0).floor().max(1.0);
    let jx = ((h & 3) as f32 - 1.5) * scale;
    let jy = (((h >> 2) & 3) as f32 - 1.5) * scale * 0.5;
    let mut dx = x as f32 + 0.5 - cx - jx;
    if wraps {
        dx -= (dx / map_w as f32).round() * map_w as f32;
    }
    glyph_pixel(glyph, layout.size, dx, y as f32 + 0.5 - cy - jy)
}

/// Which ink, if any, a relief glyph puts at offset `(dx, dy)` from its
/// centre, for hexes of size `size`. Symbols scale up with bigger hexes.
fn glyph_pixel(glyph: content::Glyph, size: f32, dx: f32, dy: f32) -> Option<Ink> {
    let rows: &[&str] = match glyph {
        content::Glyph::None => return None,
        content::Glyph::Mountains => &MOUNTAIN,
        content::Glyph::Hills => &HILLS,
    };
    let scale = (size / 4.0).floor().max(1.0);
    let (w, h) = (rows[0].len() as i32, rows.len() as i32);
    let col = (dx / scale).floor() as i32 + w / 2;
    let row = (dy / scale).floor() as i32 + h / 2;
    let line = rows.get(usize::try_from(row).ok()?)?;
    match line.as_bytes().get(usize::try_from(col).ok()?)? {
        b'L' => Some(Ink::Light),
        b'S' => Some(Ink::Shade),
        b'D' => Some(Ink::Dark),
        _ => None,
    }
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
                let x = (o.rect.x + (i as u32 % o.rect.width) as i32) as f32 + 0.5;
                let y = (o.rect.y + (i as u32 / o.rect.width) as i32) as f32 + 0.5;
                assert_eq!(layout.hex_at(&topo, x, y), Some(hex));
            }
        }
        assert!(drawn > 20, "an outline was drawn ({drawn} px)");
        let (cx, cy) = layout.center(&topo, hex);
        let (lx, ly) = ((cx as i32 - o.rect.x) as u32, (cy as i32 - o.rect.y) as u32);
        assert_eq!(o.rgba[((ly * o.rect.width + lx) * 4 + 3) as usize], 0, "the middle is see-through");
    }

    #[test]
    fn bounds_stay_compact_across_the_seam() {
        let topo = Topology::new(40, 20, Wrap::X);
        let layout = HexLayout::default();
        let (map_w, _) = layout.image_size(&topo);
        let brush = topo.within(topo.at(0, 10).unwrap(), 2);
        let r = hexes_bounds(&topo, &layout, &brush).unwrap();
        assert!(r.width < map_w / 4, "brush at the seam isn't stretched across the map ({r:?})");
        assert!(r.x < 0 || r.x + r.width as i32 > map_w as i32, "it extends past an edge ({r:?})");
        // Every brush hex is traced within that compact rectangle.
        let o = outline(&topo, &layout, &brush, 1, [255; 4]).unwrap();
        assert_eq!(o.rect, r);
        assert!(o.rgba.chunks(4).filter(|p| p[3] != 0).count() > 30);
    }

    #[test]
    fn every_hex_belongs_to_exactly_one_flower() {
        let topo = Topology::new(30, 20, Wrap::None);
        for hex in topo.ids() {
            let (col, row) = topo.offset(hex);
            // Skip the border, where a flower's centre can be off the map.
            if col == 0 || row == 0 || col == 29 || row == 19 {
                continue;
            }
            let centre = flower_centre(&topo, hex).expect("interior hexes have a centre");
            assert!(centre == hex || topo.neighbors(hex).any(|n| n == centre));
            let centres = std::iter::once(hex)
                .chain(topo.neighbors(hex))
                .filter(|&h| flower_centre(&topo, h) == Some(h));
            assert_eq!(centres.count(), 1, "exactly one centre within reach");
        }
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
