use content::BiomeId;
use serde::{Deserialize, Serialize};

use crate::{Date, HexId, Topology};

/// Per-hex physical geography, stored as parallel arrays indexed by [`HexId`].
///
/// Elevation, moisture and temperature are normalised to `0.0..=1.0`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terrain {
    pub elevation: Vec<f32>,
    pub moisture: Vec<f32>,
    pub temperature: Vec<f32>,
    pub biome: Vec<BiomeId>,
}

impl Terrain {
    pub fn new(len: usize) -> Self {
        Terrain {
            elevation: vec![0.0; len],
            moisture: vec![0.0; len],
            temperature: vec![0.0; len],
            biome: vec![BiomeId::default(); len],
        }
    }

    pub fn len(&self) -> usize {
        self.biome.len()
    }

    pub fn is_empty(&self) -> bool {
        self.biome.is_empty()
    }
}

/// The complete simulation state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct World {
    pub seed: u64,
    pub tick: u64,
    pub topology: Topology,
    pub terrain: Terrain,
    /// Incremented whenever terrain changes, so views know when to redraw
    /// the map instead of diffing it.
    pub terrain_revision: u64,
}

impl World {
    pub fn new(seed: u64, topology: Topology, terrain: Terrain) -> Self {
        assert_eq!(topology.len(), terrain.len(), "terrain size must match the topology");
        World { seed, tick: 0, topology, terrain, terrain_revision: 0 }
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
        let t = &self.terrain;
        for v in t.elevation.iter().chain(&t.moisture).chain(&t.temperature) {
            h.u64(v.to_bits() as u64);
        }
        for b in &t.biome {
            h.u64(b.0 as u64);
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
