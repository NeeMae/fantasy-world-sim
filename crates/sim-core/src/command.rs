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
    ///
    /// Consecutive reshapes with the same `edit` (e.g. one brush stroke)
    /// form a single step for [`Command::Undo`].
    Reshape { center: HexId, radius: u32, biome: BiomeId, edit: EditId },
    /// Revert the most recent edit still in the journal.
    Undo,
    /// Re-apply the most recently undone edit, if nothing new happened since.
    Redo,
}

/// Groups reshapes into one undoable step. Chosen by whoever submits them;
/// it only needs to differ from the previous edit's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditId(pub u64);
