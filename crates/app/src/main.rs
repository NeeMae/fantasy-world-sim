//! Desktop viewer: generates a world, runs it on a background thread and
//! draws it as a pixel-art map.

// Release builds on Windows are a windowed app, without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Bevy systems declare everything they touch as parameters.
#![allow(clippy::too_many_arguments)]

mod menu;
mod planet_view;
mod saves;
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

use tools::{Hover, MapModeSetting, Selection, ToolState};
use world_setup::{Content, DETAILS, LoadRequest, RegenerateRequest, WorldSettings};

#[derive(Parser)]
#[command(version, about = "Fantasy world simulator")]
struct Args {
    /// World seed (random if omitted).
    #[arg(long)]
    seed: Option<u64>,
    /// Detail preset: coarse, normal, fine, "very fine" or extreme.
    #[arg(long, default_value = "normal")]
    detail: String,
    /// Size of the planet: bigger planets hold more, smaller continents.
    #[arg(long, default_value_t = 4.0)]
    world_size: f64,
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
    let Some(detail) = DETAILS.iter().position(|(name, _)| name.eq_ignore_ascii_case(&args.detail)) else {
        eprintln!(
            "error: unknown detail {:?}; expected coarse, normal, fine, \"very fine\" or extreme",
            args.detail
        );
        return AppExit::error();
    };
    let mut settings = WorldSettings { detail, world_size: args.world_size, ..Default::default() };
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
        .init_resource::<MapModeSetting>()
        .add_message::<RegenerateRequest>()
        .add_message::<LoadRequest>()
        .init_resource::<saves::SaveState>()
        .init_resource::<menu::Menu>()
        .init_resource::<planet_view::PlanetView>()
        .add_systems(Startup, setup)
        .add_systems(Update, (world_setup::regenerate, world_setup::load, saves::poll, ui::hotkeys).chain())
        .add_systems(EguiPrimaryContextPass, ui::panels)
        .run()
}

fn setup(
    mut commands: Commands,
    content: Res<Content>,
    settings: Res<WorldSettings>,
    mut exit: MessageWriter<AppExit>,
) {
    // egui draws through a camera; the planet itself is an egui image.
    commands.spawn(Camera2d);
    if let Err(e) = world_setup::start_world(&mut commands, &content, &settings) {
        error!("world generation failed: {e}");
        exit.write(AppExit::error());
    }
}
