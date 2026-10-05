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

### Generation

- **Edges:** *Open* by default: the map is a window onto a larger world, so
  land and sea run off every side. *Ocean* gives a self-contained world:
  a gentle slope makes land likelier towards the middle without dictating
  the coast, plus a thin guaranteed band of sea at the border.
- **Continents:** a continental-crust field (domain-warped noise) decides
  where land is, independently of plate outlines: as on Earth, most
  coasts are not plate boundaries.
- **Plate tectonics:** two layers of plates (major plates and microplates)
  as a continuous Voronoi field over noise space, so they exist past open
  edges and wrap seamlessly. Each plate drifts; at each boundary the
  relative motion and the crust on either side decide the landform:
  continental collisions raise great ranges, ocean-under-continent gives a
  coastal range and trench, ocean-ocean gives island arcs, separation gives
  rifts and mid-ocean ridges. Uplift of ocean floor (arcs, microplate
  ridges, barely-continental crust, ridges) only breaks the surface at
  volcanic hot spots (jittered sites, some dormant), so it rises as chains
  of round islands rather than long lines of land. Pair effects are blended with soft plate
  memberships so the land is continuous everywhere, including triple
  junctions (tested).
- **Relief vs biome:** relief (flat/hills/mountains, content-defined) is a
  separate layer from biome, driven by tectonic ruggedness plus high
  ground. Placeholder art draws one symbol per seven-hex "flower". Biomes
  may require a relief and a *massif* (range greatness from the tectonic
  uplift), so the Alps, Rockies and Himalaya differ while the Appalachians
  keep their forests and hills never change the biome.
- **Erosion:** ten rounds of a stream-power model on a fixed-resolution
  (160-row) copy of the raw terrain, blended onto the map so every scale
  erodes alike: gathered water cuts towards its drain (√discharge, capped
  per round), steep slopes slump, sinks silt up. Cuts never dig below the
  coarse eroded surface (no notches at cliff feet); deeply cut valleys lose
  ruggedness, so ranges break up into ridges.
- **Inland seas:** low-frequency basins sag into continental interiors
  (the Inland seas setting); sea bodies not reaching the open ocean or an
  open edge and under a fifth of all sea get inland-sea biomes.
- **Ocean currents:** per row, how near land lies west and east of each sea
  hex gives its side of the basin; subtropical gyres run warm on the west,
  cold on the east, subpolar ones the reverse; smoothed over the sea. The
  anomaly shifts sea temperature (so evaporation: cold currents dry their
  coasts) and coastal land temperature.
- **Hydrology:** priority-flood sink filling so all land drains to the sea
  or off an open edge; drainage prefers the biggest drop in the original
  ground (so rivers follow valley floors through filled basins) with a
  hashed wobble so they meander; discharge accumulates rainfall × area
  (scale-independent); rivers water their banks.
- **Lakes:** a lake is a whole flooded basin, not just the hexes a river
  crosses. Water rises from the basin's lowest point until the lake covers
  about 5% of its catchment (evaporation balancing inflow) or it spills;
  then a shape pass keeps only open water (hexes within one step of a hex
  surrounded by lake, or small compact ponds), so flooded valley floors stay
  rivers instead of long strings of lake. The rest of the basin drains into
  the lake along the real terrain.
- **Climate:** temperature from latitude and altitude. Rainfall is
  simulated: winds by latitude band carry moisture from warm seas, rain it
  out over land, wring it out over mountains (rain shadows), with wet
  equator, dry subtropics and recycling by vegetation. Rates are per
  noise-space distance, so climate is scale-independent (tested).
- **World size vs scale:** *world size* is how much of the planet the map
  shows (region → planet); *scale* is how many hexes it's drawn with. The
  same seed gives the same geography at any scale, and noise detail finer
  than about three hexes is skipped.
- **Wrap:** optional east-west wrapping samples noise on a cylinder, so the
  seam is invisible; the app draws three copies and keeps the camera over
  the middle one.
- **Climate:** a regional map runs from a cool north to a warm south; a
  wrapped planet defaults to pole to pole.

## World model

| Layer | Contents |
|---|---|
| Terrain | Elevation, moisture, temperature → biome; tectonic ruggedness → relief; rivers; resources; fertility and travel cost from biome and relief defs |
| Geology | Tectonic plates and boundary stress, kept for volcanoes and earthquakes |
| Peoples | Races, cultures (with languages) and religions; biome preferences, traits, growth, lifespans, temperament |
| Population | Per-settlement demographic mix: groups of `(race, culture, religion, size)` that grow, migrate, assimilate and convert |
| Settlements | Founded *dynamically* where people gather; population, food, wealth, defence; hamlet → village → town → city |
| Counties | The territory a settlement controls (its hinterland), grown outward by travel cost; the smallest region and unit of ownership |
| Titles | County → Duchy → Kingdom → Empire, Crusader Kings-style, formed around counties rather than drawn in advance |
| Realms | A title holder plus vassals: kingdoms, empires, city-states, hordes, theocracies; ruler, dynasty, stability, treasury, personality weights |
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

