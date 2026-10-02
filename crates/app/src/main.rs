//! Desktop viewer: generates a world, runs it on a background thread and
//! draws it as a pixel-art map.

mod camera;
mod map_view;
mod sim_thread;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use clap::Parser;
use sim_core::{Simulation, Wrap};
use worldgen::WorldGenParams;

use map_view::Selection;
use sim_thread::SimThread;

#[derive(Parser, Resource, Clone)]
#[command(version, about = "Fantasy world simulator")]
struct Args {
    /// World seed (random if omitted).
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long, default_value_t = 512)]
    width: u32,
    #[arg(long, default_value_t = 320)]
    height: u32,
    /// Content packs, loaded in order. Later packs override earlier ones.
    #[arg(long = "pack", default_value = "packs/base")]
    packs: Vec<PathBuf>,
}

fn main() -> AppExit {
    let args = Args::parse();
    App::new()
        .add_plugins(DefaultPlugins.set(ImagePlugin::default_nearest()).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Fantasy World Sim".into(),
                resolution: WindowResolution::new(1600, 900),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb_u8(12, 14, 22)))
        .insert_resource(args)
        .init_resource::<Selection>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            ((camera::controls, ui::hotkeys, ui::pick), (map_view::redraw, map_view::update_marker)).chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui::panels)
        .run()
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    args: Res<Args>,
    windows: Query<&Window>,
    mut exit: MessageWriter<AppExit>,
) {
    let registry = match content::load_packs(&args.packs) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            error!("failed to load content: {e}");
            exit.write(AppExit::error());
            return;
        }
    };
    let params = WorldGenParams {
        seed: args.seed.unwrap_or_else(rand_seed),
        width: args.width,
        height: args.height,
        wrap: Wrap::None,
        ..default()
    };
    let world = match worldgen::generate(&params, &registry) {
        Ok(w) => w,
        Err(e) => {
            error!("world generation failed: {e}");
            exit.write(AppExit::error());
            return;
        }
    };
    info!("generated world with seed {}", params.seed);

    let map_size = map_view::spawn(&mut commands, &mut images, registry.clone(), &world);
    let window_size = windows.single().map_or(Vec2::new(1600.0, 900.0), Window::size);
    camera::spawn(&mut commands, map_size, window_size);
    commands.insert_resource(SimThread::spawn(Simulation::new(world, registry)));
}

/// A seed from the clock, for when the user doesn't ask for one.
fn rand_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}
