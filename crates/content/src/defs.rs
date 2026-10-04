use serde::{Deserialize, Serialize};

use crate::{ReliefId, Rgb};

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
/// values in `0.0..=1.0` for every hex, plus its relief and *massif* (how
/// great the mountain range it's part of is), then assigns the
/// highest-`priority` biome whose conditions all hold.
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
    /// Whether the biome is for lakes. Lake hexes only take lake biomes, and
    /// other hexes never do.
    #[serde(default)]
    pub lake: bool,
    #[serde(default)]
    pub elevation: Range,
    #[serde(default)]
    pub moisture: Range,
    #[serde(default)]
    pub temperature: Range,
    /// Relief ids this biome needs (e.g. `["mountains"]`); empty for any.
    #[serde(default)]
    pub relief: Vec<String>,
    /// How great a mountain range must be, from 0 (none, or old worn-down
    /// ranges) to 1 (the greatest collision ranges).
    #[serde(default)]
    pub massif: Range,
    /// How well the land feeds people, 0 (barren) to 1 (the richest).
    #[serde(default)]
    pub fertility: f32,
    /// How hard the land is to cross, relative to open grassland (1).
    #[serde(default = "one")]
    pub travel_cost: f32,
    /// Higher priority wins when several biomes match.
    #[serde(default)]
    pub priority: i32,
    /// `relief` resolved against the registry.
    #[serde(skip)]
    pub relief_ids: Vec<ReliefId>,
}

/// The conditions at one hex that decide its biome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub elevation: f32,
    pub moisture: f32,
    pub temperature: f32,
    pub relief: ReliefId,
    pub massif: f32,
    pub lake: bool,
}

impl BiomeDef {
    pub fn matches(&self, site: Site) -> bool {
        self.elevation.contains(site.elevation)
            && self.moisture.contains(site.moisture)
            && self.temperature.contains(site.temperature)
            && self.massif.contains(site.massif)
            && self.lake == site.lake
            && (self.relief_ids.is_empty() || self.relief_ids.contains(&site.relief))
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
    /// A dense cluster of snow-capped peaks, for the highest ground.
    Peaks,
}

impl TryFrom<String> for Glyph {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        match s.as_str() {
            "none" => Ok(Glyph::None),
            "hills" => Ok(Glyph::Hills),
            "mountains" => Ok(Glyph::Mountains),
            "peaks" => Ok(Glyph::Peaks),
            _ => {
                Err(format!("unknown glyph {s:?}; expected \"none\", \"hills\", \"mountains\" or \"peaks\""))
            }
        }
    }
}

impl From<Glyph> for String {
    fn from(g: Glyph) -> String {
        match g {
            Glyph::None => "none",
            Glyph::Hills => "hills",
            Glyph::Mountains => "mountains",
            Glyph::Peaks => "peaks",
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
    /// Multiplies the biome's fertility (steep ground farms worse).
    #[serde(default = "one")]
    pub fertility: f32,
    /// Multiplies the biome's travel cost.
    #[serde(default = "one")]
    pub travel_cost: f32,
}

fn one() -> f32 {
    1.0
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
            fertility: 1.0,
            travel_cost: 1.0,
        }
    }
}

/// A people: elves, dwarves, humans, ...
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaceDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Typical lifespan in years.
    #[serde(default = "lifespan")]
    pub lifespan: f32,
    /// Biome ids this race favours when settling.
    #[serde(default)]
    pub biomes: Vec<String>,
    /// Relief ids this race favours (dwarves and mountains, say).
    #[serde(default)]
    pub reliefs: Vec<String>,
}

fn lifespan() -> f32 {
    70.0
}

/// A culture: a people's way of life, language and naming.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CultureDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The race id this culture belongs to.
    pub race: String,
    #[serde(default)]
    pub description: String,
    pub language: LanguageDef,
    /// What this culture calls its four tiers of title, smallest first
    /// (county, duchy, kingdom, empire for feudal cultures).
    #[serde(default = "title_tiers")]
    pub title_tiers: Vec<String>,
}

fn title_tiers() -> Vec<String> {
    ["County", "Duchy", "Kingdom", "Empire"].map(String::from).to_vec()
}

/// How a culture's names sound.
///
/// Names are built from syllables. Each syllable follows one of the
/// `syllables` patterns, where `C` is a consonant and `V` a vowel, e.g.
/// `"CV"`, `"CVC"`, `"V"`. Repeat an entry in any list to make it more
/// likely. Names containing any `forbidden` sequence are rejected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageDef {
    pub consonants: Vec<String>,
    pub vowels: Vec<String>,
    pub syllables: Vec<String>,
    #[serde(default = "two")]
    pub min_syllables: u32,
    #[serde(default = "three")]
    pub max_syllables: u32,
    #[serde(default)]
    pub forbidden: Vec<String>,
    /// Endings for people's names (e.g. "ric", "wen").
    #[serde(default)]
    pub person_endings: Vec<String>,
    /// Endings for place names (e.g. "ford", "heim").
    #[serde(default)]
    pub place_endings: Vec<String>,
    /// Chance a name takes an ending.
    #[serde(default = "half")]
    pub ending_chance: f32,
}

fn two() -> u32 {
    2
}

fn three() -> u32 {
    3
}

fn half() -> f32 {
    0.5
}
