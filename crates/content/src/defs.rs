use serde::{Deserialize, Serialize};

use crate::Rgb;

/// An inclusive numeric range, written as `(min, max)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Range(pub f32, pub f32);

impl Range {
    pub const ANY: Range = Range(f32::NEG_INFINITY, f32::INFINITY);

    pub fn contains(&self, v: f32) -> bool {
        v >= self.0 && v <= self.1
    }
}

impl Default for Range {
    fn default() -> Self {
        Range::ANY
    }
}

/// A terrain biome.
///
/// World generation produces normalised elevation, moisture and temperature
/// values in `0.0..=1.0` for every hex, then assigns the highest-`priority`
/// biome whose ranges contain all three.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiomeDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Rgb,
    /// Whether the biome is open water (blocks land settlement and movement).
    #[serde(default)]
    pub water: bool,
    #[serde(default)]
    pub elevation: Range,
    #[serde(default)]
    pub moisture: Range,
    #[serde(default)]
    pub temperature: Range,
    /// Higher priority wins when several biomes match.
    #[serde(default)]
    pub priority: i32,
}

impl BiomeDef {
    pub fn matches(&self, elevation: f32, moisture: f32, temperature: f32) -> bool {
        self.elevation.contains(elevation)
            && self.moisture.contains(moisture)
            && self.temperature.contains(temperature)
    }
}
