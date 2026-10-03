//! Procedural world generation.
//!
//! Produces elevation, moisture and temperature for every hex, then assigns
//! each hex the best-matching biome from the loaded content. Generation is a
//! pure function of `(params, registry)`.
//!
//! Elevation and moisture are *percentiles*: a value of `0.3` means 30% of
//! the world is lower (or drier). Biome ranges in content therefore read as
//! proportions, e.g. `elevation: (0.0, 0.55)` makes the lowest 55% of the
//! world sea, whatever the seed.

use content::{BiomeId, Registry};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rayon::prelude::*;
use sim_core::{Terrain, Topology, World, Wrap, rng};

/// What happens at the map's edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeStyle {
    /// The map is a window onto a larger world: land and sea run off the
    /// edges wherever the terrain takes them.
    #[default]
    Open,
    /// The world is ringed by ocean, for a self-contained island or
    /// continent. The falloff is rounded and noisy so coasts don't trace the
    /// map's rectangle.
    Ocean,
}

#[derive(Clone, Debug)]
pub struct WorldGenParams {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub wrap: Wrap,
    pub edges: EdgeStyle,
    /// How much of the planet the map shows. 1 is a region (a sea and the
    /// lands around it); larger values zoom out to several continents.
    /// Independent of `width`/`height`, which only set how finely it's
    /// divided into hexes.
    pub world_size: f64,
    /// Size of landmasses: larger values give fewer, bigger continents.
    pub continent_scale: f64,
    /// Which climate bands the map covers.
    pub latitudes: Latitudes,
}

/// How the map maps onto the planet's climate bands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Latitudes {
    /// Warmth changes steadily from `north` to `south` (0 = polar, 1 = equatorial).
    Span { north: f64, south: f64 },
    /// Pole to pole, with the equator across the middle.
    Globe,
}

impl Default for Latitudes {
    fn default() -> Self {
        Latitudes::Span { north: 0.12, south: 0.88 }
    }
}