## Regions grow from people, not the other way round

There are no pre-drawn provinces. Peoples start at a few origin points and
spread; when enough people gather on good land, a **settlement** is
founded. Each settlement claims a **county**: the surrounding hexes it can
reach most cheaply, so counties are small in rich farmland and large in
steppe or tundra, and borders follow rivers and ridges naturally. Counties
average roughly 10–25 hexes on fertile land.

Higher **titles** form around counties: a ruler who holds enough counties in
a region can found a duchy, enough duchies a kingdom, and so on. Titles
persist after the realm that created them falls, so they can be claimed,
usurped and re-formed, which is where the CK-style stories come from.

Not every culture is feudal. Title *tiers* are universal, but cultures
name and run them differently (chiefdom → tribe → confederacy, or
city-state → league), defined in culture defs.

## World settings

Chosen at generation and stored in the world, so they're part of determinism:

- **Volatility** (calm ↔ chaotic): scales unrest growth, rebellion
  thresholds, AI aggression, succession-crisis odds and catastrophe
  frequency. Calm worlds grow long-lived empires; chaotic ones churn.
- History is balanced for good stories over roughly **2,000 years**.
- Later: magic level, monster density, starting peoples.

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
| Languages | Per-culture phonology in the culture def: sounds, syllable shapes, forbidden clusters, and templates for people, places and titles, so each culture names things in its own style |

Schema validation produces clear errors in the app and CLI. Defs come first;
Rhai is introduced with the event system (Phase 6).

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
| 0. Skeleton ✅ | Cargo workspace, headless tick loop, seeded RNG, Bevy viewer, CI | `sim-cli` runs N ticks; app shows a pannable/zoomable hex map |
| 0.5 Editor basics ✅ | Hex outlines, terrain brush with undo/redo, instant god powers, regenerate | Worlds can be rerolled and reshaped comfortably |
| 0.6 World shape ✅ | Open/ocean edges, world size vs scale, custom canvas, east-west wrap | Region maps through wrapped planets from one generator |
| 0.7 Tectonics ✅ | Plates, boundary landforms, relief layer, map views (terrain/elevation/plates), relief brush | Mountain ranges follow plate collisions |
| 0.8 Climate ✅ | Wind-driven rainfall with rain shadows, range-dependent mountain biomes, rainfall/temperature views | Deserts sit in rain shadows and subtropics |
| 0.9 World tuning ✅ | Save/load; sea level, temperature, rainfall, mountains, erosion and inland-sea settings; island chains; ocean currents; erosion | Worlds are varied, natural and keepable |
| 1. Living land 🧪 | Rivers and lakes (fed by simulated rainfall); biome fertility and travel cost; race, culture and language defs; name generator; world settings (volatility) | Rivers carve the map; cultures generate distinct names |
| 2. Peoples & settlements | Population groups, growth, migration; settlements founded dynamically; counties as settlement hinterlands; border and settlement rendering | Peoples spread from origins and settle the land into counties |
| 3. Titles & realms | County → Duchy → Kingdom → Empire; titles formed around counties; realms, vassals, expansion | Kingdoms form and fill the map |
| 4. Conflict | Diplomacy, armies, war, rebellion, secession, scaled by volatility | Empires rise and fall unattended over 2,000 years |
| 5. Story | Characters, dynasties, succession, chronicle and inspector UI | Clicking a realm shows its history |
| 6. Fantasy | Monsters, magic, religion, cellular hazards, catastrophes, Rhai events | Distinctly high fantasy |
| 7. Play | Save/load ✅ (full state, deterministic), more god powers, ruler mode, scenarios | Optional game layers work |
| 8. Worldbuilding tools | Import heightmaps and painted maps, lore export (Markdown/JSON) | Usable as a setting-design tool |

🧪 = prototyped: in place and working, still to be tuned as later phases use it.

**MVP = phases 0–4.**

## Performance targets (initial)

- 160k hexes, ~200 factions, 5k settlements: ≥ 100 ticks/sec headless on a
  modern 8-core desktop (≈ 8 years/sec of history)
- App holds 60 fps while rendering independently of sim speed
  (sim on its own thread, renderer reads a published snapshot)

## Decisions

| Question | Decision |
|---|---|
| Tick length | One month |
| World shape | Bounded rectangle or optional east-west wrap, chosen per world; every system goes through `Topology` |
| Content format | RON, RimWorld-style defs with inheritance and patching |
| Scripting | Rhai, introduced with events (Phase 6) |
| Regions | No pre-drawn provinces: settlements emerge from population, counties form around settlements, titles form around counties |
| Title tiers | County → Duchy → Kingdom → Empire, with culture-specific names and customs |
| Population | Demographic mix per settlement (race, culture, religion groups) |
| Smallest region | Counties, averaging roughly 10–25 hexes on fertile land |
| History length | Balanced for about 2,000 years, with a world volatility setting |
| Names | Per-culture language rules in culture defs |
