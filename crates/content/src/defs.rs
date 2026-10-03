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

/// How a relief level is drawn on the placeholder map. Written as a
/// lowercase string (`glyph: "hills"`): def files go through a generic value
/// tree for inheritance, which only keeps enum variants that are strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Glyph {
    #[default]
    None,
    Hills,
    Mountains,
}

impl TryFrom<String> for Glyph {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        match s.as_str() {
            "none" => Ok(Glyph::None),
            "hills" => Ok(Glyph::Hills),
            "mountains" => Ok(Glyph::Mountains),
            _ => Err(format!("unknown glyph {s:?}; expected \"none\", \"hills\" or \"mountains\"")),
        }
    }
}

impl From<Glyph> for String {
    fn from(g: Glyph) -> String {
        match g {
            Glyph::None => "none",
            Glyph::Hills => "hills",
            Glyph::Mountains => "mountains",
        }
        .into()
    }
}

/// A level of terrain relief (flat, hills, mountains, ...), layered on top
/// of the biome so a hex can be, say, forested hills or desert mountains.
///
/// World generation computes a *ruggedness* value in `0.0..=1.0` for every
/// hex, driven mostly by plate tectonics, and assigns the highest-`priority`
/// relief whose range contains it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReliefDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ruggedness: Range,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub glyph: Glyph,
}

impl ReliefDef {
    /// Used when no pack defines any relief.
    pub fn flat() -> Self {
        ReliefDef {
            id: "flat".into(),
            name: "Flat".into(),
            ruggedness: Range::ANY,
            priority: 0,
            glyph: Glyph::None,
        }
    }
}
