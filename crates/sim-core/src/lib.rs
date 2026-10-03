//! The simulation core: world state, commands and the tick loop.
//!
//! This crate must never depend on rendering, windowing or wall-clock time.
//! Given the same seed, content and command log it produces the same history.

pub mod chronicle;
pub mod command;
pub mod hex;
pub mod rng;
pub mod sim;
pub mod time;
pub mod world;

pub use chronicle::{Chronicle, Entry, EventKind};
pub use command::{Command, EditId, Paint};
pub use hex::{Axial, HexId, Topology, Wrap, round_axial};
pub use sim::Simulation;
pub use time::Date;
pub use world::{Geology, Plate, Terrain, World};
