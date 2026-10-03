use content::BiomeId;
use serde::{Deserialize, Serialize};

use crate::HexId;

/// Something that happened, recorded as data so it can be rendered as prose,
/// filtered into timelines, or exported as lore.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    /// The land around a hex was remade by an outside power.
    TerrainReshaped { center: HexId, radius: u32, to: BiomeId, hexes_changed: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub tick: u64,
    pub kind: EventKind,
}

/// The world's history, in the order it happened.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Chronicle {
    entries: Vec<Entry>,
}

impl Chronicle {
    pub fn record(&mut self, tick: u64, kind: EventKind) {
        self.entries.push(Entry { tick, kind });
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}
