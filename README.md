# Fantasy World Sim

A pixel-art, simulation-first world simulator for high fantasy settings, in
the spirit of Galimulator. The world runs itself; you watch history unfold,
and can optionally step in as a god or take the throne of a realm.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and roadmap.

**Status:** Phase 1 (living land) prototyped: tectonics, simulated climate,
rivers and lakes, fertility, and cultures with their own languages. Nothing
lives in the world yet; that's Phase 2.

## Running

Requires a recent stable Rust toolchain. On Linux, Bevy also needs
`libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev`.

```sh
# Desktop app (run from the repository root so it finds packs/base)
cargo run --release -p app -- --seed 42
cargo run --release -p app -- --seed 3 --world-size 4 --wrap   # a whole planet

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
| Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y | Undo / redo a brush stroke |
| M | Cycle map views: Terrain, Elevation, Rainfall, Temperature, Fertility, Plates |
| Space, 1–5 | Pause/resume, speed |

The World panel generates a new world from:

| Setting | Effect |
|---|---|
| World size | How much of the planet the map shows, from a *Region* (one sea and its shores) to a *Planet* |
| Continents | Fewer, bigger landmasses or more, smaller ones |
| Scale | How finely the world is divided into hexes; the geography stays the same |
| Canvas | Aspect ratio, or a custom width × height in hexes |
| Edges | *Open*: land runs off the map. *Ocean*: the world is ringed by sea |
| Wrap | East–west wrapping, for a globe you can pan around forever |
| Climate | *Regional* (cool north, warm south) or *Globe* (pole to pole) |

`sim-cli` takes the same options as flags (`--world-size`, `--continent-size`,
`--shape cylinder`, `--edges ocean`, `--climate globe`, `--width`, `--height`),
`--volatility`, `--view terrain|elevation|rainfall|temperature|fertility|plates`
for `--png`, and `--names N` to print sample names for every culture.

Mountains come from plate tectonics: colliding plates raise ranges (with
trenches offshore and island arcs at sea), separating plates open rifts, and
microplates add hill country. The Plates view shows the plates, with
boundaries tinted red where they collide and blue where they pull apart.

Rainfall is simulated after Dwarf Fortress: prevailing winds (trade winds,
westerlies, polar easterlies) carry moisture from warm seas inland, rain it
out steadily, and dump most of it on the windward side of mountains,
leaving rain shadows behind them. Latitude bands make the equator wet and
the subtropics dry.

Rivers come from that rainfall: sinks are filled so everything drains to
the sea (or off an open edge), water gathers downhill into rivers, and
basins that rivers keep filled become lakes. Rivers water their banks, so
a river through a desert keeps a green floodplain.

## Layout

| Crate | Purpose |
|---|---|
| `crates/sim-core` | World state, hex topology, commands, tick loop, chronicle. No graphics. |
| `crates/content` | Def schemas and the RON content-pack loader |
| `crates/worldgen` | Procedural terrain and biomes |
| `crates/map-raster` | Draws the hex world as pixel art (used by the app and CLI) |
| `crates/names` | Name generator for culture languages |
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

Relief (flat, hills, mountains, …) is separate from biome, so a hex can be
forested hills or desert mountains. Biomes can also require a relief and a
`massif` (how great the mountain range is), so great ranges get their own
biomes (Alpine, Crags, Snowy Peaks in the base pack) while small or old
ranges keep the biome of the land around them:

```ron
(id: "alpine", relief: ["mountains"], massif: (0.35, 1.0), moisture: (0.3, 1.0), priority: 6)
```

Biomes and reliefs also carry `fertility` and `travel_cost`, which Phase 2's
peoples will use.

Races and cultures live in `races.ron` and `cultures.ron`. Each culture
defines its own language (sounds, syllable shapes, forbidden sequences and
endings for people and places), and names are generated from it:

```ron
language: (
    consonants: ["k", "kh", "g", "d", "r", "z", "m", "n"],
    vowels: ["a", "u", "o", "i"],
    syllables: ["CVC", "CV", "VC"],
    place_endings: ["dum", "zad", "gor"],
)
```

Relief defs pick a range of the tectonically driven `ruggedness` value and
a map symbol; see `packs/base/defs/reliefs.ron`:

```ron
(reliefs: [(id: "badlands", name: "Badlands", ruggedness: (0.35, 0.5), priority: 1, glyph: "hills")])
```
