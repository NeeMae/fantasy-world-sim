# Fantasy World Sim — Plan

A pixel-art, simulation-first world simulator for high fantasy settings, in the
spirit of Galimulator. The world runs itself; the player observes, and can
optionally intervene as a god or take over a ruler.

## Guiding principles

1. **Simulation first.** The sim core has no knowledge of rendering or the player.
   It runs headless, from a seed, on fixed ticks.
2. **Deterministic.** Same seed + same content + same command log = same history.
   This enables replays, save files that are just `seed + commands`, and
   regression tests over thousands of simulated years.
3. **Everything is a command.** God powers, ruler orders and AI decisions all go
   through one command queue applied at tick boundaries. Gamification is a
   layer on top, never a branch inside the core.
4. **Define what you want, generate the rest.** Every piece of content
   (terrain, races, factions, rulers, religions, names) can be authored by the
   user; generators only fill the gaps.

## Technology

**Rust**, as a native desktop app.

| Concern | Choice | Why |
|---|---|---|
| Language | Rust (stable) | C++-level performance, safe threading for parallel sim, Cargo workspace keeps sim/render separation enforced by crate boundaries |
| Parallelism | `rayon` | Data-parallel tick phases over hex/province arrays |
| Rendering | `bevy` (render + input only) | wgpu-based, cross-platform, good 2D/pixel support; the sim does **not** live in Bevy's ECS |
| UI | `bevy_egui` | Inspector panels, chronicle, debug tooling with little effort |
| Content format | RON via `serde` | Human-editable, Rust-native (enums, tuples), versionable content packs |
| Scripting | Rhai | Sandboxed, pure-Rust embedded scripting for events and mod logic |
| RNG | `rand_chacha` / `rand_pcg` | Seedable, portable, stable across versions |
| Noise | `noise` / `fastnoise-lite` | Worldgen |
| Tests | `cargo test` + headless CLI | Long-run "does history stay interesting" checks |

Bevy is used only as a view so the sim stays free of engine churn and can be
benchmarked/tested without a window. If Bevy ever becomes a burden, the renderer
can be swapped (e.g. `macroquad` or raw `wgpu`) without touching the sim.

### Workspace layout

```
crates/
  sim-core/     # world state, tick systems, commands, chronicle. No graphics deps.
  worldgen/     # terrain, rivers, biomes, provinces, initial peoples/factions
  map-raster/   # hex world -> pixel-art image; shared by app (texture) and CLI (PNG)
  content/      # schema + loader for user content packs; validation
  sim-cli/      # headless runner: `sim-cli --seed 42 --years 2000 --pack packs/base`
  app/          # Bevy + egui desktop app (renderer, camera, UI, input -> commands)
packs/
  base/         # default races, biomes, factions, name lists, placeholder tileset
docs/
```

## The world: hexes, rendered as pixels

The sim runs on a **hex grid** (axial coordinates, flat-topped), e.g. 512×320
hexes ≈ 160k cells to start, scaling toward ~1M as performance allows.

**Topology:** a bounded rectangle first. Cylindrical (east-west) wrap comes
later as an option, not a replacement. To keep that cheap, all neighbour,
distance and pathfinding queries go through a `Topology` type
(`Bounded | WrapX`) from day one, and no system does raw coordinate maths.

The "pixels like sand" feeling is kept **in the presentation and in local effects**,
not in the macro sim:

- Each hex renders as a small pixel-art tile (placeholder ~16×14 px), chunked
  into large meshes/texture atlases for speed.
- Borders, rivers, roads and coastlines are drawn at pixel resolution with
  autotiling, so the map reads as a pixel painting, not a board game.
- **Cellular overlays** on the hex grid give sand-sim-style spreading
  behaviour where it matters: wildfire, plague, blight, corruption, flooding,
  magical taint. These are cheap per-hex automata ticked in parallel.
- Later (stretch): a "zoom into a hex" local view could run a true per-pixel sim
  for flavour, without affecting world-scale performance.

### Data layout

Hex data is stored as struct-of-arrays (`Vec<Elevation>`, `Vec<Biome>`,
`Vec<OwnerId>`, …), indexed by hex id. Higher-level entities (settlements,
factions, characters, armies) live in generational arenas (`slotmap`) and
reference hexes/provinces by id. This is cache-friendly, rayon-friendly and
trivially serialisable.

## World model

| Layer | Contents |
|---|---|
| Terrain | Elevation, moisture, temperature → biome; rivers; resources |
| Provinces | Clusters of hexes; the unit of ownership (the "stars" of Galimulator) |
| Peoples | Races and cultures: biome preferences, traits, growth, temperament |
| Settlements | Population, food, wealth, defence; village → town → city → capital |
| Factions | Kingdoms, empires, city-states, hordes, theocracies; ruler, dynasty, stability, treasury, personality weights |
| Characters | Rulers, heirs, generals, heroes, archmages; age, traits, ambition, relationships |
| Forces | Armies, monsters, migrations moving on the province graph |
| Overlays | Religion, magic/ley lines, trade routes, cellular hazards |

## Tick model

One tick = one month (tunable). Each tick runs ordered phases; within a phase,
work is parallel over hexes or provinces, writing into a double buffer, so
results never depend on thread scheduling.