impl Default for WorldGenParams {
    fn default() -> Self {
        WorldGenParams {
            seed: 0,
            width: 512,
            height: 320,
            wrap: Wrap::None,
            edges: EdgeStyle::Open,
            world_size: 1.0,
            continent_scale: 1.0,
            latitudes: Latitudes::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorldGenError {
    #[error("the loaded content defines no biomes")]
    NoBiomes,
    #[error("a wrapping world needs an even width (got {0})")]
    OddWrapWidth(u32),
    #[error("world dimensions must be non-zero")]
    Empty,
    #[error("world size must be positive (got {0})")]
    BadWorldSize(f64),
}

/// Independent noise layers, each seeded from the world seed.
struct Layers {
    /// Broad shapes: where the continents and ocean basins are.
    continents: Fbm<Perlin>,
    /// Coastline wiggle, ridges and hills on top of the continents.
    detail: Fbm<Perlin>,
    /// Distorts the coordinates the elevation layers are sampled at, so
    /// landmasses get swirled, irregular shapes rather than round blobs.
    warp: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    /// Wobbles climate bands so they don't follow lines of latitude exactly.
    climate: Fbm<Perlin>,
    continent_scale: f64,
}

impl Layers {
    fn new(seed: u64, continent_scale: f64, world_size: f64, rows: u32) -> Self {
        let layer_seed = |layer| rng::mix(seed, 0, layer, rng::purpose::WORLDGEN) as u32;
        // Skip detail octaves finer than about three hexes: they can't be
        // drawn and only add speckle. Each octave doubles the frequency.
        const DETAIL_FREQUENCY: f64 = 3.2;
        let cycles = DETAIL_FREQUENCY * world_size; // base cycles per map height
        let detail_octaves = ((rows as f64 / (3.0 * cycles)).log2().floor() as i64 + 1).clamp(1, 5) as usize;
        Layers {
            continents: Fbm::<Perlin>::new(layer_seed(0))
                .set_octaves(3)
                .set_frequency(1.1 / continent_scale)
                .set_persistence(0.45),
            detail: Fbm::<Perlin>::new(layer_seed(3))
                .set_octaves(detail_octaves)
                .set_frequency(DETAIL_FREQUENCY)
                .set_persistence(0.5),
            warp: Fbm::<Perlin>::new(layer_seed(4)).set_octaves(3).set_frequency(1.4 / continent_scale),
            moisture: Fbm::<Perlin>::new(layer_seed(1)).set_octaves(4).set_frequency(2.2),
            climate: Fbm::<Perlin>::new(layer_seed(2)).set_octaves(3).set_frequency(3.0),
            continent_scale,
        }
    }

    /// Raw elevation at `p`: domain-warped continents plus detail.
    fn elevation(&self, p: [f64; 3]) -> f64 {
        let strength = 0.25 * self.continent_scale;
        let offset = |o: f64| self.warp.get([p[0] + o, p[1] - o, p[2] + o]) * strength;
        let q = [p[0] + offset(10.0), p[1] + offset(20.0), p[2] + offset(30.0)];
        self.continents.get(q) + self.detail.get(q) * 0.35
    }
}

pub fn generate(params: &WorldGenParams, registry: &Registry) -> Result<World, WorldGenError> {
    if registry.biomes().is_empty() {
        return Err(WorldGenError::NoBiomes);
    }
    if params.width == 0 || params.height == 0 {
        return Err(WorldGenError::Empty);
    }
    if !(params.world_size > 0.0 && params.world_size.is_finite()) {
        return Err(WorldGenError::BadWorldSize(params.world_size));
    }
    if params.wrap == Wrap::X && !params.width.is_multiple_of(2) {
        return Err(WorldGenError::OddWrapWidth(params.width));
    }
    let topology = Topology::new(params.width, params.height, params.wrap);
    let layers = Layers::new(params.seed, params.continent_scale, params.world_size, params.height);

    let samples: Vec<(f32, f32, f32)> = (0..topology.len() as u32)
        .into_par_iter()
        .map(|i| sample(&topology, &layers, params, sim_core::HexId(i)))
        .collect();

    let mut terrain = Terrain::new(topology.len());
    for (i, (e, m, latitude)) in samples.into_iter().enumerate() {
        terrain.elevation[i] = e;
        terrain.moisture[i] = m;
        terrain.temperature[i] = latitude;
    }
    to_percentiles(&mut terrain.elevation);
    to_percentiles(&mut terrain.moisture);
    // Higher ground is colder.
    for (t, e) in terrain.temperature.iter_mut().zip(&terrain.elevation) {
        *t = (*t - (e - 0.6).max(0.0) * 0.5).clamp(0.0, 1.0);
    }
    terrain.biome = terrain
        .elevation
        .par_iter()
        .zip(&terrain.moisture)
        .zip(&terrain.temperature)
        .map(|((&e, &m), &t)| choose_biome(registry, e, m, t))
        .collect();

    Ok(World::new(params.seed, topology, terrain))
}

/// Raw `(elevation, moisture, latitude warmth)` for one hex.
fn sample(
    topology: &Topology,
    layers: &Layers,
    params: &WorldGenParams,
    hex: sim_core::HexId,
) -> (f32, f32, f32) {
    let (col, row) = topology.offset(hex);
    let (w, h) = (topology.width() as f64, topology.height() as f64);
    // Hex centre in "rows" units: columns are 3/4 of a hex width apart and
    // flat-topped hexes are √3/2 as tall as they are wide.
    let x = col as f64 * 0.75 / (3f64.sqrt() / 2.0);
    let y = row as f64 + if col & 1 == 1 { 0.5 } else { 0.0 };
    let (u, v) = (x / h, y / h); // v in 0..1, u scaled to keep hexes isotropic
    let span = w * 0.75 / (3f64.sqrt() / 2.0) / h;

    // Noise-space position, centred on the middle of the map so that a
    // wider canvas or a bigger world reveals more around the same centre.
    // `world_size` is how many "region heights" of terrain the map spans.
    let ws = params.world_size;
    let point = |offset: f64| -> [f64; 3] {
        match topology.wrap() {
            // Sample a cylinder so the noise is seamless across the east-west seam.
            Wrap::X => {
                let angle = u / span * std::f64::consts::TAU;
                let radius = span * ws / std::f64::consts::TAU;
                [angle.cos() * radius + offset, angle.sin() * radius, (v - 0.5) * ws]
            }
            Wrap::None => [(u - span / 2.0) * ws + offset, (v - 0.5) * ws, 0.0],
        }
    };

    let mut elevation = layers.elevation(point(0.0));
    if params.edges == EdgeStyle::Ocean {
        // A gentle slope down from the middle across the whole map makes
        // land more likely towards the centre without dictating where the
        // coast goes, so the terrain still draws it (islands, bays, inland
        // seas). A ring-shaped falloff instead makes every world the same
        // rounded box. A thin band at the very border guarantees open sea all
        // round. A wrapping map only has north and south edges.
        let ny = 2.0 * v - 1.0;
        let (r, border) = match topology.wrap() {
            Wrap::None => {
                let nx = 2.0 * u / span - 1.0;
                ((nx * nx + ny * ny).sqrt() / std::f64::consts::SQRT_2, nx.abs().max(ny.abs()))
            }
            Wrap::X => (ny.abs(), ny.abs()),
        };
        elevation -= r * r * 1.4 + smoothstep(0.9, 0.98, border) * 2.0;
    }

    let moisture = layers.moisture.get(point(100.0));
    let latitude_warmth = match params.latitudes {
        Latitudes::Span { north, south } => north + (south - north) * v,
        Latitudes::Globe => 1.0 - (v - 0.5).abs() * 2.0,
    } + layers.climate.get(point(200.0)) * 0.12;
    (elevation as f32, moisture as f32, latitude_warmth as f32)
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Replaces each value with its rank as a fraction in `0.0..=1.0`.
fn to_percentiles(values: &mut [f32]) {
    let mut order: Vec<u32> = (0..values.len() as u32).collect();
    // Ties break by index so the result never depends on sort stability.
    order.par_sort_unstable_by(|&a, &b| values[a as usize].total_cmp(&values[b as usize]).then(a.cmp(&b)));
    let last = (values.len().max(2) - 1) as f32;
    for (rank, i) in order.into_iter().enumerate() {
        values[i as usize] = rank as f32 / last;
    }
}

/// The highest-priority biome matching the climate; the first biome if none match.
fn choose_biome(registry: &Registry, elevation: f32, moisture: f32, temperature: f32) -> BiomeId {
    registry
        .biomes()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.matches(elevation, moisture, temperature))
        // `max_by_key` keeps the last maximum; reverse so earlier defs win ties.
        .rev()
        .max_by_key(|(_, b)| b.priority)
        .map(|(i, _)| BiomeId(i as u16))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        content::load_str(
            r##"(biomes: [
                (id: "sea", water: true, elevation: (0.0, 0.5)),
                (id: "land", elevation: (0.5, 1.0)),
                (id: "snow", elevation: (0.5, 1.0), temperature: (0.0, 0.1), priority: 1),
            ])"##,
        )
        .unwrap()
    }

