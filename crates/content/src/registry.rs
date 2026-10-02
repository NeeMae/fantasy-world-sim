use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{BiomeDef, ContentError, PackManifest};

/// Dense index of a [`BiomeDef`] within a [`Registry`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BiomeId(pub u16);

/// All loaded content, with string ids resolved to dense indices.
///
/// Def order is the order defs were first defined across packs, so ids are
/// stable for a given set of packs.
#[derive(Clone, Debug)]
pub struct Registry {
    packs: Vec<PackManifest>,
    biomes: Vec<BiomeDef>,
    biome_ids: HashMap<String, BiomeId>,
}

impl Registry {
    pub(crate) fn new(packs: Vec<PackManifest>, biomes: Vec<BiomeDef>) -> Result<Self, ContentError> {
        if biomes.len() > u16::MAX as usize {
            return Err(ContentError::Invalid { origin: "biomes".into(), message: "too many biomes".into() });
        }
        let biome_ids = biomes.iter().enumerate().map(|(i, b)| (b.id.clone(), BiomeId(i as u16))).collect();
        Ok(Registry { packs, biomes, biome_ids })
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
