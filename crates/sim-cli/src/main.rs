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
use content::Registry;
use map_raster::{MapMode, Projection, RenderOptions};
use sim_core::{HexId, Simulation, World};
use worldgen::WorldGenParams;

#[derive(Parser)]
#[command(version, about = "Generate a world and simulate its history headlessly")]
struct Args {
    /// World seed.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Grid frequency: the planet has 10 × detail² + 2 cells (130 is about
    /// 170 000). The same world comes out at any detail.
    #[arg(long, default_value_t = 130)]
    detail: u32,
    /// Size of the planet (pole to pole in noise space): bigger planets hold
    /// more, smaller continents.
    #[arg(long, default_value_t = 4.0)]
    world_size: f64,
    /// How turbulent history is, 0 (calm) to 1 (chaotic).
    #[arg(long, default_value_t = 0.5)]
    volatility: f32,
    /// Size of landmasses: larger gives fewer, bigger continents.
    #[arg(long, default_value_t = 1.0)]
    continent_size: f64,
    /// Share of the map under sea, 0..1 (default: the content's sea level).
    #[arg(long)]
    ocean: Option<f64>,
    /// Temperature offset, about -0.3 (ice age) to 0.3 (hothouse).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    temperature: f64,
    /// Rainfall multiplier: above 1 wetter, below 1 drier.
    #[arg(long, default_value_t = 1.0)]
    rainfall: f64,
    /// How deeply basins sink into continents, flooding as inland seas, 0..1.
    #[arg(long, default_value_t = 0.3)]
    inland_seas: f64,
    /// How many streams count as rivers, 0 (great rivers only) to 1 (many).
    #[arg(long, default_value_t = 0.5)]
    rivers: f64,
    /// Lake size: 0 none, 1 normal, 2 big.
    #[arg(long, default_value_t = 1.0)]
    lakes: f64,
    /// Erosion: 0 none, 1 normal, 2 ancient, deeply carved land.
    #[arg(long, default_value_t = 1.0)]
    erosion: f64,
    /// Mountain-building multiplier: 0 worn flat, 2 jagged.
    #[arg(long, default_value_t = 1.0)]
    mountains: f64,
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
    /// Start from a saved world instead of generating one (world options
    /// are then ignored).
    #[arg(long)]
    load: Option<PathBuf>,
    /// Save the world after simulating.
    #[arg(long)]
    save: Option<PathBuf>,
    /// Write a pixel-art image of the final map.
    #[arg(long)]
    png: Option<PathBuf>,
    /// Width of `--png` in pixels.
    #[arg(long, default_value_t = 1600)]
    png_width: u32,
    /// How `--png` lays the planet out: a flat map of the whole planet, or
    /// the globe seen from above `--lat`/`--lon`.
    #[arg(long, value_enum, default_value_t = ProjectionArg::Flat)]
    projection: ProjectionArg,
    /// Latitude and longitude (degrees) the globe view looks at.
    #[arg(long, default_value_t = 20.0, allow_hyphen_values = true)]
    lat: f64,
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    lon: f64,
    /// Globe zoom: 1 shows the whole disc.
    #[arg(long, default_value_t = 1.0)]
    zoom: f64,
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
    /// Ocean currents.
    Currents,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProjectionArg {
    /// The whole planet, longitude across and latitude down (2:1).
    Flat,
    /// The globe, as seen from space.
    Globe,
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
        frequency: args.detail,
        world_size: args.world_size,
        continent_scale: args.continent_size,
        ocean: args.ocean,
        temperature: args.temperature,
        rainfall: args.rainfall,
        mountains: args.mountains,
        inland_seas: args.inland_seas,
        erosion: args.erosion,
        rivers: args.rivers,
        lakes: args.lakes,
        rules: sim_core::WorldRules { volatility: args.volatility.clamp(0.0, 1.0) },
    };
    let started = Instant::now();
    let mut sim = if let Some(path) = &args.load {
        let sim = Simulation::from_save(sim_core::save::load(path, &registry)?, registry.clone());
        let t = &sim.world().topology;
        println!("loaded {} ({} cells, {})", path.display(), t.len(), sim.world().date());
        sim
    } else {
        let world = worldgen::generate(&params, &registry)?;
        println!(
            "worldgen: detail {} ({} cells) in {:.0?}",
            args.detail,
            world.topology.len(),
            started.elapsed()
        );
        Simulation::new(world, registry.clone())
    };
    print_biomes(sim.world(), &registry);

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
    if let Some(path) = &args.save {
        let started = Instant::now();
        sim_core::save::save(path, &sim.save_data(String::new()), &registry)?;
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        println!("saved {} ({:.1} MB) in {:.0?}", path.display(), size as f64 / 1e6, started.elapsed());
    }

