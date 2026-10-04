//! Generates a world and runs it with no window.
//!
//! ```text
//! sim-cli --seed 42 --years 100 --png world.png
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, ValueEnum};
use map_raster::{HexLayout, MapMode, RenderOptions};
use sim_core::{Simulation, Wrap};
use worldgen::WorldGenParams;

#[derive(Parser)]
#[command(version, about = "Generate a world and simulate its history headlessly")]
struct Args {
    /// World seed.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Map width in hexes.
    #[arg(long, default_value_t = 512)]
    width: u32,
    /// Map height in hexes.
    #[arg(long, default_value_t = 320)]
    height: u32,
    /// Map shape.
    #[arg(long, value_enum, default_value_t = Shape::Flat)]
    shape: Shape,
    /// What lies at the map's edges.
    #[arg(long, value_enum, default_value_t = Edges::Open)]
    edges: Edges,
    /// How much of the planet the map shows: 1 is a region, 4+ several continents.
    #[arg(long, default_value_t = 1.0)]
    world_size: f64,
    /// How turbulent history is, 0 (calm) to 1 (chaotic).
    #[arg(long, default_value_t = 0.5)]
    volatility: f32,
    /// Size of landmasses: larger gives fewer, bigger continents.
    #[arg(long, default_value_t = 1.0)]
    continent_size: f64,
    /// Climate bands across the map (default: regional, or globe for a cylinder).
    #[arg(long, value_enum)]
    climate: Option<Climate>,
    /// Ticks (months) to simulate. Added to `--years`.
    #[arg(long, default_value_t = 0)]
    ticks: u64,
    /// Years to simulate. Added to `--ticks`.
    #[arg(long, default_value_t = 0)]
    years: u64,
    /// Content packs, loaded in order. Later packs override earlier ones.
    /// (Default: the base pack, found in ./packs/base or next to the program.)
    #[arg(long = "pack")]
    packs: Vec<PathBuf>,
    /// Write a pixel-art image of the final map.
    #[arg(long)]
    png: Option<PathBuf>,
    /// Hex radius in pixels for `--png`.
    #[arg(long, default_value_t = 4.0)]
    hex_size: f32,
    /// Draw hex outlines in `--png`.
    #[arg(long)]
    grid: bool,
    /// Print this many sample person and place names for each culture.
    #[arg(long, default_value_t = 0)]
    names: usize,
    /// What `--png` shows.
    #[arg(long, value_enum, default_value_t = View::Terrain)]
    view: View,
}

#[derive(Clone, Copy, ValueEnum)]
enum View {
    /// Biomes and relief.
    Terrain,
    /// Height.
    Elevation,
    /// Tectonic plates and their boundaries.
    Plates,
    /// Rainfall.
    Rainfall,
    /// Temperature.
    Temperature,
    /// Fertility.
    Fertility,
}

#[derive(Clone, Copy, ValueEnum)]
enum Climate {
    /// Cool north to warm south.
    Regional,
    /// Pole to pole with the equator across the middle.
    Globe,
}

#[derive(Clone, Copy, ValueEnum)]
enum Edges {
    /// Land and sea run off the edges, as if the map were part of a larger world.
    Open,
    /// The world is ringed by ocean.
    Ocean,
}

#[derive(Clone, Copy, ValueEnum)]
enum Shape {
    /// A bordered rectangle.
    Flat,
    /// East and west edges join.
    Cylinder,
}

