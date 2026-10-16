//! Procedural world generation.
//!
//! Produces elevation, moisture and temperature for every cell of a sphere
//! grid (see [`sim_core::Topology`]), then assigns each cell the
//! best-matching biome from the loaded content. Generation is a pure
//! function of `(params, registry)`.
//!
//! The planet is a sphere in noise space whose pole-to-pole distance is
//! `world_size`; every layer (continents, plates, climate) is sampled at the
//! cells' 3D positions on it, so there are no seams and no poles pinched by
//! a projection.
//!
//! Elevation and moisture are *percentiles*: a value of `0.3` means 30% of
//! the world is lower (or drier). Biome ranges in content therefore read as
//! proportions, e.g. `elevation: (0.0, 0.55)` makes the lowest 55% of the
//! world sea, whatever the seed.

use content::{BiomeId, Registry, Site};
use std::collections::HashMap;

use content::ReliefId;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin, RidgedMulti};
use rayon::prelude::*;
use sim_core::{Geology, Plate, Terrain, Topology, World, rng};
use sphere::Shape;
use tectonics::{PlateLayer, smoothstep};

mod climate;
mod currents;
mod erosion;
mod hydrology;
mod seas;
mod sphere;
mod tectonics;

#[derive(Clone, Debug)]
pub struct WorldGenParams {
    pub seed: u64,
    /// Grid frequency: the planet has `10 × frequency² + 2` cells. The same
    /// world comes out at any frequency, just finer or coarser.
    pub frequency: u32,
    /// How big the planet is, as the distance from pole to pole in noise
    /// space: larger planets hold more, smaller continents.
    pub world_size: f64,
    /// Size of landmasses: larger values give fewer, bigger continents.
    pub continent_scale: f64,
    /// World-wide rules stored in the world (e.g. volatility).
    pub rules: sim_core::WorldRules,
    /// Share of the planet under sea, 0..1. `None` uses the content's sea
    /// level (the top of its water biomes' elevation range).
    pub ocean: Option<f64>,
    /// Added to every hex's temperature (0..1 scale): positive for a hotter
    /// world, negative for an ice age.
    pub temperature: f64,
    /// Scales rainfall: above 1 is wetter (more forest, bigger rivers),
    /// below 1 drier.
    pub rainfall: f64,
    /// Scales tectonic mountain building: 0 is worn-down, gentle land, 2 is
    /// young and jagged.
    pub mountains: f64,
    /// How deeply basins sink into continental interiors, 0..1: higher
    /// values flood more of them into inland seas.
    pub inland_seas: f64,
    /// How much rivers and weather wear the land down: 0 none, 1 normal,
    /// 2 ancient, deeply carved land.
    pub erosion: f64,
    /// How many streams count as rivers, 0..1: 0 only the great rivers, 1
    /// every sizeable stream. Changes what's drawn and watered, not where
    /// water flows.
    pub rivers: f64,
    /// Scales how big lakes grow before evaporation balances their inflow:
    /// 0 gives none (closed basins end in dry flats), 2 big lakes.
    pub lakes: f64,
}

