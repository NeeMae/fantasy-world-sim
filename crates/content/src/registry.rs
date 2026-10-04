use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{BiomeDef, ContentError, CultureDef, PackManifest, RaceDef, ReliefDef};

/// Dense index of a [`BiomeDef`] within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BiomeId(pub u16);

/// Dense index of a [`ReliefDef`] within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReliefId(pub u8);

/// Dense index of a race within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RaceId(pub u16);

/// Dense index of a culture within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CultureId(pub u16);

/// A culture with its race resolved.
#[derive(Clone, Debug)]
pub struct Culture {
    pub def: CultureDef,
    pub race_id: RaceId,
}

/// All loaded content, with string ids resolved to dense indices.
///
/// Def order is the order defs were first defined across packs, so ids are
/// stable for a given set of packs.
#[derive(Clone, Debug)]
pub struct Registry {
    packs: Vec<PackManifest>,
    biomes: Vec<BiomeDef>,
    biome_ids: HashMap<String, BiomeId>,
    reliefs: Vec<ReliefDef>,
    relief_ids: HashMap<String, ReliefId>,
    races: Vec<RaceDef>,
    cultures: Vec<Culture>,
}

impl Registry {
    pub(crate) fn new(
        packs: Vec<PackManifest>,
        mut biomes: Vec<BiomeDef>,
        mut reliefs: Vec<ReliefDef>,
        races: Vec<RaceDef>,
        cultures: Vec<CultureDef>,
    ) -> Result<Self, ContentError> {
        let too_many =
            |kind: &str| ContentError::Invalid { origin: kind.into(), message: format!("too many {kind}") };
        if biomes.len() > u16::MAX as usize {
            return Err(too_many("biomes"));
        }
        if reliefs.len() > u8::MAX as usize {
            return Err(too_many("reliefs"));
        }
        // Every hex needs some relief, so there is always at least one.
        if reliefs.is_empty() {
            reliefs.push(ReliefDef::flat());
        }
        let biome_ids: HashMap<String, BiomeId> =
            biomes.iter().enumerate().map(|(i, b)| (b.id.clone(), BiomeId(i as u16))).collect();
        let relief_ids: HashMap<String, ReliefId> =
            reliefs.iter().enumerate().map(|(i, r)| (r.id.clone(), ReliefId(i as u8))).collect();
        for biome in &mut biomes {
            biome.relief_ids = biome
                .relief
                .iter()
                .map(|id| {
                    relief_ids.get(id).copied().ok_or_else(|| ContentError::Invalid {
                        origin: format!("biome {:?}", biome.id),
                        message: format!("unknown relief {id:?}"),
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        let invalid = |origin: String, message: String| ContentError::Invalid { origin, message };
        for race in &races {
            for b in &race.biomes {
                if !biome_ids.contains_key(b) {
                    return Err(invalid(format!("race {:?}", race.id), format!("unknown biome {b:?}")));
                }
            }
            for r in &race.reliefs {
                if !relief_ids.contains_key(r) {
                    return Err(invalid(format!("race {:?}", race.id), format!("unknown relief {r:?}")));
                }
            }
        }
        let race_ids: HashMap<&str, RaceId> =
            races.iter().enumerate().map(|(i, r)| (r.id.as_str(), RaceId(i as u16))).collect();
        let cultures = cultures
            .into_iter()
            .map(|def| {
                let origin = format!("culture {:?}", def.id);
                let race_id = *race_ids
                    .get(def.race.as_str())
                    .ok_or_else(|| invalid(origin.clone(), format!("unknown race {:?}", def.race)))?;
                let lang = &def.language;
                if lang.consonants.is_empty() || lang.vowels.is_empty() || lang.syllables.is_empty() {
                    return Err(invalid(origin, "a language needs consonants, vowels and syllables".into()));
                }
                if let Some(bad) =
                    lang.syllables.iter().find(|p| p.is_empty() || p.chars().any(|c| c != 'C' && c != 'V'))
                {
                    return Err(invalid(origin, format!("syllable pattern {bad:?} may only use C and V")));
                }
                if lang.min_syllables == 0 || lang.min_syllables > lang.max_syllables {
                    return Err(invalid(origin, "need 1 <= min_syllables <= max_syllables".into()));
                }
                Ok(Culture { def, race_id })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Registry { packs, biomes, biome_ids, reliefs, relief_ids, races, cultures })
    }

    /// The elevation (a percentile) below which hexes are sea: the highest
    /// upper elevation bound of any water biome.
    pub fn sea_level(&self) -> f32 {
        self.biomes
            .iter()
            .filter(|b| b.water && b.elevation.1.is_finite())
            .map(|b| b.elevation.1)
            .fold(None, |acc: Option<f32>, v| Some(acc.map_or(v, |a| a.max(v))))
            .unwrap_or(0.5)
    }

    pub fn races(&self) -> &[RaceDef] {
        &self.races
    }

    pub fn race(&self, id: RaceId) -> &RaceDef {
        &self.races[id.0 as usize]
    }

    pub fn cultures(&self) -> &[Culture] {
        &self.cultures
    }

    pub fn culture(&self, id: CultureId) -> &Culture {
        &self.cultures[id.0 as usize]
    }

    pub fn reliefs(&self) -> &[ReliefDef] {
        &self.reliefs
    }

    pub fn relief(&self, id: ReliefId) -> &ReliefDef {
        &self.reliefs[id.0 as usize]
    }

    pub fn relief_id(&self, id: &str) -> Option<ReliefId> {
        self.relief_ids.get(id).copied()
    }

    pub fn packs(&self) -> &[PackManifest] {
        &self.packs
    }

    pub fn biomes(&self) -> &[BiomeDef] {
        &self.biomes
    }

    pub fn biome(&self, id: BiomeId) -> &BiomeDef {
        &self.biomes[id.0 as usize]
    }

    pub fn biome_id(&self, id: &str) -> Option<BiomeId> {
        self.biome_ids.get(id).copied()
    }
}
