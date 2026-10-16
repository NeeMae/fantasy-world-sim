//! The simulation core: world state, commands and the tick loop.
//!
//! This crate must never depend on rendering, windowing or wall-clock time.
//! Given the same seed, content and command log it produces the same history.

pub mod chronicle;
pub mod command;
pub mod hex;
pub mod rng;
pub mod save;
pub mod sim;
pub mod time;
pub mod world;

pub use chronicle::{Chronicle, Entry, EventKind};
pub use command::{Command, EditId, Paint};
pub use hex::{HexId, MAX_FREQUENCY, Topology, cross, dot, from_lat_lon, lat_lon, normalize, tangent_basis};
pub use save::{SaveData, SaveError};
pub use sim::Simulation;
pub use time::Date;
pub use world::{Geology, NO_DRAIN, Plate, RIVER_DISCHARGE, Terrain, World, WorldRules};
