//! Turns the world into pixels: the planet seen as a globe, or unrolled into
//! a flat (equirectangular) map.
//!
//! Every pixel finds the point of the planet under it and the cell whose
//! centre is nearest that point (so cells tile with no gaps or overlaps at
//! any zoom), then colours it with simple pixel-art treatment: dithered biome
//! colours, relief symbols, rivers and outlined coasts. Edges (coasts, plate
//! boundaries, the grid) are found from how close the pixel lies to the
//! boundary between neighbouring cells.
//!
//! This is shared by the desktop app (as a texture) and the CLI (as a PNG),
//! and is the placeholder until real tilesets arrive via content packs.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use content::{Registry, Rgb};
use rayon::prelude::*;
use sim_core::{HexId, Topology, World, dot, normalize, tangent_basis};

/// How the planet is laid out on the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// The globe seen from space.
    Globe(View),
    /// A flat map: longitude across, latitude down (equirectangular). At
    /// zoom 1 the whole planet fits the image.
    Map(View),
}

impl Projection {
    /// The whole planet as a flat map, centred on longitude 0.
    pub const WHOLE_MAP: Projection = Projection::Map(View { lon: 0.0, lat: 0.0, zoom: 1.0 });
}

/// Where a globe view looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Longitude and latitude at the centre of the view, in radians.
    pub lon: f64,
    pub lat: f64,
    /// 1 fits the globe to the image's shorter side; more zooms in.
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        View { lon: 0.0, lat: 0.3, zoom: 1.0 }
    }
}

/// Share of the image's shorter side the globe fills at zoom 1.
const GLOBE_FILL: f64 = 0.92;

impl View {
    /// Radius of the globe on screen, in pixels.
    pub fn radius_px(&self, width: u32, height: u32) -> f64 {
        width.min(height).max(1) as f64 / 2.0 * GLOBE_FILL * self.zoom
    }

    /// Radians per pixel on a flat map.
    pub fn map_angle(&self, width: u32, height: u32) -> f64 {
        (TAU / width.max(1) as f64).max(PI / height.max(1) as f64) / self.zoom
    }
}

/// Wraps a longitude difference into −π..π.
fn wrap_angle(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    pub mode: MapMode,
    /// Draw faint outlines between hexes.
    pub grid: bool,
    /// Light the globe from one side, with a haze of atmosphere at its rim.
    pub lighting: bool,
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
    /// How well the land feeds people.
    Fertility,
    /// Ocean currents: warm in red, cold in blue.
    Currents,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { mode: MapMode::Terrain, grid: false, lighting: true, background: Rgb(12, 14, 22) }
    }
}

/// An RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The point of the planet under pixel `(x, y)` (fractional, from the
/// top-left) of a `width × height` image, if any.
pub fn point_at(projection: Projection, width: u32, height: u32, x: f64, y: f64) -> Option<[f64; 3]> {
    match projection {
        Projection::Map(view) => {
            let a = view.map_angle(width, height);
            let lon = view.lon + (x - width as f64 / 2.0) * a;
            let lat = view.lat - (y - height as f64 / 2.0) * a;
            (lat.abs() <= FRAC_PI_2).then(|| sim_core::from_lat_lon(lat, lon))
        }
        Projection::Globe(view) => {
            let r = view.radius_px(width, height);
            let sx = (x - width as f64 / 2.0) / r;
            let sy = (height as f64 / 2.0 - y) / r;
            let d2 = sx * sx + sy * sy;
            if d2 > 1.0 {
                return None;
            }
            Some(view_to_world(view, [sx, sy, (1.0 - d2).sqrt()]))
        }
    }
}

