//! Creating (and re-creating) the world the app shows.

use std::sync::Arc;

use bevy::prelude::*;
use content::Registry;
use sim_core::Simulation;
use worldgen::WorldGenParams;

use crate::sim_thread::SimThread;
use crate::tools::{Hover, Selection};

/// Detail presets: the planet's grid frequency (it has 10 × f² + 2 hexes).
/// The same world is drawn at every detail, just with finer or coarser
/// hexes.
pub const DETAILS: [(&str, u32); 5] =
    [("Coarse", 80), ("Normal", 130), ("Fine", 190), ("Very fine", 260), ("Extreme", 360)];

/// Above this many hexes, warn that the world may be slow.
pub const LARGE_WORLD: u64 = 1_000_000;

/// Loaded content, shared by every world generated this session.
#[derive(Resource)]
pub struct Content(pub Arc<Registry>);

/// The world-generation form in the UI. Saved with each world (as RON), so
/// loading a world restores the settings that made it.
#[derive(Resource, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WorldSettings {
    pub seed: String,
    /// Index into [`DETAILS`].
    pub detail: usize,
    /// Size of the planet: bigger planets hold more, smaller continents.
    pub world_size: f64,
    pub continent_size: f64,
    pub volatility: f32,
    /// Share of the map under sea.
    pub ocean: f64,
    /// Temperature offset, about -0.3 (ice age) to 0.3 (hothouse).
    pub temperature: f64,
    /// Rainfall multiplier.
    pub rainfall: f64,
    /// Mountain-building multiplier.
    pub mountains: f64,
    /// How deeply basins sink into continents, 0..1.
    pub inland_seas: f64,
    /// Erosion strength, 0..2.
    pub erosion: f64,
    /// How many streams count as rivers, 0..1.
    pub rivers: f64,
    /// Lake size, 0..2.
    pub lakes: f64,
}

impl Default for WorldSettings {
    fn default() -> Self {
        WorldSettings {
            seed: random_seed().to_string(),
            detail: 1,
            world_size: 4.0,
            continent_size: 1.0,
            volatility: 0.5,
            ocean: 0.55,
            temperature: 0.0,
            rainfall: 1.0,
            mountains: 1.0,
            inland_seas: 0.3,
            erosion: 1.0,
            rivers: 0.5,
            lakes: 1.0,
        }
    }
}

impl WorldSettings {
    pub fn frequency(&self) -> u32 {
        DETAILS[self.detail.min(DETAILS.len() - 1)].1
    }

    /// How many hexes the planet will have.
    pub fn hexes(&self) -> u64 {
        let f = self.frequency() as u64;
        10 * f * f + 2
    }

    pub fn params(&self) -> Result<WorldGenParams, String> {
        let seed = self.seed.trim().parse::<u64>().map_err(|_| format!("invalid seed {:?}", self.seed))?;
        Ok(WorldGenParams {
            seed,
            frequency: self.frequency(),
            world_size: self.world_size,
            continent_scale: self.continent_size,
            rules: sim_core::WorldRules { volatility: self.volatility },
            ocean: Some(self.ocean),
            temperature: self.temperature,
            rainfall: self.rainfall,
            mountains: self.mountains,
            inland_seas: self.inland_seas,
            erosion: self.erosion,
            rivers: self.rivers,
            lakes: self.lakes,
        })
    }
}

#[derive(Message)]
pub struct RegenerateRequest;

pub fn random_seed() -> u64 {
    // Clock-derived: varied enough for picking worlds, and not part of the
    // simulation, so it doesn't affect determinism.
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    sim_core::rng::mix(nanos, 0, 0, 0) % 1_000_000_000
}

/// Generates a world from the current settings and starts simulating it.
pub fn start_world(
    commands: &mut Commands,
    content: &Content,
    settings: &WorldSettings,
) -> Result<(), String> {
    let params = settings.params()?;
    let started = std::time::Instant::now();
    let world = worldgen::generate(&params, &content.0).map_err(|e| e.to_string())?;
    info!(
        "generated a planet of {} hexes with seed {} (world size {}) in {:.1?}",
        world.topology.len(),
        params.seed,
        params.world_size,
        started.elapsed()
    );
    show_world(commands, Simulation::new(world, content.0.clone()));
    Ok(())
}

/// Starts running a simulation's world, replacing any running world. The
/// view redraws when it sees the new world.
pub fn show_world(commands: &mut Commands, sim: Simulation) {
    // Replacing the resource drops the old handle, which stops the old thread.
    commands.insert_resource(SimThread::spawn(sim));
}

pub fn regenerate(
    mut requests: MessageReader<RegenerateRequest>,
    mut commands: Commands,
    content: Res<Content>,
    settings: Res<WorldSettings>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
    mut saves: ResMut<crate::saves::SaveState>,
) {
    if requests.read().count() == 0 {
        return;
    }
    match start_world(&mut commands, &content, &settings) {
        Ok(()) => {
            // A new world: don't let Ctrl+S overwrite the last one's save.
            saves.name.clear();
            saves.status = None;
            selection.0 = None;
            hover.0 = None;
        }
        Err(e) => error!("could not generate world: {e}"),
    }
}

/// Asks for a saved world to replace the current one.
#[derive(Message)]
pub struct LoadRequest(pub std::path::PathBuf);

pub fn load(
    mut requests: MessageReader<LoadRequest>,
    mut commands: Commands,
    content: Res<Content>,
    mut settings: ResMut<WorldSettings>,
    mut saves: ResMut<crate::saves::SaveState>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
) {
    let Some(LoadRequest(path)) = requests.read().last() else {
        return;
    };
    let data = match sim_core::save::load(path, &content.0) {
        Ok(data) => data,
        Err(e) => {
            saves.report(format!("Could not load {}: {e}", crate::saves::display_name(path)), false);
            return;
        }
    };
    // Settings from an older or newer build may not parse; keep the form.
    if let Ok(saved) = ron::from_str::<WorldSettings>(&data.settings) {
        *settings = saved;
    }
    let name = crate::saves::display_name(path);
    saves.report(format!("Loaded {name}"), true);
    saves.name = name;
    let sim = Simulation::from_save(data, content.0.clone());
    saves.mark_saved(sim.world());
    show_world(&mut commands, sim);
    selection.0 = None;
    hover.0 = None;
}
