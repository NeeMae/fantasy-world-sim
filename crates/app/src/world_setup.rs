//! Creating (and re-creating) the world the app shows.

use std::sync::Arc;

use bevy::prelude::*;
use content::Registry;
use sim_core::{Simulation, Wrap};
use worldgen::{EdgeStyle, WorldGenParams};

use crate::camera::{self, MapCamera};
use crate::map_view::{self, MapEntity};
use crate::sim_thread::SimThread;
use crate::tools::{Hover, Selection};

/// Map size presets: name, width, height (in hexes).
pub const SIZES: [(&str, u32, u32); 4] =
    [("Small", 256, 160), ("Medium", 512, 320), ("Large", 768, 480), ("Huge", 1024, 640)];

/// Loaded content, shared by every world generated this session.
#[derive(Resource)]
pub struct Content(pub Arc<Registry>);

/// The world-generation form in the UI.
#[derive(Resource)]
pub struct WorldSettings {
    pub seed: String,
    /// Index into [`SIZES`].
    pub size: usize,
    pub edges: EdgeStyle,
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
) -> Result<Vec2, String> {
    let seed =
        settings.seed.trim().parse::<u64>().map_err(|_| format!("invalid seed {:?}", settings.seed))?;
    let (_, width, height) = SIZES[settings.size];
    let params = WorldGenParams { seed, width, height, wrap: Wrap::None, edges: settings.edges, ..default() };
    let world = worldgen::generate(&params, &content.0).map_err(|e| e.to_string())?;
    info!("generated {width}×{height} world with seed {seed}");
    let size = map_view::spawn(commands, images, content.0.clone(), &world);
    // Replacing the resource drops the old handle, which stops the old thread.
    commands.insert_resource(SimThread::spawn(Simulation::new(world, content.0.clone())));
    Ok(size)
}

pub fn regenerate(
    mut requests: MessageReader<RegenerateRequest>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    content: Res<Content>,
    settings: Res<WorldSettings>,
    old: Query<Entity, With<MapEntity>>,
    windows: Query<&Window>,
    mut camera: Query<(&Camera, &mut Transform, &mut Projection, &mut MapCamera)>,
    mut selection: ResMut<Selection>,
    mut hover: ResMut<Hover>,
) {
    if requests.read().count() == 0 {
        return;
    }
    match start_world(&mut commands, &mut images, &content, &settings) {
        Ok(size) => {
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
