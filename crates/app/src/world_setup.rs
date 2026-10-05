//! Creating (and re-creating) the world the app shows.

use std::sync::Arc;

use bevy::prelude::*;
use content::Registry;
use sim_core::{Simulation, Wrap};
use worldgen::{EdgeStyle, Latitudes, WorldGenParams};

use crate::camera::{self, MapCamera};
use crate::map_view::{self, MapEntity};
use crate::sim_thread::SimThread;
use crate::tools::{Hover, MapModeSetting, Selection};

/// Detail presets: how many hex rows tall the map is. The same world is
/// drawn at every scale, just with finer or coarser hexes.
pub const SCALES: [(&str, u32); 4] = [("Coarse", 160), ("Normal", 320), ("Fine", 480), ("Very fine", 640)];

/// Canvas shapes, as on-screen width : height.
pub const ASPECTS: [(&str, f64); 6] =
    [("1:1", 1.0), ("4:3", 4.0 / 3.0), ("3:2", 1.5), ("16:9", 16.0 / 9.0), ("2:1", 2.0), ("3:1", 3.0)];

/// Largest custom canvas edge, in hexes.
pub const MAX_CANVAS: u32 = 4096;
/// Above this many hexes, warn that the world may be slow.
pub const LARGE_WORLD: u64 = 1_000_000;

/// Climate bands across the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Climate {
    /// Cool north to warm south, like a regional map.
    Regional,
    /// Pole to pole with the equator across the middle.
    Globe,
}

/// Loaded content, shared by every world generated this session.
#[derive(Resource)]
pub struct Content(pub Arc<Registry>);

/// The world-generation form in the UI. Saved with each world (as RON), so
/// loading a world restores the settings that made it.
#[derive(Resource, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WorldSettings {
    pub seed: String,
    /// Index into [`SCALES`].
    pub scale: usize,
    /// Index into [`ASPECTS`].
    pub aspect: usize,
    /// Explicit width × height in hexes, overriding scale and aspect.
    pub custom: Option<(u32, u32)>,
    pub world_size: f64,
    pub continent_size: f64,
    pub edges: EdgeStyle,
    pub wrap: bool,
    pub climate: Climate,
    pub volatility: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        WorldSettings {
            seed: random_seed().to_string(),
            scale: 1,
            aspect: 2,
            custom: None,
            world_size: 1.0,
            continent_size: 1.0,
            edges: EdgeStyle::Open,
            wrap: false,
            climate: Climate::Regional,
            volatility: 0.5,
        }
    }
}

impl WorldSettings {
    /// Canvas size in hexes. Widths are kept even so wrapping always works.
    pub fn dimensions(&self) -> (u32, u32) {
        let (w, h) = self.custom.unwrap_or_else(|| {
            let rows = SCALES[self.scale].1;
            // A flat-topped hex grid is ~0.866 as wide on screen as its
            // column count suggests relative to its rows.
            let cols = (ASPECTS[self.aspect].1 * rows as f64 / 0.866).round() as u32;
            (cols, rows)
        });
        ((w.clamp(2, MAX_CANVAS) + 1) & !1, h.clamp(2, MAX_CANVAS))
    }

    pub fn params(&self) -> Result<WorldGenParams, String> {
        let seed = self.seed.trim().parse::<u64>().map_err(|_| format!("invalid seed {:?}", self.seed))?;
        let (width, height) = self.dimensions();
        Ok(WorldGenParams {
            seed,
            width,
            height,
            wrap: if self.wrap { Wrap::X } else { Wrap::None },
            edges: self.edges,
            world_size: self.world_size,
            continent_scale: self.continent_size,
            latitudes: match self.climate {
                Climate::Regional => Latitudes::default(),
                Climate::Globe => Latitudes::Globe,
            },
            rules: sim_core::WorldRules { volatility: self.volatility },
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
/// Returns the map size in pixels.
pub fn start_world(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    content: &Content,
    settings: &WorldSettings,
    mode: map_raster::MapMode,
) -> Result<Vec2, String> {
    let params = settings.params()?;
    let world = worldgen::generate(&params, &content.0).map_err(|e| e.to_string())?;
    info!(
        "generated {}×{} world with seed {} (world size {}, wrap {})",
        params.width, params.height, params.seed, params.world_size, settings.wrap
    );
    Ok(show_world(commands, images, Simulation::new(world, content.0.clone()), mode))
}

/// Draws a simulation's world and starts running it, replacing any running
/// world. Returns the map size in pixels.
pub fn show_world(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    sim: Simulation,
    mode: map_raster::MapMode,
) -> Vec2 {
    let size = map_view::spawn(commands, images, sim.registry().clone(), sim.world(), mode);
    // Replacing the resource drops the old handle, which stops the old thread.
    commands.insert_resource(SimThread::spawn(sim));
    size
}

pub fn regenerate(
    mut requests: MessageReader<RegenerateRequest>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    content: Res<Content>,
    settings: Res<WorldSettings>,
    mode: Res<MapModeSetting>,
    old: Query<Entity, With<MapEntity>>,
    windows: Query<&Window>,
    mut camera: Query<(&Camera, &mut Transform, &mut Projection, &mut MapCamera)>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
    mut saves: ResMut<crate::saves::SaveState>,
) {
    if requests.read().count() == 0 {
        return;
    }
    match start_world(&mut commands, &mut images, &content, &settings, mode.0) {
        Ok(size) => {
            // A new world: don't let Ctrl+S overwrite the last one's save.
            saves.name.clear();
            saves.status = None;
            for entity in &old {
                commands.entity(entity).despawn();
            }
            selection.0 = None;
            hover.0 = None;
            let window = windows.single().map_or(Vec2::new(1600.0, 900.0), Window::size);
            camera::refit(&mut camera, size, window);
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
    mut images: ResMut<Assets<Image>>,
    content: Res<Content>,
    mut settings: ResMut<WorldSettings>,
    mut saves: ResMut<crate::saves::SaveState>,
    mode: Res<MapModeSetting>,
    old: Query<Entity, With<MapEntity>>,
    windows: Query<&Window>,
    mut camera: Query<(&Camera, &mut Transform, &mut Projection, &mut MapCamera)>,
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
    let size = show_world(&mut commands, &mut images, sim, mode.0);
    for entity in &old {
        commands.entity(entity).despawn();
    }
    selection.0 = None;
    hover.0 = None;
    let window = windows.single().map_or(Vec2::new(1600.0, 900.0), Window::size);
    camera::refit(&mut camera, size, window);
}
