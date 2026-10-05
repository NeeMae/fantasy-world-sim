use content::{BiomeId, ReliefId};
use serde::{Deserialize, Serialize};

use crate::{Date, HexId, Topology};

/// Per-hex physical geography, stored as parallel arrays indexed by [`HexId`].
///
/// Elevation, moisture, temperature and ruggedness are in `0.0..=1.0`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terrain {
    pub elevation: Vec<f32>,
    pub moisture: Vec<f32>,
    pub temperature: Vec<f32>,
    /// How broken the land is, mostly from tectonic uplift; decides relief.
    pub ruggedness: Vec<f32>,
    /// How great the mountain range here is: 0 for none or old worn-down
    /// ranges, 1 for the greatest collision ranges. Shapes mountain biomes.
    pub massif: Vec<f32>,
    pub biome: Vec<BiomeId>,
    /// Flat, hills, mountains, ...: layered on top of the biome.
    pub relief: Vec<ReliefId>,
    /// The hex each hex's water drains into ([`NO_DRAIN`] for the sea or
    /// off the map's edge).
    pub drain: Vec<u32>,
    /// Water flowing out of each hex: its catchment's rainfall × area.
    /// At [`RIVER_DISCHARGE`] and above it's a river.
    pub discharge: Vec<f32>,
    /// Ocean currents: sea-surface temperature anomaly, about −1 (a cold
    /// current) to +1 (a warm one); 0 on land.
    pub current: Vec<f32>,
}

/// [`Terrain::drain`] for water that leaves the land.
pub const NO_DRAIN: u32 = u32::MAX;

/// Discharge at which a stream counts as a river.
pub const RIVER_DISCHARGE: f32 = 0.0025;

impl Terrain {
    pub fn new(len: usize) -> Self {
        Terrain {
            elevation: vec![0.0; len],
            moisture: vec![0.0; len],
            temperature: vec![0.0; len],
            ruggedness: vec![0.0; len],
            massif: vec![0.0; len],
            biome: vec![BiomeId::default(); len],
            relief: vec![ReliefId::default(); len],
            drain: vec![NO_DRAIN; len],
            discharge: vec![0.0; len],
            current: vec![0.0; len],
        }
    }

    /// How well a hex feeds people, 0..=1: its biome's fertility, scaled by
    /// relief, plus a bonus for river floodplains.
    pub fn fertility(&self, registry: &content::Registry, i: usize) -> f32 {
        let base = registry.biome(self.biome[i]).fertility * registry.relief(self.relief[i]).fertility;
        let river = (self.river(i) / 4.0).min(1.0) * 0.25;
        (base + if base > 0.0 { river } else { 0.0 }).clamp(0.0, 1.0)
    }

    /// How hard a hex is to cross, relative to open grassland (1).
    pub fn travel_cost(&self, registry: &content::Registry, i: usize) -> f32 {
        registry.biome(self.biome[i]).travel_cost * registry.relief(self.relief[i]).travel_cost
    }

    /// How big the river through a hex is: 0 for none, 1 at the threshold,
    /// one more for each doubling of discharge beyond it.
    pub fn river(&self, i: usize) -> f32 {
        let q = self.discharge[i] / RIVER_DISCHARGE;
        if q < 1.0 { 0.0 } else { 1.0 + q.log2() }
    }

    pub fn len(&self) -> usize {
        self.biome.len()
    }

    pub fn is_empty(&self) -> bool {
        self.biome.is_empty()
    }
}

/// A tectonic plate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plate {
    /// Whether most of the plate (on this map) is land.
    pub continental: bool,
}

/// The world's tectonic plates, as generated. Kept for display now, and for
/// volcanoes and earthquakes later.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Geology {
    pub plates: Vec<Plate>,
    /// Index into `plates` for every hex.
    pub plate: Vec<u16>,
    /// Signed stress at plate boundaries: positive where plates collide
    /// (mountains, trenches, island arcs), negative where they pull apart
    /// (rifts, ridges), zero in plate interiors.
    pub stress: Vec<f32>,
}

/// World-wide rules chosen when the world is made. Part of the state, so
/// they're covered by determinism.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRules {
    /// How turbulent history is, from 0 (calm: long-lived empires) to 1
    /// (chaotic: constant upheaval). Will scale unrest, rebellion, AI
    /// aggression, succession crises and catastrophes.
    pub volatility: f32,
}

impl Default for WorldRules {
    fn default() -> Self {
        WorldRules { volatility: 0.5 }
    }
}

/// The complete simulation state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub tick: u64,
    pub topology: Topology,
    pub terrain: Terrain,
    pub geology: Geology,
    pub rules: WorldRules,
    /// Incremented whenever terrain changes, so views know when to redraw
    /// the map instead of diffing it.
    pub terrain_revision: u64,
}

impl World {
    pub fn new(seed: u64, topology: Topology, terrain: Terrain) -> Self {
        assert_eq!(topology.len(), terrain.len(), "terrain size must match the topology");
        let geology = Geology {
            plates: vec![Plate { continental: true }],
            plate: vec![0; terrain.len()],
            stress: vec![0.0; terrain.len()],
        };
        World { seed, tick: 0, topology, terrain, geology, rules: WorldRules::default(), terrain_revision: 0 }
    }

    pub fn date(&self) -> Date {
        Date::from_tick(self.tick)
    }

    pub fn contains(&self, hex: HexId) -> bool {
        hex.index() < self.topology.len()
    }

    /// A 64-bit fingerprint of the full state, for determinism checks.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.seed);
        h.u64(self.tick);
        h.u64(self.terrain_revision);
        h.u64(self.rules.volatility.to_bits() as u64);
        let t = &self.terrain;
        for v in t
            .elevation
            .iter()
            .chain(&t.moisture)
            .chain(&t.temperature)
            .chain(&t.ruggedness)
            .chain(&t.massif)
            .chain(&t.discharge)
            .chain(&t.current)
        {
            h.u64(v.to_bits() as u64);
        }
        for ((b, r), d) in t.biome.iter().zip(&t.relief).zip(&t.drain) {
            h.u64(b.0 as u64 | (r.0 as u64) << 16 | (*d as u64) << 32);
        }
        let g = &self.geology;
        for (p, s) in g.plate.iter().zip(&g.stress) {
            h.u64(*p as u64 | (s.to_bits() as u64) << 16);
        }
        h.0
    }
}

/// FNV-1a: tiny, stable across platforms and Rust versions (unlike `DefaultHasher`).
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn u64(&mut self, v: u64) {
        for byte in v.to_le_bytes() {
            self.0 = (self.0 ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