/// The cell under pixel `(x, y)`, if any.
pub fn cell_at(
    topo: &Topology,
    projection: Projection,
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> Option<HexId> {
    point_at(projection, width, height, x, y).map(|p| topo.nearest(p, None))
}

/// Where a point of the planet appears on screen (pixels), and whether it's
/// on the visible side.
pub fn screen_of(projection: Projection, width: u32, height: u32, p: [f64; 3]) -> Option<(f64, f64)> {
    match projection {
        Projection::Map(view) => {
            let (lat, lon) = sim_core::lat_lon(p);
            let a = view.map_angle(width, height);
            Some((
                width as f64 / 2.0 + wrap_angle(lon - view.lon) / a,
                height as f64 / 2.0 + (view.lat - lat) / a,
            ))
        }
        Projection::Globe(view) => {
            let v = world_to_view(view, p);
            if v[2] < 0.0 {
                return None;
            }
            let r = view.radius_px(width, height);
            Some((width as f64 / 2.0 + v[0] * r, height as f64 / 2.0 - v[1] * r))
        }
    }
}

/// View space (x right, y up, z towards the viewer) to the planet.
fn view_to_world(view: View, v: [f64; 3]) -> [f64; 3] {
    // The view looks at (lat, lon): its centre is that point, up is north.
    let centre = sim_core::from_lat_lon(view.lat, view.lon);
    let (east, north) = tangent_basis(centre);
    normalize([
        east[0] * v[0] + north[0] * v[1] + centre[0] * v[2],
        east[1] * v[0] + north[1] * v[1] + centre[1] * v[2],
        east[2] * v[0] + north[2] * v[1] + centre[2] * v[2],
    ])
}

fn world_to_view(view: View, p: [f64; 3]) -> [f64; 3] {
    let centre = sim_core::from_lat_lon(view.lat, view.lon);
    let (east, north) = tangent_basis(centre);
    [dot(p, east), dot(p, north), dot(p, centre)]
}

/// Where the light comes from, in view space.
const LIGHT: [f64; 3] = [-0.45, 0.5, 0.74];
const ATMOSPHERE: [f32; 3] = [120.0, 170.0, 235.0];
const RIVER: Rgb = Rgb(58, 104, 172);

/// Renders the planet as a `width × height` RGBA8 image.
pub fn render(
    world: &World,
    registry: &Registry,
    opts: &RenderOptions,
    projection: Projection,
    width: u32,
    height: u32,
) -> Image {
    let topo = &world.topology;
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return Image { width, height, rgba };
    }
    // How big a cell is on screen decides what's worth drawing.
    let pixel_angle = match projection {
        Projection::Globe(view) => 1.0 / view.radius_px(width, height),
        Projection::Map(view) => view.map_angle(width, height),
    };
    let cell_px = topo.spacing() / pixel_angle;
    let hex_size = (cell_px / 3f64.sqrt()) as f32; // centre-to-corner, as on the old flat map
    let symbols = opts.mode == MapMode::Terrain && hex_size >= MIN_GLYPH_SIZE;
    let glyph_centres = if symbols { glyph_centres(world, registry) } else { Vec::new() };
    let water: Vec<bool> = world.terrain.biome.iter().map(|&b| registry.biome(b).water).collect();
    let light = normalize(LIGHT);

    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let mut hint: Option<HexId> = None;
        for x in 0..w {
            let px = &mut row[x * 4..x * 4 + 4];
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let Some(p) = point_at(projection, width, height, fx, fy) else {
                // Off the globe: space, with a glow of atmosphere at the rim.
                let mut c = [opts.background.0, opts.background.1, opts.background.2, 255];
                if let (true, Projection::Globe(view)) = (opts.lighting, projection) {
                    let r = view.radius_px(width, height);
                    let d = ((fx - w as f64 / 2.0).powi(2) + (h as f64 / 2.0 - fy).powi(2)).sqrt() / r;
                    let a = ((1.0 - (d - 1.0) / 0.05).clamp(0.0, 1.0) * 0.55) as f32;
                    for k in 0..3 {
                        c[k] = (c[k] as f32 * (1.0 - a) + ATMOSPHERE[k] * a) as u8;
                    }
                }
                px.copy_from_slice(&c);
                continue;
            };
            let cell = topo.nearest(p, hint);
            hint = Some(cell);
            let i = cell.index();
            let centre = topo.position(cell);

            // Distance (in pixels) from this pixel to the boundary with each
            // neighbour: (|p−n|² − |p−c|²) / 2|c−n| on the unit sphere.
            let edge_px = |n: HexId| {
                let q = topo.position(n);
                let gap =
                    ((centre[0] - q[0]).powi(2) + (centre[1] - q[1]).powi(2) + (centre[2] - q[2]).powi(2))
                        .sqrt();
                (dot(p, centre) - dot(p, q)) / gap.max(1e-12) / pixel_angle
            };
            let mut coast = false;
            let mut plate_edge = false;
            let mut grid_edge = false;
            for n in topo.neighbors(cell) {
                let d = edge_px(n);
                if d < 1.0 && water[n.index()] != water[i] {
                    coast = true;
                }
                if d < 1.0 && world.geology.plate[n.index()] != world.geology.plate[i] {
                    plate_edge = true;
                }
                if d < 0.5 {
                    grid_edge = true;
                }
            }

            let mut shade = 1.0 + (BAYER[y % 4][x % 4] - 0.5) * 0.10;
            let base = base_color(world, registry, opts.mode, i, plate_edge, &mut shade);
            if coast {
                // Coastline: dark edge on land, pale surf on water.
                shade = if water[i] { 1.35 } else { 0.62 };
            } else if opts.grid && cell_px >= 6.0 && grid_edge {
                shade *= 0.85;
            }
            let mut c = base.scale(shade);

            let rivers = matches!(opts.mode, MapMode::Terrain | MapMode::Rainfall | MapMode::Fertility);
            if rivers && !water[i] && on_river(world, cell, p, pixel_angle) {
                c = RIVER.scale(1.0 + (BAYER[y % 4][x % 4] - 0.5) * 0.08);
            } else if symbols
                && !coast
                && !water[i]
                && let Some(ink) = relief_ink(world, registry, &glyph_centres, cell, p, pixel_angle, hex_size)
            {
                // On very bright ground (snowfields) "lit" can't get any
                // lighter, so both faces step down instead.
                let bright = c.0 as u32 + c.1 as u32 + c.2 as u32 > 600;
                c = match ink {
                    Ink::Light if bright => c.scale(0.95),
                    Ink::Light => c.scale(1.25),
                    Ink::Shade if bright => c.scale(0.74),
                    Ink::Shade => c.scale(0.8),
                    Ink::Dark => c.scale(0.45),
                    Ink::Snow => Rgb(236, 240, 244),
                };
            }

            let mut out = [c.0 as f32, c.1 as f32, c.2 as f32];
            if let (true, Projection::Globe(view)) = (opts.lighting, projection) {
                let v = world_to_view(view, p);
                let lambert = dot(v, light).max(0.0) as f32;
                let shade = 0.45 + 0.6 * lambert;
                let haze = ((1.0 - v[2]).powi(3) * 0.6) as f32;
                for k in 0..3 {
                    out[k] = out[k] * shade * (1.0 - haze) + ATMOSPHERE[k] * haze;
                }
            }
            px.copy_from_slice(&[
                out[0].clamp(0.0, 255.0) as u8,
                out[1].clamp(0.0, 255.0) as u8,
                out[2].clamp(0.0, 255.0) as u8,
                255,
            ]);
        }
    });
    Image { width, height, rgba }
}

