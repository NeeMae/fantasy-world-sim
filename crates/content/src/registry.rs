use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{BiomeDef, ContentError, PackManifest, ReliefDef};

/// Dense index of a [`BiomeDef`] within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BiomeId(pub u16);

/// Dense index of a [`ReliefDef`] within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReliefId(pub u8);

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
}

impl Registry {
    pub(crate) fn new(
        packs: Vec<PackManifest>,
        biomes: Vec<BiomeDef>,
        mut reliefs: Vec<ReliefDef>,
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
        let biome_ids = biomes.iter().enumerate().map(|(i, b)| (b.id.clone(), BiomeId(i as u16))).collect();
        let relief_ids = reliefs.iter().enumerate().map(|(i, r)| (r.id.clone(), ReliefId(i as u8))).collect();
        Ok(Registry { packs, biomes, biome_ids, reliefs, relief_ids })
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
