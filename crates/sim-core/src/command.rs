use content::BiomeId;
use serde::{Deserialize, Serialize};

use crate::HexId;

/// A request to change the world, applied at the start of the next tick.
///
/// Everything that changes the world from outside the systems goes through
/// here: god powers, a player ruler's orders and AI decisions alike. A save
/// file can therefore be just the seed, content and the command log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Reshape a hex into another biome.
    SetBiome { hex: HexId, biome: BiomeId },
}
