//! Desktop viewer: generates a world, runs it on a background thread and
//! draws it as a pixel-art map.

// Release builds on Windows are a windowed app, without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Bevy systems declare everything they touch as parameters.
#![allow(clippy::too_many_arguments)]

mod camera;
mod map_view;
mod sim_thread;
mod tools;
mod ui;
mod world_setup;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use clap::Parser;

use tools::{Hover, MapModeSetting, PointerOverUi, Selection, ToolState};
use world_setup::{Climate, Content, RegenerateRequest, SCALES, WorldSettings};

#[derive(Parser)]
#[command(version, about = "Fantasy world simulator")]
struct Args {
    /// World seed (random if omitted).
    #[arg(long)]
    seed: Option<u64>,
    /// Detail preset: coarse, normal, fine or "very fine".
    #[arg(long, default_value = "normal")]
    scale: String,
    /// How much of the planet the map shows: 1 is a region, 4+ several continents.
    #[arg(long, default_value_t = 1.0)]
    world_size: f64,
    /// Wrap east-west (a globe). Also switches the climate to pole-to-pole.
    #[arg(long)]
    wrap: bool,
    /// Ring the world with ocean instead of letting land run off the edges.
    #[arg(long)]
    ocean_edges: bool,
    /// Content packs, loaded in order. Later packs override earlier ones.
    /// (Default: the base pack, found in ./packs/base or next to the program.)
    #[arg(long = "pack")]
    packs: Vec<PathBuf>,
}

fn main() -> AppExit {
    let mut args = Args::parse();
    if args.packs.is_empty() {
        args.packs.push(content::default_base_pack());
    }
    let registry = match content::load_packs(&args.packs) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            eprintln!("error: failed to load content: {e}");
            return AppExit::error();
        }
    };
    let Some(scale) = SCALES.iter().position(|(name, _)| name.eq_ignore_ascii_case(&args.scale)) else {
        eprintln!("error: unknown scale {:?}; expected coarse, normal, fine or \"very fine\"", args.scale);
        return AppExit::error();
    };
    let mut settings = WorldSettings {
        scale,
        world_size: args.world_size,
        wrap: args.wrap,
        climate: if args.wrap { Climate::Globe } else { Climate::Regional },
        edges: if args.ocean_edges { worldgen::EdgeStyle::Ocean } else { worldgen::EdgeStyle::Open },
        ..Default::default()
    };
    if let Some(seed) = args.seed {
        settings.seed = seed.to_string();
    }

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
        .insert_resource(Content(registry))
        .insert_resource(settings)
        .init_resource::<Selection>()
        .init_resource::<Hover>()
        .init_resource::<ToolState>()
        .init_resource::<PointerOverUi>()
        .init_resource::<MapModeSetting>()
        .add_message::<RegenerateRequest>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                world_setup::regenerate,
                (camera::controls, ui::hotkeys),
                tools::update_hover,
                tools::use_tool,
                map_view::apply_map_mode,
                map_view::redraw_changes,
                map_view::update_outlines,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui::panels)
        .run()
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    content: Res<Content>,
    settings: Res<WorldSettings>,
    windows: Query<&Window>,
    mut exit: MessageWriter<AppExit>,
) {
    match world_setup::start_world(&mut commands, &mut images, &content, &settings, default()) {
        Ok(map_size) => {
            let window = windows.single().map_or(Vec2::new(1600.0, 900.0), Window::size);
            camera::spawn(&mut commands, map_size, window);
        }
        Err(e) => {
            error!("world generation failed: {e}");
            exit.write(AppExit::error());
        }
    }
}