/// The colour a map view gives cell `i` (before edges, rivers and symbols),
/// adjusting `shade`.
fn base_color(
    world: &World,
    registry: &Registry,
    mode: MapMode,
    i: usize,
    plate_edge: bool,
    shade: &mut f32,
) -> Rgb {
    let biome = registry.biome(world.terrain.biome[i]);
    let elevation = world.terrain.elevation[i];
    match mode {
        MapMode::Terrain => {
            *shade *= if biome.water { 0.80 + elevation * 0.4 } else { 0.88 + elevation * 0.24 };
            // Rugged ground darkens slightly, so ranges still read
            // when zoomed out too far for relief symbols.
            *shade *= 1.0 - world.terrain.ruggedness[i] * 0.3;
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
        MapMode::Fertility => {
            if biome.water {
                Rgb(40, 50, 70)
            } else {
                ramp(
                    &[
                        (0.0, Rgb(120, 95, 80)),
                        (0.3, Rgb(200, 180, 110)),
                        (0.7, Rgb(110, 175, 70)),
                        (1.0, Rgb(40, 140, 50)),
                    ],
                    world.terrain.fertility(registry, i),
                )
            }
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
        MapMode::Currents => {
            if biome.water {
                ramp(
                    &[
                        (-1.0, Rgb(40, 110, 220)),
                        (-0.3, Rgb(45, 75, 130)),
                        (0.0, Rgb(38, 48, 72)),
                        (0.3, Rgb(130, 60, 70)),
                        (1.0, Rgb(230, 80, 50)),
                    ],
                    world.terrain.current[i],
                )
            } else {
                let t = world.terrain.temperature[i];
                Rgb(70, 72, 66).scale(0.8 + 0.5 * t)
            }
        }
        MapMode::Plates => {
            let g = &world.geology;
            let plate = g.plate[i];
            if plate_edge {
                *shade *= 0.55;
            }
            plate_color(plate, &g.plates[plate as usize], g.stress[i], biome.water)
        }
    }
}

/// Where a cell's river runs through it: its centre nudged a little by a
/// hash, so rivers meander instead of tracing the grid.
fn river_node(topo: &Topology, hex: HexId) -> [f64; 3] {
    let c = topo.position(hex);
    let (east, north) = tangent_basis(c);
    let h = sim_core::rng::mix(hex.0 as u64, 1, 0, 0);
    let unit = |bits: u64| (bits & 0xffff) as f64 / 65535.0 - 0.5;
    let r = topo.spacing() / 3f64.sqrt() * 0.6;
    let (a, b) = (unit(h) * r, unit(h >> 16) * r);
    normalize([
        c[0] + east[0] * a + north[0] * b,
        c[1] + east[1] * a + north[1] * b,
        c[2] + east[2] * a + north[2] * b,
    ])
}

/// Whether point `p`, in cell `own`, lies on a river: the stretch from
/// `own` to the cell it drains into, or from a river draining into it.
fn on_river(world: &World, own: HexId, p: [f64; 3], pixel_angle: f64) -> bool {
    let t = &world.terrain;
    let topo = &world.topology;
    let hex_size = topo.spacing() / 3f64.sqrt();
    let reach = |from: HexId, to: u32| -> bool {
        let size = t.river(from.index());
        if size <= 0.0 || to == sim_core::NO_DRAIN {
            return false;
        }
        // Widths in hex sizes, as on the old flat map at 4 px per hex,
        // scaling up when zoomed in.
        let half_width =
            (0.45 + 0.3 * size.min(4.0) as f64) * (hex_size / pixel_angle / 4.0).max(0.5) * pixel_angle;
        distance_to_segment(p, river_node(topo, from), river_node(topo, HexId(to))) <= half_width
    };
    reach(own, t.drain[own.index()])
        || topo.neighbors(own).any(|nb| t.drain[nb.index()] == own.0 && reach(nb, own.0))
}

/// Distance (chord, about the angle for nearby points) from `p` to the
/// segment `a`–`b`.
fn distance_to_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = dot(ab, ab);
    let t = if len2 > 0.0 { (dot(ap, ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t, ap[2] - ab[2] * t];
    dot(d, d).sqrt()
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

/// A colour per plate: one hue family per group, each plate a shade of it;
/// bright for continental crust, muted for oceanic, vivid for microplates;
/// tinted red (colliding) or blue (pulling apart) near boundaries.
fn plate_color(id: u16, plate: &sim_core::Plate, stress: f32, water: bool) -> Rgb {
    // Golden-angle hue spacing keeps neighbouring groups distinct; plates
    // in a group spread a little either side of its hue.
    let shade = (id as f32 * 0.618_034).fract() - 0.5;
    let hue = (plate.group as f32 * 137.508 + shade * 40.0).rem_euclid(360.0);
    let (sat, val) = match (plate.micro, plate.continental) {
        (true, _) => (0.7, 0.9),
        (false, true) => (0.45, 0.8 + shade * 0.2),
        (false, false) => (0.35, 0.55 + shade * 0.15),
    };
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
    /// Snowcap.
    Snow,
}

/// Pixel-art relief symbols: `D` outline, `L` sunlit face, `S` shadowed
/// face, `W` snow.
///
/// Symbols are drawn only at some cells (see [`glyph_centres`]), with a
/// small hashed offset: spaced out like a hand-drawn map, a cluster of
/// mountain hexes reads as a range of peaks rather than a texture.
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
/// The highest ground: a tall snow-capped peak between two lesser ones,
/// wider than a mountain so the cores of great ranges read as solid massifs.
const PEAKS: [&str; 8] = [
    "........D........",
    ".......DWD.......",
    "......DWWSD......",
    "...D.DLLLSSD.D...",
    "..DWDLLLLLSSDWD..",
    ".DLLLDLLLLSSDLSD.",
    "DLLLLDLLLLLSSDSSD",
    "DLLLLLLLLLLSSSSSD",
];
const HILLS: [&str; 4] = ["...DDD........", ".DDLLSDD..DDD.", "DLLLLSSSDDLLSD", "..............."];

/// Hexes smaller than this (centre to corner, in pixels) are too small for
/// symbols.
const MIN_GLYPH_SIZE: f32 = 3.5;

/// Which cells carry a relief symbol: land cells whose hash beats all their
/// neighbours', which spreads symbols about two cells apart.
fn glyph_centres(world: &World, registry: &Registry) -> Vec<bool> {
    let topo = &world.topology;
    let hash = |i: u32| sim_core::rng::mix(i as u64, 0, 0, 0);
    (0..topo.len() as u32)
        .into_par_iter()
        .map(|i| {
            let mine = hash(i);
            !registry.biome(world.terrain.biome[i as usize]).water
                && topo.neighbors(HexId(i)).all(|n| hash(n.0) < mine)
        })
        .collect()
}

/// The relief-symbol ink (if any) at point `p`, which lies in cell `own`.
fn relief_ink(
    world: &World,
    registry: &Registry,
    centres: &[bool],
    own: HexId,
    p: [f64; 3],
    pixel_angle: f64,
    hex_size: f32,
) -> Option<Ink> {
    let topo = &world.topology;
    // The symbol over this point, from this cell or a neighbour.
    std::iter::once(own).chain(topo.neighbors(own)).filter(|c| centres[c.index()]).find_map(|centre| {
        let glyph = registry.relief(world.terrain.relief[centre.index()]).glyph;
        let c = topo.position(centre);
        let (east, north) = tangent_basis(c);
        let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        // Nudge each symbol a little so ranges don't look stamped on a grid.
        let h = sim_core::rng::mix(centre.0 as u64, 0, 0, 0);
        let scale = (hex_size / 4.0).floor().max(1.0);
        let jx = ((h & 3) as f32 - 1.5) * scale;
        let jy = (((h >> 2) & 3) as f32 - 1.5) * scale * 0.5;
        let dx = (dot(d, east) / pixel_angle) as f32 - jx;
        let dy = (-dot(d, north) / pixel_angle) as f32 - jy;
        glyph_pixel(glyph, hex_size, dx, dy)
    })
}

/// Which ink, if any, a relief glyph puts at offset `(dx, dy)` from its
/// centre, for hexes of size `size`. Symbols scale up with bigger hexes.
fn glyph_pixel(glyph: content::Glyph, size: f32, dx: f32, dy: f32) -> Option<Ink> {
    let rows: &[&str] = match glyph {
        content::Glyph::None => return None,
        content::Glyph::Mountains => &MOUNTAIN,
        content::Glyph::Hills => &HILLS,
        content::Glyph::Peaks => &PEAKS,
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
        b'W' => Some(Ink::Snow),
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
    fn screen_and_planet_agree() {
        let view = View { lon: 0.7, lat: -0.4, zoom: 1.3 };
        for projection in [Projection::Globe(view), Projection::Map(view)] {
            for (x, y) in [(400.0, 300.0), (350.5, 280.25), (512.0, 200.0)] {
                let p = point_at(projection, 800, 600, x, y).unwrap();
                let (sx, sy) = screen_of(projection, 800, 600, p).unwrap();
                assert!(
                    (sx - x).abs() < 1e-6 && (sy - y).abs() < 1e-6,
                    "{projection:?}: ({x}, {y}) -> ({sx}, {sy})"
                );
            }
        }
    }

    #[test]
    fn the_globe_centre_is_where_the_view_looks() {
        let view = View { lon: 1.0, lat: 0.5, zoom: 1.0 };
        let p = point_at(Projection::Globe(view), 600, 600, 300.0, 300.0).unwrap();
        let (lat, lon) = sim_core::lat_lon(p);
        assert!((lat - 0.5).abs() < 1e-3 && (lon - 1.0).abs() < 1e-3);
        // Up the screen is north.
        let above = point_at(Projection::Globe(view), 600, 600, 300.0, 200.0).unwrap();
        assert!(sim_core::lat_lon(above).0 > 0.5);
        assert!(point_at(Projection::Globe(view), 600, 600, 2.0, 2.0).is_none(), "corners are space");
    }

    #[test]
    fn every_pixel_of_the_globe_is_a_cell() {
        let topo = Topology::new(8);
        let mut terrain = sim_core::Terrain::new(topo.len());
        terrain.elevation.fill(0.3);
        let world = World::new(1, topo, terrain);
        let registry =
            content::load_str(r##"(biomes: [(id: "sea", water: true, color: "#204080")])"##).unwrap();
        let img = render(&world, &registry, &RenderOptions::default(), Projection::WHOLE_MAP, 64, 32);
        assert_eq!(img.rgba.len(), 64 * 32 * 4);
        assert!(img.rgba.chunks(4).all(|p| p[3] == 255));
    }
}
