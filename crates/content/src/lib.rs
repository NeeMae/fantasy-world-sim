//! Content definitions ("defs") and the content-pack loader.
//!
//! Modding follows the RimWorld model: all game content is data. A *pack* is a
//! directory containing a `pack.ron` manifest and any number of `.ron` files
//! under `defs/`. Packs are layered in load order; the base game is itself
//! just a pack.
//!
//! Each def file is a struct whose fields are lists of defs by kind:
//!
//! ```ron
//! (
//!     biomes: [
//!         (id: "land_base", abstract: true, water: false),
//!         (id: "grassland", parent: "land_base", name: "Grassland", color: "#6a9a3a"),
//!     ],
//! )
//! ```
//!
//! * `abstract: true` defs exist only to be inherited from.
//! * `parent: "<id>"` inherits every field the child doesn't set.
//! * Redefining an existing id (e.g. in a later pack) *merges* over the
//!   earlier definition, so a mod can change a single field.

mod color;
mod defs;
mod loader;
mod registry;

pub use color::Rgb;
pub use defs::{BiomeDef, Glyph, Range, ReliefDef, Site};
pub use loader::{ContentError, PackManifest, default_base_pack, load_packs, load_str};
pub use registry::{BiomeId, Registry, ReliefId};
