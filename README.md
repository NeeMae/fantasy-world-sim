# Fantasy World Sim

A pixel-art, simulation-first world simulator for high fantasy settings, in
the spirit of Galimulator. The world runs itself; you watch history unfold,
and can optionally step in as a god or take the throne of a realm.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and roadmap.

> **This branch (`globe-view`) generates the world on a sphere**: a
> geodesic grid of hexes (and 12 pentagons) wrapped round the planet, shown
> as a globe or a flat map. See [`docs/GLOBE.md`](docs/GLOBE.md) for what
> changed and how it compares with the flat-map `master`.

**Status:** Phase 1 (living land) prototyped: tectonics, simulated climate,
rivers and lakes, fertility, and cultures with their own languages. Nothing
lives in the world yet; that's Phase 2.

## Running

Requires a recent stable Rust toolchain. On Linux, Bevy also needs
`libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev`.

```sh
# Desktop app (run from the repository root so it finds packs/base)
cargo run --release -p app -- --seed 42
cargo run --release -p app -- --seed 3 --detail fine --world-size 6   # a bigger, finer planet

# Headless: generate, simulate 100 years, export the map as a PNG
cargo run --release -p sim-cli -- --seed 42 --years 100 --png world.png
cargo run --release -p sim-cli -- --seed 42 --png globe.png --projection globe --lat 30 --lon 90
```

App controls:

| Input | Action |
|---|---|
| Right- or middle-drag, WASD / arrows | Turn the globe (or pan the map) |
| Mouse wheel, + / − | Zoom |
| G | Switch between the globe and a flat map of the whole planet |
| Home | Recentre the view |
| Left click | Use the current tool |
| I / B | Inspect tool / Paint (terrain brush) tool |
| [ / ] | Shrink / grow the brush |
| Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y | Undo / redo a brush stroke |
| Ctrl+S | Save the world (see the Saves panel to name and load saves) |
| M | Cycle map views: Terrain, Elevation, Rainfall, Temperature, Fertility, Currents, Plates |
| Space, 1–5 | Pause/resume, speed |

The World panel generates a new world from:

| Setting | Effect |
|---|---|
| Planet size | Bigger planets hold more, smaller continents |
| Continents | Fewer, bigger landmasses or more, smaller ones |
| Detail | How finely the planet is divided into hexes (64 000 to 1.3 million); the geography stays the same |
| Sea level | How much of the map lies under the sea |
| Temperature | From an ice age to a hothouse world |
| Rainfall | From arid to drenched |
| Mountains | How much mountain building the plates do |
| Erosion | How much rivers have worn the land: young and sharp to ancient and carved |
| Rivers | How many streams count as rivers: only the great rivers, or every sizeable stream |
| Lakes | How big lakes grow before evaporation balances their inflow (none to big) |
| Inland seas | How many basins sink into the continents and flood into enclosed seas |
| Volatility | How turbulent history will be (used from Phase 2) |

Worlds are saved to `fantasy-world-sim/saves` in your user data folder
(`~/.local/share` on Linux, `%APPDATA%` on Windows, `~/Library/Application
Support` on macOS); set `FWS_SAVES` to use another folder. A save holds the
world, its history and the settings that made it.

`sim-cli` takes the same options as flags (`--detail` (grid frequency: the
planet has 10 × detail² + 2 hexes), `--world-size`, `--continent-size`,
`--ocean`, `--temperature`, `--rainfall`, `--mountains`, `--erosion`,
`--rivers`, `--lakes`, `--inland-seas`, `--volatility`), `--view
terrain|elevation|rainfall|temperature|fertility|currents|plates` and
`--projection flat|globe` (with `--lat`, `--lon`, `--zoom`) for `--png`,
`--save FILE` / `--load FILE`, and `--names N` to print sample names for every
culture.

Mountains come from plate tectonics: colliding plates raise ranges (with
trenches offshore and chains of volcanic islands at sea), separating plates
open rifts, and microplates add hill country. Erosion then carves river
valleys into the ranges and silts up basins. The Plates view shows the plates, with
boundaries tinted red where they collide and blue where they pull apart.

Rainfall is simulated after Dwarf Fortress: prevailing winds (trade winds,
westerlies, polar easterlies) carry moisture from warm seas inland, rain it
out steadily, and dump most of it on the windward side of mountains,
leaving rain shadows behind them. Latitude bands make the equator wet and
the subtropics dry. Ocean currents circle each ocean basin: warm along its
western side and cold along its eastern side in the subtropics (so west
coasts there run dry, like the Atacama), reversed further poleward (a warm
drift for north-west coasts, like Europe). The Currents view shows them.

Rivers come from that rainfall: water gathers downhill into rivers, small
streams turn to join bigger rivers beside them, and basins fill with lakes.
A lake that fills its basin overflows and its river carries on to the sea;
one that doesn't (too dry, like the Great Salt Lake) has no outlet, and the
rivers feeding it end there. Rivers water their banks, so
a river through a desert keeps a green floodplain.

## Layout

| Crate | Purpose |
|---|---|
| `crates/sim-core` | World state, the geodesic hex grid, commands, tick loop, chronicle. No graphics. |
| `crates/content` | Def schemas and the RON content-pack loader |
| `crates/worldgen` | Procedural terrain and biomes |
| `crates/map-raster` | Draws the planet as pixel art, as a globe or a flat map (used by the app and CLI) |
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
