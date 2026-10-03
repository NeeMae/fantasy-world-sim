# Fantasy World Sim

A pixel-art, simulation-first world simulator for high fantasy settings, in
the spirit of Galimulator. The world runs itself; you watch history unfold,
and can optionally step in as a god or take the throne of a realm.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and roadmap.

**Status:** Phase 0.5. Worlds generate, render and can be reshaped by hand; nothing lives in them yet.

## Running

Requires a recent stable Rust toolchain. On Linux, Bevy also needs
`libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev`.

```sh
# Desktop app (run from the repository root so it finds packs/base)
cargo run --release -p app -- --seed 42 --size medium

# Headless: generate, simulate 100 years, export the map as a PNG
cargo run --release -p sim-cli -- --seed 42 --years 100 --png world.png
```

App controls:

| Input | Action |
|---|---|
| Right- or middle-drag, WASD | Pan |
| Mouse wheel | Zoom towards the cursor |
| Left click | Use the current tool |
| I / B | Inspect tool / Paint (terrain brush) tool |
| [ / ] | Shrink / grow the brush |
| Space, 1–5 | Pause/resume, speed |

The World panel regenerates the map from a seed, size preset and edge
style: **Open** (the map is a window onto a larger world, so land runs off
the edges) or **Ocean** (a self-contained world ringed by sea). On the
command line, pass `--ocean-edges` to the app or `--edges ocean` to `sim-cli`.

## Layout

| Crate | Purpose |
|---|---|
| `crates/sim-core` | World state, hex topology, commands, tick loop, chronicle. No graphics. |
| `crates/content` | Def schemas and the RON content-pack loader |
| `crates/worldgen` | Procedural terrain and biomes |
| `crates/map-raster` | Draws the hex world as pixel art (used by the app and CLI) |
| `crates/sim-cli` | Headless runner |
| `crates/app` | Bevy + egui desktop app |
| `packs/base` | Default content; the base game is just a pack |

## Modding

Content lives in packs: a directory with a `pack.ron` manifest and RON def
files under `defs/`. Pass extra packs with `--pack`; later packs override
earlier ones.

```sh
cargo run --release -p app -- --pack packs/base --pack path/to/my_pack
```

Defs work like RimWorld's:

```ron
(
    biomes: [
        // Redefining an existing id merges over it: only `color` changes.
        (id: "grassland", color: "#88b04b"),
        // `parent` inherits all fields; `abstract` defs exist only to be inherited.
        (id: "ashlands", parent: "land_base", name: "Ashlands", color: "#4a4040",
            temperature: (0.5, 1.0), moisture: (0.0, 0.1), priority: 2),
    ],
)
```

Biome climate ranges are percentiles for elevation and moisture (so
`elevation: (0.0, 0.55)` means "the lowest 55% of the world") and a 0–1
polar-to-equatorial scale for temperature. See `packs/base/defs/biomes.ron`.
