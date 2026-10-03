use content::BiomeId;
use serde::{Deserialize, Serialize};

use crate::HexId;

/// A request to change the world from outside its own systems.
///
/// Everything that changes the world from outside goes through here: god
/// powers, a player ruler's orders and AI decisions alike. Commands are
/// applied *between* ticks, in submission order, so a save file can be just
/// the seed, content and the command log (each tagged with its tick).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Reshape every hex within `radius` of `center` into `biome`.
    Reshape { center: HexId, radius: u32, biome: BiomeId },
}
