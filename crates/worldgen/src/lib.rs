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

#[derive(Clone, Debug)]
pub struct WorldGenParams {
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub wrap: Wrap,
    /// Size of landmasses: larger values give fewer, bigger continents.
    pub continent_scale: f64,
}

impl Default for WorldGenParams {
    fn default() -> Self {
        WorldGenParams { seed: 0, width: 512, height: 320, wrap: Wrap::None, continent_scale: 1.0 }
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
}

/// Independent noise layers, each seeded from the world seed.
struct Layers {
    elevation: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    /// Wobbles climate bands so they don't follow lines of latitude exactly.
    climate: Fbm<Perlin>,
}

impl Layers {
    fn new(seed: u64, continent_scale: f64) -> Self {
        let layer_seed = |layer| rng::mix(seed, 0, layer, rng::purpose::WORLDGEN) as u32;
        Layers {
            elevation: Fbm::<Perlin>::new(layer_seed(0))
                .set_octaves(6)
                .set_frequency(1.6 / continent_scale)
                .set_persistence(0.5),
            moisture: Fbm::<Perlin>::new(layer_seed(1)).set_octaves(4).set_frequency(2.2),
            climate: Fbm::<Perlin>::new(layer_seed(2)).set_octaves(3).set_frequency(3.0),
        }
    }
}

pub fn generate(params: &WorldGenParams, registry: &Registry) -> Result<World, WorldGenError> {
    if registry.biomes().is_empty() {
        return Err(WorldGenError::NoBiomes);
    }
    if params.width == 0 || params.height == 0 {
        return Err(WorldGenError::Empty);
    }
    if params.wrap == Wrap::X && !params.width.is_multiple_of(2) {
        return Err(WorldGenError::OddWrapWidth(params.width));
    }
    let topology = Topology::new(params.width, params.height, params.wrap);
    let layers = Layers::new(params.seed, params.continent_scale);

    let samples: Vec<(f32, f32, f32)> = (0..topology.len() as u32)
        .into_par_iter()
        .map(|i| sample(&topology, &layers, sim_core::HexId(i)))
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
fn sample(topology: &Topology, layers: &Layers, hex: sim_core::HexId) -> (f32, f32, f32) {
    let (col, row) = topology.offset(hex);
    let (w, h) = (topology.width() as f64, topology.height() as f64);
    // Hex centre in "rows" units: columns are 3/4 of a hex width apart and
    // flat-topped hexes are √3/2 as tall as they are wide.
    let x = col as f64 * 0.75 / (3f64.sqrt() / 2.0);
    let y = row as f64 + if col & 1 == 1 { 0.5 } else { 0.0 };
    let (u, v) = (x / h, y / h); // v in 0..1, u scaled to keep hexes isotropic
    let span = w * 0.75 / (3f64.sqrt() / 2.0) / h;

    let point = |offset: f64| -> [f64; 3] {
        match topology.wrap() {
            // Sample a cylinder so the noise is seamless across the east-west seam.
            Wrap::X => {
                let angle = u / span * std::f64::consts::TAU;
                let radius = span / std::f64::consts::TAU;
                [angle.cos() * radius + offset, angle.sin() * radius, v]
            }
            Wrap::None => [u + offset, v, 0.0],
        }
    };

    let mut elevation = layers.elevation.get(point(0.0));
    // Push land away from the borders so a bounded map reads as a continent
    // surrounded by sea. A wrapping map only has north and south edges.
    let edge = match topology.wrap() {
        Wrap::None => (u / span).min(1.0 - u / span).min(v).min(1.0 - v),
        Wrap::X => v.min(1.0 - v),
    };
    elevation -= (1.0 - (edge / 0.18).min(1.0)).powi(2) * 1.6;

    let moisture = layers.moisture.get(point(100.0));
    let latitude_warmth = 1.0 - (v - 0.5).abs() * 2.0 + layers.climate.get(point(200.0)) * 0.12;
    (elevation as f32, moisture as f32, latitude_warmth as f32)
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
    fn bounded_map_has_sea_borders_and_some_land() {
        let reg = registry();
        let w = generate(&params(3, Wrap::None), &reg).unwrap();
        let sea = reg.biome_id("sea").unwrap();
        let land = w.terrain.biome.iter().filter(|&&b| b != sea).count();
        assert!(land > 0 && land < w.terrain.len());
        for col in 0..64 {
            let hex = w.topology.at(col, 0).unwrap();
            assert!(reg.biome(w.terrain.biome[hex.index()]).water, "top edge is sea");
        }
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