    fn params(seed: u64, wrap: Wrap) -> WorldGenParams {
        WorldGenParams { seed, width: 64, height: 40, wrap, ..Default::default() }
    }

    #[test]
    fn deterministic_per_seed() {
        let reg = registry();
        let a = generate(&params(7, Wrap::None), &reg).unwrap();
        let b = generate(&params(7, Wrap::None), &reg).unwrap();
        let c = generate(&params(8, Wrap::None), &reg).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        assert_ne!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn ocean_edges_ring_the_map_with_sea() {
        let reg = registry();
        let w = generate(&WorldGenParams { edges: EdgeStyle::Ocean, ..params(3, Wrap::None) }, &reg).unwrap();
        let sea = reg.biome_id("sea").unwrap();
        let land = w.terrain.biome.iter().filter(|&&b| b != sea).count();
        assert!(land > 0 && land < w.terrain.len());
        for col in 0..64 {
            let hex = w.topology.at(col, 0).unwrap();
            assert!(reg.biome(w.terrain.biome[hex.index()]).water, "top edge is sea");
        }
    }

    #[test]
    fn open_edges_let_land_reach_the_border() {
        let reg = registry();
        let sea = reg.biome_id("sea").unwrap();
        // Across a handful of seeds, land should touch the border somewhere.
        let touches = (0..8).any(|seed| {
            let w = generate(&params(seed, Wrap::None), &reg).unwrap();
            let t = &w.topology;
            let border = (0..64)
                .flat_map(|c| [t.at(c, 0), t.at(c, 39)])
                .chain((0..40).flat_map(|r| [t.at(0, r), t.at(63, r)]));
            border.flatten().any(|h| w.terrain.biome[h.index()] != sea)
        });
        assert!(touches);
    }

    #[test]
    fn same_world_at_any_scale() {
        // Doubling the hex count should redraw the same geography finer.
        let reg = registry();
        let coarse =
            generate(&WorldGenParams { width: 64, height: 40, ..params(9, Wrap::None) }, &reg).unwrap();
        let fine =
            generate(&WorldGenParams { width: 128, height: 80, ..params(9, Wrap::None) }, &reg).unwrap();
        let sea = reg.biome_id("sea").unwrap();
        let mut agree = 0;
        for id in coarse.topology.ids() {
            let (c, r) = coarse.topology.offset(id);
            let f = fine.topology.at(c * 2, r * 2).unwrap();
            agree += ((coarse.terrain.biome[id.index()] == sea) == (fine.terrain.biome[f.index()] == sea))
                as usize;
        }
        let share = agree as f64 / coarse.topology.len() as f64;
        assert!(share > 0.9, "land and sea mostly agree across scales ({share:.2})");
    }

    #[test]
    fn wrapping_world_is_seamless() {
        // Elevation should change no more across the east-west seam than
        // between any other pair of neighbouring columns.
        let reg = registry();
        let w = generate(&WorldGenParams { world_size: 3.0, ..params(5, Wrap::X) }, &reg).unwrap();
        let t = &w.topology;
        let col_step = |a: i32, b: i32| -> f32 {
            (0..t.height() as i32)
                .map(|r| {
                    (w.terrain.elevation[t.at(a, r).unwrap().index()]
                        - w.terrain.elevation[t.at(b, r).unwrap().index()])
                    .abs()
                })
                .sum::<f32>()
        };
        let seam = col_step(63, 0);
        let typical = (1..63).map(|c| col_step(c - 1, c)).sum::<f32>() / 62.0;
        assert!(seam < typical * 2.5, "seam step {seam} vs typical {typical}");
    }

    #[test]
    fn rejects_bad_world_size() {
        let reg = registry();
        let bad = WorldGenParams { world_size: 0.0, ..params(1, Wrap::None) };
        assert!(matches!(generate(&bad, &reg), Err(WorldGenError::BadWorldSize(_))));
    }

    #[test]
    fn priority_breaks_ties() {
        let reg = registry();
        assert_eq!(choose_biome(&reg, 0.9, 0.5, 0.05), reg.biome_id("snow").unwrap());
        assert_eq!(choose_biome(&reg, 0.9, 0.5, 0.5), reg.biome_id("land").unwrap());
    }

    #[test]
    fn biome_ranges_are_proportions() {
        let reg = registry();
        let w = generate(&params(11, Wrap::None), &reg).unwrap();
        let water = w.terrain.biome.iter().filter(|&&b| reg.biome(b).water).count();
        let share = water as f32 / w.terrain.len() as f32;
        assert!((share - 0.5).abs() < 0.01, "sea covers half the map, got {share}");
    }

    #[test]
    fn rejects_bad_params() {
        let reg = registry();
        let odd = WorldGenParams { width: 63, ..params(1, Wrap::X) };
        assert!(matches!(generate(&odd, &reg), Err(WorldGenError::OddWrapWidth(63))));
        assert!(generate(&params(1, Wrap::X), &reg).is_ok());
    }
}