1. Apply queued commands (player + AI)
2. Environment & cellular overlays (fire, plague, blight…)
3. Population, food, growth, migration
4. Economy & trade
5. Faction AI: evaluate opinions, choose actions → emit commands for next tick
6. Military movement & battle resolution
7. Stability, unrest, rebellion, succession
8. Events & catastrophes
9. Chronicle: record significant changes

Randomness in parallel phases uses per-entity RNG streams derived from
`(world_seed, tick, entity_id)`, never a shared RNG.

## Emergent dynamics (the Galimulator engine)

- **Expansion** into unclaimed neighbours
- **Diplomacy** via pairwise opinion (borders, culture, religion, grievances)
- **War**: armies on the province graph, simple odds-based battles, conquest
- **Instability** — the key loop: size, diversity, war-weariness and weak rulers
  accumulate unrest → rebellion, civil war, secession. Empires must fall.
- **Succession**: death → inheritance, crisis, or partition
- **Catastrophes**: plagues, monster awakenings, cataclysms, rising dark lords

## Chronicle

Every significant event is stored as structured data (`year, kind, actors,
places, cause`) and rendered to text. Drives timelines, faction histories,
family trees, and lore export.

## User-defined content

Modding follows the RimWorld model: **everything is a def.** Content packs are
directories of RON files, layered (`base` → user packs → per-world overrides).
Anything not specified is generated.

- Every def has a unique `id`; packs can add new defs or **override/patch**
  existing ones by id.
- Defs can be `abstract` and inherit from a `parent` (RimWorld's
  `ParentName`), so a mod can define `ElfBase` once and derive variants.
- Engine code never hard-codes content: the base game is itself just a pack.
- The loader is format-agnostic through serde, so JSON could be accepted as an
  alternate input later at little cost; RON is canonical.
- **Rhai scripts** handle logic that data can't express: event conditions and
  outcomes, custom god powers, AI personality hooks. Scripts run inside
  the deterministic tick (seeded RNG exposed to them, no wall-clock or I/O) and
  can only act through commands.

| Defineable | Example |
|---|---|
| Map | Supply a heightmap / painted biome image, or pin specific regions; generator fills the rest |
| Biomes & resources | Custom biomes, colours, tiles, yields |
| Races & cultures | Traits, preferred biomes, lifespans, name generators |
| Factions | Starting realms, capitals, rulers, relationships |
| Characters | Named historic figures with traits and dynasties |
| Religions & magic | Doctrines, schools, effects |
| Monsters | Spawning rules, behaviours |
| Events | Scripted or conditional events ("in year 300, the Lich awakens") |
| Name lists | Per-culture syllable/markov name tables |

Schema validation produces clear errors in the app and CLI. Defs come first;
Rhai is introduced with the event system (Phase 5).

## Art

Placeholder pixel tiles generated procedurally in code (noise-dithered biome
colours, simple settlement/army glyphs), packed into an atlas at startup. The
tileset format is defined by the content pack from day one, so a commissioned
tileset is a drop-in replacement.

## Gamification (optional layers)

1. **Observer** (default): time control, inspect anything, follow a faction or character
2. **God mode**: spawn heroes or monsters, smite, bless or curse, trigger events
3. **Ruler mode**: take over one faction; its AI reads your commands instead
4. **Goals**: scenarios, survival or unification objectives, achievements

## Roadmap

| Phase | Goal | Done when |
|---|---|---|
| 0. Skeleton | Cargo workspace, headless tick loop, seeded RNG, Bevy window drawing a hex grid, CI | `sim-cli` runs N ticks; app shows a pannable/zoomable hex map |
| 1. Worldgen | Terrain, biomes, rivers, provinces, placeholder tiles, content-pack loading for biomes | Good-looking continent from a seed; a pack can override biomes/heightmap |
| 2. Life | Peoples, settlements, factions, expansion, border rendering | Kingdoms grow and fill the map |
| 3. Conflict | Diplomacy, armies, war, rebellion, secession | Empires rise and fall unattended over 1000+ years |
| 4. Story | Characters, dynasties, succession, chronicle + inspector UI | Clicking a realm shows its history |
| 5. Fantasy | Monsters, magic, religion, cellular hazards, catastrophes | Distinctly high fantasy |
| 6. Play | Save/load (seed + command log), god powers, ruler mode, scenarios | Optional game layers work |
| 7. Worldbuilding tools | Full user-defined content, map painting, lore export (Markdown/JSON) | Usable as a setting-design tool |
| 8. Cylindrical worlds | `WrapX` topology: wrapping camera, borders and pathfinding across the seam | A world can be generated and simulated with east-west wrap |

**MVP = phases 0–3.**

## Performance targets (initial)

- 160k hexes, ~200 factions, 5k settlements: ≥ 100 ticks/sec headless on a
  modern 8-core desktop (≈ 8 years/sec of history)
- App holds 60 fps while rendering independently of sim speed
  (sim on its own thread, renderer reads a published snapshot)

## Decisions

| Question | Decision |
|---|---|
| Tick length | One month |
| World shape | Bounded rectangle now; optional east-west wrap later (Phase 8), kept possible via the `Topology` abstraction |
| Content format | RON, RimWorld-style defs with inheritance and patching |
| Scripting | Rhai, from Phase 5 |