impl Default for WorldGenParams {
    fn default() -> Self {
        WorldGenParams {
            seed: 0,
            frequency: 130,
            world_size: 4.0,
            continent_scale: 1.0,
            rules: sim_core::WorldRules::default(),
            ocean: None,
            temperature: 0.0,
            rainfall: 1.0,
            mountains: 1.0,
            inland_seas: 0.3,
            erosion: 1.0,
            rivers: 0.5,
            lakes: 1.0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorldGenError {
    #[error("the loaded content defines no biomes")]
    NoBiomes,
    #[error("grid frequency must be 1..={max} (got {0})", max = sim_core::MAX_FREQUENCY)]
    BadFrequency(u32),
    #[error("world size must be positive (got {0})")]
    BadWorldSize(f64),
}

/// The discharge a stream needs to count as a river for a `rivers` setting
/// (0..1): from 8× the default threshold (great rivers only) down to half
/// of it, with the middle setting at twice the default.
pub fn river_threshold(rivers: f64) -> f32 {
    let v = rivers.clamp(0.0, 1.0);
    (sim_core::RIVER_DISCHARGE as f64 * 8f64.powf(1.0 - v) * 0.5f64.powf(v)) as f32
}

/// The planet's radius in noise space for a world size (pole to pole).
pub fn planet_radius(world_size: f64) -> f64 {
    world_size / std::f64::consts::PI
}

/// Frequency of the coarse copy of the world that erosion works on (about
/// 37 000 cells), so every grid gets the same valleys.
const EROSION_FREQUENCY: u32 = 61;
/// Raw depth of cutting at which a valley floor stops being rugged.
const VALLEY_DEPTH: f64 = 0.3;

/// Erodes a coarse copy of the world; returns its topology, how much each
/// of its hexes rose (negative where cut down) and its eroded heights.
fn erosion_delta(
    params: &WorldGenParams,
    layers: &Layers,
    sea_share: f64,
) -> Option<(Topology, Vec<f32>, Vec<f32>)> {
    if params.erosion <= 0.0 {
        return None;
    }
    let coarse = Topology::new(EROSION_FREQUENCY.min(params.frequency));
    let shape = Shape::new(&coarse, planet_radius(params.world_size));
    let before: Vec<f32> = (0..coarse.len() as u32)
        .into_par_iter()
        .map(|i| sample(&coarse, layers, &shape, sim_core::HexId(i)).elevation)
        .collect();
    let coast = {
        let mut sorted = before.clone();
        sorted.sort_unstable_by(f32::total_cmp);
        sorted[((sorted.len() - 1) as f64 * sea_share) as usize]
    };
    let mut after = before.clone();
    erosion::erode(&coarse, &mut after, coast, shape.cell_area, shape.spacing, params.erosion);
    let delta = after.iter().zip(&before).map(|(a, b)| a - b).collect();
    Some((coarse, delta, after))
}

/// How far inland (noise-space units) a coast feels the current offshore.
const COAST_REACH: f64 = 0.09;

/// Independent noise layers, each seeded from the world seed.
struct Layers {
    /// Continental crust: broad swells and basins that decide where land
    /// is, independently of plate outlines (as on Earth, most coasts are
    /// not plate boundaries).
    continents: Fbm<Perlin>,
    /// Coastline wiggle and small hills.
    detail: Fbm<Perlin>,
    /// Sharp crests that turn tectonic uplift into ridgelines and valleys.
    ridges: RidgedMulti<Perlin>,
    /// Distorts the coordinates the terrain is sampled at, so plate
    /// boundaries and coasts wander instead of running straight.
    warp: Fbm<Perlin>,
    /// A broader, stronger warp for plate boundaries alone, so plates are
    /// irregular rather than convex cells.
    plate_warp: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    /// Wobbles climate bands so they don't follow lines of latitude exactly.
    climate: Fbm<Perlin>,
    /// Basins sinking into continental interiors (inland seas).
    basins: Fbm<Perlin>,
    /// Where ocean-floor mountain building breaks the surface as islands.
    volcanoes: tectonics::Volcanoes,
    major_plates: PlateLayer,
    minor_plates: PlateLayer,
    continent_scale: f64,
    mountains: f64,
    inland_seas: f64,
}

/// The land at one point, before normalisation.
struct Land {
    elevation: f64,
    ruggedness: f64,
    /// Smooth strength of the mountain range here (before ridging).
    massif: f64,
    plate: tectonics::Site,
    stress: f64,
}

impl Layers {
    fn new(params: &WorldGenParams, shape: &Shape) -> Self {
        let (seed, continent_scale) = (params.seed, params.continent_scale);
        let layer_seed = |layer| rng::mix(seed, 0, layer, rng::purpose::WORLDGEN) as u32;
        // Skip detail octaves finer than about three hexes: they can't be
        // drawn and only add speckle. Each octave doubles the frequency.
        const DETAIL_FREQUENCY: f64 = 3.2;
        let cells_per_unit = 1.0 / shape.spacing;
        let detail_octaves =
            ((cells_per_unit / (3.0 * DETAIL_FREQUENCY)).log2().floor() as i64 + 1).clamp(1, 5) as usize;
        let mut major_plates = PlateLayer::on_planet(seed, 0, 1.0 * continent_scale, shape.radius);
        let mut minor_plates = PlateLayer::on_planet(seed, 1, 0.38 * continent_scale, shape.radius);
        // Plates cover the planet, padded for domain warping.
        let r = shape.radius + 1.5 * continent_scale;
        major_plates.prepare([-r; 3], [r; 3]);
        minor_plates.prepare([-r; 3], [r; 3]);
        Layers {
            continents: Fbm::<Perlin>::new(layer_seed(0))
                .set_octaves(3)
                .set_frequency(1.1 / continent_scale)
                .set_persistence(0.45),
            detail: Fbm::<Perlin>::new(layer_seed(3))
                .set_octaves(detail_octaves)
                .set_frequency(DETAIL_FREQUENCY)
                .set_persistence(0.5),
            ridges: RidgedMulti::<Perlin>::new(layer_seed(5)).set_octaves(4).set_frequency(5.0),
            warp: Fbm::<Perlin>::new(layer_seed(4)).set_octaves(3).set_frequency(1.4 / continent_scale),
            plate_warp: Fbm::<Perlin>::new(layer_seed(6)).set_octaves(2).set_frequency(0.8 / continent_scale),
            moisture: Fbm::<Perlin>::new(layer_seed(1)).set_octaves(4).set_frequency(2.2),
            climate: Fbm::<Perlin>::new(layer_seed(2)).set_octaves(3).set_frequency(3.0),
            volcanoes: tectonics::Volcanoes::new(seed, 0.09 * continent_scale.sqrt(), true),
            major_plates,
            minor_plates,
            continent_scale,
            mountains: params.mountains.max(0.0),
            inland_seas: params.inland_seas.clamp(0.0, 1.0),
            basins: Fbm::<Perlin>::new(layer_seed(8)).set_octaves(2).set_frequency(1.7 / continent_scale),
        }
    }

    /// The land at noise-space point `p`.
    fn land(&self, p: [f64; 3]) -> Land {
        let strength = 0.25 * self.continent_scale;
        let offset = |o: f64| self.warp.get([p[0] + o, p[1] - o, p[2] + o]) * strength;
        let q = [p[0] + offset(10.0), p[1] + offset(20.0), p[2] + offset(30.0)];

        let plate_offset =
            |o: f64| self.plate_warp.get([q[0] - o, q[1] + o, q[2] - o]) * 0.4 * self.continent_scale;
        let pq = [q[0] + plate_offset(40.0), q[1] + plate_offset(50.0), q[2] + plate_offset(60.0)];
        // Continental crust: broad swells that decide where land is. Plates
        // see the same field (offset by the plate warp) to judge each side
        // of a boundary.
        let crust = |x: [f64; 3]| self.continents.get(x);
        let islands = self.volcanoes.get(q);
        let t = tectonics::evaluate(
            &self.major_plates,
            &self.minor_plates,
            pq,
            |x| crust([x[0] - pq[0] + q[0], x[1] - pq[1] + q[1], x[2] - pq[2] + q[2]]),
            islands,
        );
        let t = tectonics::Tectonics { orogeny: t.orogeny * self.mountains, ..t };
        let detail = self.detail.get(q);
        // Ridged noise is ~[-1, 1]; as 0..1 it carves uplift into crests.
        let crest = ((self.ridges.get(q) + 1.0) / 2.0).clamp(0.0, 1.0);
        // Basins sag into the interiors of continents, away from their
        // coasts, deep enough to flood: the Caspian, the Black Sea.
        let interior = smoothstep(0.05, 0.3, crust(q));
        // The setting widens how much of the interior sags, and deepens it.
        let s = self.inland_seas;
        let basin = if s <= 0.0 {
            0.0
        } else {
            smoothstep(0.5 - 0.45 * s, 0.8 - 0.4 * s, self.basins.get(q)) * interior * (0.3 + 0.3 * s)
        };
        let elevation = crust(q) * 0.6 + detail * 0.18 + t.uplift + t.orogeny * (0.55 + 0.45 * crest) - basin;
        // Rugged ground follows the crests, so uplift reads as ridgelines
        // with gentler ground between rather than solid massifs.
        let ruggedness = (t.orogeny * (0.2 + 0.8 * crest * crest) * 1.5 + detail.abs() * 0.1).clamp(0.0, 1.0);
        // The greatest collisions reach ~0.8 uplift; old and microplate
        // ranges stay well below, like the Appalachians beside the Alps.
        let massif = (t.orogeny / 0.8).clamp(0.0, 1.0);
        Land { elevation, ruggedness, massif, plate: t.plate, stress: t.stress }
    }
}

/// Raw values for one hex, before normalisation.
struct Sample {
    elevation: f32,
    /// Local variation in rainfall (soils, small weather systems), ~[-1, 1].
    moisture: f32,
    warmth: f32,
    ruggedness: f32,
    massif: f32,
    plate: tectonics::Site,
    stress: f32,
}

pub fn generate(params: &WorldGenParams, registry: &Registry) -> Result<World, WorldGenError> {
    if registry.biomes().is_empty() {
        return Err(WorldGenError::NoBiomes);
    }
    if !(1..=sim_core::MAX_FREQUENCY).contains(&params.frequency) {
        return Err(WorldGenError::BadFrequency(params.frequency));
    }
    if !(params.world_size > 0.0 && params.world_size.is_finite()) {
        return Err(WorldGenError::BadWorldSize(params.world_size));
    }
    let topology = Topology::new(params.frequency);
    let shape = Shape::new(&topology, planet_radius(params.world_size));
    let layers = Layers::new(params, &shape);

    let samples: Vec<Sample> = (0..topology.len() as u32)
        .into_par_iter()
        .map(|i| sample(&topology, &layers, &shape, sim_core::HexId(i)))
        .collect();

    let mut terrain = Terrain::new(topology.len());
    let mut geology = Geology::default();
    let mut variation = Vec::with_capacity(topology.len());
    // Plates are numbered in order of first appearance, so ids are stable.
    let mut plate_ids: HashMap<[i32; 3], u16> = HashMap::new();
    for (i, s) in samples.into_iter().enumerate() {
        terrain.elevation[i] = s.elevation;
        terrain.temperature[i] = s.warmth;
        terrain.ruggedness[i] = s.ruggedness;
        terrain.massif[i] = s.massif;
        variation.push(s.moisture);
        let next = plate_ids.len() as u16;
        let id = *plate_ids.entry(s.plate.cell).or_insert_with(|| {
            geology.plates.push(Plate { continental: false });
            next
        });
        geology.plate.push(id);
        geology.stress.push(s.stress);
    }
    // Latitude (degrees from the equator) from the climate bands, before
    // altitude cooling, so the bands' wobble carries into the winds.
    let latitude: Vec<f32> = terrain.temperature.iter().map(|w| 90.0 * (1.0 - w.clamp(0.0, 1.0))).collect();
    let sea_level = registry.sea_level();
    // Wear the land down. Erosion runs on a copy of the world at a fixed
    // resolution, so it carves the same valleys at every scale, and the
    // change is blended onto the map. Where it cut deep, a range is broken
    // into separate ridges by its valleys.
    let share = params.ocean.unwrap_or(sea_level as f64).clamp(0.0, 1.0);
    let carved = erosion_delta(params, &layers, share);
    if let Some((coarse, delta, after)) = &carved {
        let delta = sphere::resample(coarse, delta, &topology);
        let after = sphere::resample(coarse, after, &topology);
        let original = terrain.elevation.clone();
        for i in 0..terrain.len() {
            let e = terrain.elevation[i];
            // Cut down by the coarse cut, but never below the coarse eroded
            // surface: detail too fine for the coarse copy (the foot of a
            // cliff) isn't dug out by cutting meant for the slope above.
            let new = if delta[i] < 0.0 { e.min((e + delta[i]).max(after[i])) } else { e + delta[i] };
            terrain.elevation[i] = new;
            terrain.ruggedness[i] *= 1.0 - smoothstep(0.0, VALLEY_DEPTH, (e - new) as f64) as f32 * 0.6;
        }
        // Blending a coarse valley onto the map leaves it a chain of dips
        // (deepest at coarse hex centres, with saddles between), which rivers
        // would have to climb out of. Erosion makes no new basins: fill each
        // dip back up, but never above the ground's original height, so
        // basins that were there before (lakes, inland seas) stay.
        let coast = {
            let mut sorted = terrain.elevation.clone();
            sorted.sort_unstable_by(f32::total_cmp);
            sorted[((sorted.len() - 1) as f64 * share) as usize]
        };
        let sea: Vec<bool> = terrain.elevation.iter().map(|&e| e < coast).collect();
        let filled = hydrology::fill(&topology, &terrain.elevation, &sea);
        for i in 0..terrain.len() {
            let f = filled[i] as f32;
            if f > terrain.elevation[i] {
                terrain.elevation[i] = f.min(original[i]).max(terrain.elevation[i]);
            }
        }
    }

    to_percentiles(&mut terrain.elevation);
    if let Some(ocean) = params.ocean {
        // Stretch the ranks so the chosen share of the map falls below the
        // content's sea level, keeping biome elevation bands meaningful.
        let s = ocean.clamp(0.01, 0.99) as f32;
        for e in &mut terrain.elevation {
            *e = if *e < s {
                *e / s * sea_level
            } else {
                sea_level + (*e - s) / (1.0 - s) * (1.0 - sea_level)
            };
        }
    }
    let sea: Vec<bool> = terrain.elevation.iter().map(|&e| e < sea_level).collect();
    let inland = seas::inland(&topology, &sea);
    for (i, &is_sea) in sea.iter().enumerate() {
        if is_sea {
            // Relief and ranges belong to land; seas are flat.
            terrain.ruggedness[i] = 0.0;
            terrain.massif[i] = 0.0;
        } else {
            // The highest ground is broken country even away from plate
            // boundaries: plateaus and old uplands become at least hills.
            let e = terrain.elevation[i] as f64;
            terrain.ruggedness[i] = (terrain.ruggedness[i] + smoothstep(0.86, 1.0, e) as f32 * 0.45).min(1.0);
        }
    }
    // Higher and more rugged ground is colder, so ranges carry snow.
    for ((t, e), r) in terrain.temperature.iter_mut().zip(&terrain.elevation).zip(&terrain.ruggedness) {
        *t = (*t + params.temperature as f32 - (e - 0.62).max(0.0) * 0.6 - r * 0.12).clamp(0.0, 1.0);
    }
    terrain.relief = terrain.ruggedness.iter().map(|&r| choose_relief(registry, r)).collect();

    // Ocean currents warm or chill the sea, and with it the coasts nearby
    // (and so how much rain the air off that sea carries inland).
    let rings = sphere::rings(&topology, &shape);
    let current = currents::anomaly(&topology, &shape, &rings, &sea, &latitude);
    terrain.current = current.clone();
    let coastal = sphere::smooth(&topology, &shape, &current, COAST_REACH);
    for i in 0..terrain.len() {
        let shift = if sea[i] { 0.15 * current[i] } else { 0.3 * coastal[i] };
        terrain.temperature[i] = (terrain.temperature[i] + shift).clamp(0.0, 1.0);
    }

    // Rainfall from prevailing winds, wrung out by mountains.
    let height: Vec<f32> = (0..terrain.len())
        .map(|i| {
            if sea[i] {
                0.0
            } else {
                (terrain.elevation[i] - sea_level) / (1.0 - sea_level).max(1e-3) + terrain.ruggedness[i] * 0.5
            }
        })
        .collect();
    let rain = climate::rainfall(&climate::ClimateInput {
        topology: &topology,
        shape: &shape,
        rings: &rings,
        sea: &sea,
        height: &height,
        latitude: &latitude,
        temperature: &terrain.temperature,
    });

    let rain: Vec<f32> = rain.iter().map(|r| r * params.rainfall.max(0.0) as f32).collect();

    // Rivers and lakes from that rainfall.
    let water = hydrology::compute(
        &topology,
        &terrain.elevation,
        &sea,
        &rain,
        shape.cell_area,
        hydrology::Routing::default(),
        params.lakes.max(0.0),
    );
    terrain.river_threshold = river_threshold(params.rivers);
    terrain.drain = water.drain;
    terrain.discharge = water.discharge;
    for (i, &lake) in water.lake.iter().enumerate() {
        if lake {
            // Lakes are water: flat, and no part of a mountain range.
            terrain.ruggedness[i] = 0.0;
            terrain.massif[i] = 0.0;
            terrain.relief[i] = choose_relief(registry, 0.0);
        }
    }

    // Rivers water their banks: a floodplain stays green through dry land.
    let mut wet: Vec<f32> = rain.iter().zip(&variation).map(|(r, v)| r * (1.0 + 0.3 * v)).collect();
    for i in 0..terrain.len() {
        let size = terrain.river(i);
        if size > 0.0 && !sea[i] {
            wet[i] += 0.25 * size;
            for nb in topology.neighbors(sim_core::HexId(i as u32)) {
                wet[nb.index()] += 0.12 * size;
            }
        }
    }
    terrain.moisture = wet;
    to_percentiles(&mut terrain.moisture);
    // Moisture is ranked so biomes keep their shares whatever the climate;
    // the rainfall setting then bends the ranks wetter or drier.
    let bend = 1.0 / params.rainfall.clamp(0.1, 10.0) as f32;
    for m in &mut terrain.moisture {
        *m = m.powf(bend);
    }

    terrain.biome = (0..terrain.len())
        .into_par_iter()
        .map(|i| {
            let t = &terrain;
            choose_biome(
                registry,
                Site {
                    elevation: t.elevation[i],
                    moisture: t.moisture[i],
                    temperature: t.temperature[i],
                    relief: t.relief[i],
                    massif: t.massif[i],
                    lake: water.lake[i],
                    inland: inland[i],
                },
            )
        })
        .collect();

    // A plate counts as continental if most of it (on this map) is land.
    let mut land = vec![(0u32, 0u32); geology.plates.len()];
    for (plate, biome) in geology.plate.iter().zip(&terrain.biome) {
        let entry = &mut land[*plate as usize];
        entry.0 += !registry.biome(*biome).water as u32;
        entry.1 += 1;
    }
    for (plate, (land, total)) in geology.plates.iter_mut().zip(land) {
        plate.continental = land * 2 > total;
    }

    let mut world = World::new(params.seed, topology, terrain);
    world.geology = geology;
    world.rules = params.rules;
    Ok(world)
}

/// Raw values for one hex.
fn sample(topology: &Topology, layers: &Layers, shape: &Shape, hex: sim_core::HexId) -> Sample {
    let p = topology.position(hex);
    let r = shape.radius;
    // Noise-space position on the planet; other layers sample the same
    // sphere shifted well away, so they're independent.
    let point = |offset: f64| -> [f64; 3] { [p[0] * r + offset, p[1] * r, p[2] * r] };
    let land = layers.land(point(0.0));
    let moisture = layers.moisture.get(point(100.0));
    let latitude = p[2].clamp(-1.0, 1.0).asin().abs() / std::f64::consts::FRAC_PI_2;
    let warmth = 1.0 - latitude + layers.climate.get(point(200.0)) * 0.12;
    Sample {
        elevation: land.elevation as f32,
        moisture: moisture as f32,
        warmth: warmth as f32,
        ruggedness: land.ruggedness as f32,
        massif: land.massif as f32,
        plate: land.plate,
        stress: land.stress as f32,
    }
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

/// The highest-priority relief whose ruggedness range contains `r`; the
/// first relief if none match.
fn choose_relief(registry: &Registry, r: f32) -> ReliefId {
    registry
        .reliefs()
        .iter()
        .enumerate()
        .filter(|(_, d)| d.ruggedness.contains(r))
        .rev()
        .max_by_key(|(_, d)| d.priority)
        .map(|(i, _)| ReliefId(i as u8))
        .unwrap_or_default()
}

/// The highest-priority biome matching the site; the first biome if none
/// match. A lake with no lake biome in the content takes the biome the
/// land there would have had; an inland sea with no inland biome, an
/// ordinary sea biome.
fn choose_biome(registry: &Registry, site: Site) -> BiomeId {
    let best = |site: Site| {
        registry
            .biomes()
            .iter()
            .enumerate()
            .filter(|(_, b)| b.matches(site))
            // `max_by_key` keeps the last maximum; reverse so earlier defs win ties.
            .rev()
            .max_by_key(|(_, b)| b.priority)
            .map(|(i, _)| BiomeId(i as u16))
    };
    best(site)
        .or_else(|| site.lake.then(|| best(Site { lake: false, ..site })).flatten())
        .or_else(|| site.inland.then(|| best(Site { inland: false, ..site })).flatten())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::HexId;

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

    fn params(seed: u64) -> WorldGenParams {
        WorldGenParams { seed, frequency: 24, ..Default::default() }
    }

    #[test]
    fn deterministic_per_seed() {
        let reg = registry();
        let a = generate(&params(7), &reg).unwrap();
        let b = generate(&params(7), &reg).unwrap();
        let c = generate(&params(8), &reg).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        assert_ne!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn same_world_at_any_detail() {
        // A finer grid should draw the same geography more finely.
        let reg = registry();
        let coarse = generate(&WorldGenParams { frequency: 24, ..params(9) }, &reg).unwrap();
        let fine = generate(&WorldGenParams { frequency: 48, ..params(9) }, &reg).unwrap();
        let sea = reg.biome_id("sea").unwrap();
        let mut agree = 0;
        for id in coarse.topology.ids() {
            let f = fine.topology.nearest(coarse.topology.position(id), None);
            agree += ((coarse.terrain.biome[id.index()] == sea) == (fine.terrain.biome[f.index()] == sea))
                as usize;
        }
        let share = agree as f64 / coarse.topology.len() as f64;
        assert!(share > 0.85, "land and sea mostly agree across detail levels ({share:.2})");
    }

    #[test]
    fn poles_are_colder_than_the_equator() {
        let reg = registry();
        let w = generate(&params(4), &reg).unwrap();
        let mean = |pick: &dyn Fn(f64) -> bool| {
            let cells: Vec<usize> = w
                .topology
                .ids()
                .filter(|&id| pick(w.topology.lat_lon(id).0.to_degrees()))
                .map(HexId::index)
                .collect();
            cells.iter().map(|&i| w.terrain.temperature[i]).sum::<f32>() / cells.len() as f32
        };
        let polar = mean(&|lat| lat.abs() > 70.0);
        let tropical = mean(&|lat| lat.abs() < 20.0);
        assert!(tropical > polar + 0.4, "tropics {tropical} vs poles {polar}");
    }

    #[test]
    fn rejects_bad_params() {
        let reg = registry();
        let bad = WorldGenParams { world_size: 0.0, ..params(1) };
        assert!(matches!(generate(&bad, &reg), Err(WorldGenError::BadWorldSize(_))));
        let bad = WorldGenParams { frequency: 0, ..params(1) };
        assert!(matches!(generate(&bad, &reg), Err(WorldGenError::BadFrequency(0))));
    }

    #[test]
    fn priority_breaks_ties() {
        let reg = registry();
        let site = |elevation, temperature| Site {
            elevation,
            moisture: 0.5,
            temperature,
            relief: content::ReliefId(0),
            massif: 0.0,
            lake: false,
            inland: false,
        };
        assert_eq!(choose_biome(&reg, site(0.9, 0.05)), reg.biome_id("snow").unwrap());
        assert_eq!(choose_biome(&reg, site(0.9, 0.5)), reg.biome_id("land").unwrap());
    }

    #[test]
    fn great_ranges_get_mountain_biomes_and_small_ones_keep_theirs() {
        let reg = content::load_str(
            r#"(
                reliefs: [(id: "flat"), (id: "mountains")],
                biomes: [
                    (id: "forest"),
                    (id: "alpine", relief: ["mountains"], massif: (0.4, 1.0), priority: 5),
                ],
            )"#,
        )
        .unwrap();
        let mountains = reg.relief_id("mountains").unwrap();
        let site = |relief, massif| Site {
            elevation: 0.8,
            moisture: 0.6,
            temperature: 0.5,
            relief,
            massif,
            lake: false,
            inland: false,
        };
        let forest = reg.biome_id("forest").unwrap();
        let alpine = reg.biome_id("alpine").unwrap();
        assert_eq!(choose_biome(&reg, site(mountains, 0.8)), alpine, "the Alps");
        assert_eq!(choose_biome(&reg, site(mountains, 0.2)), forest, "the Appalachians");
        assert_eq!(choose_biome(&reg, site(content::ReliefId(0), 0.8)), forest, "flat land beside a range");
    }

    #[test]
    fn biome_ranges_are_proportions() {
        let reg = registry();
        let w = generate(&params(11), &reg).unwrap();
        let water = w.terrain.biome.iter().filter(|&&b| reg.biome(b).water).count();
        let share = water as f32 / w.terrain.len() as f32;
        assert!((share - 0.5).abs() < 0.02, "sea covers half the planet, got {share}");
    }
}