    if let Some(path) = args.png {
        let mode = match args.view {
            View::Terrain => MapMode::Terrain,
            View::Elevation => MapMode::Elevation,
            View::Plates => MapMode::Plates,
            View::Rainfall => MapMode::Rainfall,
            View::Temperature => MapMode::Temperature,
            View::Fertility => MapMode::Fertility,
            View::Currents => MapMode::Currents,
        };
        let (projection, width, height, lighting) = match args.projection {
            ProjectionArg::Flat => (Projection::WHOLE_MAP, args.png_width, args.png_width / 2, false),
            ProjectionArg::Globe => {
                let view = map_raster::View {
                    lon: args.lon.to_radians(),
                    lat: args.lat.to_radians(),
                    zoom: args.zoom,
                };
                (Projection::Globe(view), args.png_width, args.png_width, true)
            }
        };
        let opts = RenderOptions { mode, grid: args.grid, lighting, ..Default::default() };
        let img = map_raster::render(sim.world(), &registry, &opts, projection, width, height);
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
    river_stats(world, registry);
    lake_stats(world, registry);
    let continental = world.geology.plates.iter().filter(|p| p.continental).count();
    println!("  plates: {} ({continental} continental)", world.geology.plates.len());
}

/// How many lakes there are and how compact they are: a lake hex with at
/// most two lake neighbours is part of a thin string rather than open water.
fn lake_stats(world: &World, registry: &Registry) {
    let topo = &world.topology;
    let is_lake = |i: usize| registry.biome(world.terrain.biome[i]).lake;
    let mut seen = vec![false; topo.len()];
    let mut sizes = Vec::new();
    let mut thin = 0;
    for start in 0..topo.len() {
        if seen[start] || !is_lake(start) {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let mut size = 0;
        while let Some(i) = stack.pop() {
            size += 1;
            let mut lake_neighbours = 0;
            for nb in topo.neighbors(HexId(i as u32)) {
                let j = nb.index();
                if is_lake(j) {
                    lake_neighbours += 1;
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if lake_neighbours <= 2 {
                thin += 1;
            }
        }
        let (lat, lon) = topo.lat_lon(HexId(start as u32));
        sizes.push((size, (lat.to_degrees().round() as i32, lon.to_degrees().round() as i32)));
    }
    if sizes.is_empty() {
        println!("  lakes: none");
        return;
    }
    let total: usize = sizes.iter().map(|s| s.0).sum();
    sizes.sort_unstable();
    let (largest, at) = sizes[sizes.len() - 1];
    println!(
        "  lakes: {} ({} hexes, largest {} at lat/lon {:?}, median {}), {:.0}% of lake hexes in thin strings",
        sizes.len(),
        total,
        largest,
        at,
        sizes[sizes.len() / 2].0,
        thin as f64 / total as f64 * 100.0
    );
}

/// How rivers behave: how many river hexes flow uphill (into higher ground,
/// as when crossing a filled basin), and how many run side by side with a
/// different river instead of joining it.
fn river_stats(world: &World, registry: &Registry) {
    let uphill_step: f32 = std::env::var("FWS_UPHILL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.002);

    let t = &world.terrain;
    let is_river = |i: usize| t.river(i) > 0.0 && !registry.biome(t.biome[i]).water;
    let (mut total, mut uphill, mut parallel) = (0, 0, 0);
    for i in (0..t.len()).filter(|&i| is_river(i)) {
        total += 1;
        let d = t.drain[i];
        // Noticeably uphill: more than a hair (fills add tiny slopes).
        if d != sim_core::NO_DRAIN && t.elevation[d as usize] > t.elevation[i] + uphill_step {
            uphill += 1;
        }
        let beside = world.topology.neighbors(HexId(i as u32)).any(|nb| {
            let j = nb.index();
            is_river(j)
                && d != j as u32
                && t.drain[j] != i as u32
                && (d == sim_core::NO_DRAIN || t.drain[j] != d)
        });
        if beside {
            parallel += 1;
        }
    }
    let pct = |n: usize| n as f64 / total.max(1) as f64 * 100.0;
    println!(
        "  river hexes  {total}: {:.1}% flow uphill, {:.1}% run beside another river",
        pct(uphill),
        pct(parallel)
    );
}