fn main() -> ExitCode {
    let mut args = Args::parse();
    if args.packs.is_empty() {
        args.packs.push(content::default_base_pack());
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let registry = Arc::new(content::load_packs(&args.packs)?);
    let packs: Vec<_> = registry.packs().iter().map(|p| p.id.as_str()).collect();
    println!("content: packs [{}], {} biomes", packs.join(", "), registry.biomes().len());

    if args.names > 0 {
        print_names(&registry, args.seed, args.names);
    }

    let params = WorldGenParams {
        seed: args.seed,
        width: args.width,
        height: args.height,
        wrap: match args.shape {
            Shape::Flat => Wrap::None,
            Shape::Cylinder => Wrap::X,
        },
        edges: match args.edges {
            Edges::Open => worldgen::EdgeStyle::Open,
            Edges::Ocean => worldgen::EdgeStyle::Ocean,
        },
        world_size: args.world_size,
        continent_scale: args.continent_size,
        rules: sim_core::WorldRules { volatility: args.volatility.clamp(0.0, 1.0) },
        latitudes: match (args.climate, args.shape) {
            (Some(Climate::Globe), _) | (None, Shape::Cylinder) => worldgen::Latitudes::Globe,
            (Some(Climate::Regional), _) | (None, Shape::Flat) => worldgen::Latitudes::default(),
        },
    };
    let started = Instant::now();
    let world = worldgen::generate(&params, &registry)?;
    println!(
        "worldgen: {}x{} hexes ({}) in {:.0?}",
        args.width,
        args.height,
        world.topology.len(),
        started.elapsed()
    );
    print_biomes(&world, &registry);

    let mut sim = Simulation::new(world, registry.clone());
    let ticks = args.ticks + args.years * sim_core::time::MONTHS_PER_YEAR;
    let started = Instant::now();
    sim.run(ticks);
    let elapsed = started.elapsed();
    println!(
        "simulated {ticks} ticks in {elapsed:.2?} ({:.0} ticks/s), now {}",
        ticks as f64 / elapsed.as_secs_f64().max(1e-9),
        sim.world().date()
    );
    println!("chronicle: {} entries", sim.chronicle().entries().len());
    println!("state hash: {:016x}", sim.world().state_hash());

    if let Some(path) = args.png {
        let mode = match args.view {
            View::Terrain => MapMode::Terrain,
            View::Elevation => MapMode::Elevation,
            View::Plates => MapMode::Plates,
            View::Rainfall => MapMode::Rainfall,
            View::Temperature => MapMode::Temperature,
            View::Fertility => MapMode::Fertility,
        };
        let opts = RenderOptions {
            layout: HexLayout { size: args.hex_size },
            mode,
            grid: args.grid,
            ..Default::default()
        };
        let img = map_raster::render(sim.world(), &registry, &opts);
        image::save_buffer(&path, &img.rgba, img.width, img.height, image::ExtendedColorType::Rgba8)?;
        println!("wrote {} ({}x{} px)", path.display(), img.width, img.height);
    }
    Ok(())
}

fn print_names(registry: &content::Registry, seed: u64, count: usize) {
    for (i, culture) in registry.cultures().iter().enumerate() {
        let mut rng = sim_core::rng::stream(seed, 0, i as u64, sim_core::rng::purpose::NAMES);
        let mut sample = |kind| {
            (0..count).map(|_| names::generate(&culture.def.language, kind, &mut rng)).collect::<Vec<_>>()
        };
        let (people, places) = (sample(names::NameKind::Person), sample(names::NameKind::Place));
        println!("{} ({}):", culture.def.name, registry.race(culture.race_id).name);
        println!("  people: {}", people.join(", "));
        println!("  places: {}", places.join(", "));
    }
}

fn print_biomes(world: &sim_core::World, registry: &content::Registry) {
    let mut counts = vec![0usize; registry.biomes().len()];
    for b in &world.terrain.biome {
        counts[b.0 as usize] += 1;
    }
    let total = world.terrain.len() as f64;
    let mut rows: Vec<_> = registry.biomes().iter().zip(counts).filter(|(_, n)| *n > 0).collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
    for (biome, n) in rows {
        println!("  {:<12} {:>6.2}%", biome.name, n as f64 / total * 100.0);
    }
    let land = world.terrain.biome.iter().filter(|b| !registry.biome(**b).water).count().max(1);
    for (i, relief) in registry.reliefs().iter().enumerate() {
        let n = world
            .terrain
            .relief
            .iter()
            .zip(&world.terrain.biome)
            .filter(|(r, b)| r.0 as usize == i && !registry.biome(**b).water)
            .count();
        println!("  relief {:<10} {:>6.2}% of land", relief.name, n as f64 / land as f64 * 100.0);
    }
    let rivers = (0..world.terrain.len())
        .filter(|&i| world.terrain.river(i) > 0.0 && !registry.biome(world.terrain.biome[i]).water)
        .count();
    println!("  rivers       {:>6.2}% of land", rivers as f64 / land as f64 * 100.0);
    let continental = world.geology.plates.iter().filter(|p| p.continental).count();
    println!("  plates: {} ({continental} continental)", world.geology.plates.len());
}
