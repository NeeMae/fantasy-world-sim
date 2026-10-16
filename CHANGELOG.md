# Changelog

## Unreleased

### New
- **Plates rebuilt** (globe-view branch): plates are grown over the planet
  in varied sizes, drift in groups (plates in a group meet along quiet old
  seams; the great ranges rise between groups), and microplates are carved
  out along the boundaries between groups. New World settings: Plates,
  Plate sizes, Microplates and Plate groups. The Plates view colours plates
  by group and shows each plate's number and drift; a selected plate's
  heading and speed can be set by hand. Saves move to format 5.
- **Esc menu**: Resume, Save, Save as…, Load…, New random world and Quit,
  with the world paused while it's open. Loading, quitting or starting a
  new world asks first if there are unsaved changes; saves can be deleted
  from the Load page. It replaces the sidebar's Saves panel.
- **(globe-view branch) The world is generated on a sphere**: a geodesic
  grid of hexes round the whole planet, shown as a globe or a flat map. See
  `docs/GLOBE.md`. Saves move to format 4; flat-map saves don't load here.
- **Globe view** (G, or Globe in the top bar): the world on a lit, spinning
  globe. Drag to turn it, scroll to zoom, click to inspect; every map view
  works on it. Wrapping worlds cover the whole globe; region maps sit as a
  patch on a larger, unexplored world.
- **Rivers** setting: from only the great rivers to every sizeable stream.
  The default now shows about half as many rivers as v0.2.0.
- **Lakes** setting: from none to big lakes.
- Closed basins: a lake too small to fill its basin has no outlet, so its
  rivers end there (like the Great Salt Lake) instead of climbing out of the
  basin to reach the sea.

### Fixed
- Rivers no longer climb small rises left by erosion: carving valleys left
  dips along them, which are now filled back up.
- Small streams beside a bigger river turn to join it more often instead of
  running alongside it.

### Note
- The same seed gives somewhat different rivers and lakes from v0.2.0.
  Saves from v0.2.0 still load, with their rivers as they were.
- Zoomed out, the map no longer shows a checkerboard of lighter and darker
  squares (or a grainy shimmer): map textures are now mipmapped.

## v0.2.0

A big round of world generation: living land (rivers, lakes, cultures),
more natural geography, and worlds you can keep.

### New
- **Plates rebuilt** (globe-view branch): plates are grown over the planet
  in varied sizes, drift in groups (plates in a group meet along quiet old
  seams; the great ranges rise between groups), and microplates are carved
  out along the boundaries between groups. New World settings: Plates,
  Plate sizes, Microplates and Plate groups. The Plates view colours plates
  by group and shows each plate's number and drift; a selected plate's
  heading and speed can be set by hand. Saves move to format 5.
- **Save and load.** Ctrl+S saves the world; the Saves panel names, lists
  and loads saves. A save holds the world, its history and the settings
  that made it, and carrying on after loading gives exactly the same history
  as never stopping. Saves live in your user data folder
  (`~/.local/share/fantasy-world-sim/saves` on Linux, `%APPDATA%` on
  Windows, `~/Library/Application Support` on macOS).
- **World settings:** sea level, temperature (ice age to hothouse), rainfall
  (arid to drenched), mountains (worn flat to jagged), erosion (none to
  ancient), inland seas (none to many) and volatility (stored for the
  history simulation to come).
- **Rivers and lakes** fed by the simulated rainfall. Rivers water their
  banks, so a river through a desert keeps a green floodplain. Lakes fill
  whole basins.
- **Ocean currents:** warm and cold currents circle each ocean, giving dry
  coasts beside cold currents and mild ones beside warm currents. New
  Currents map view.
- **Inland seas:** seas cut off from the ocean get their own look, and
  basins can sink into continents to flood into Caspian- or
  Mediterranean-like seas.
- **Erosion:** rivers carve valleys into mountain ranges, steep slopes
  slump and basins silt up.
- **High Mountains** with their own dense, snow-capped map symbol.
- **Fertility and travel cost** for every hex, with a Fertility map view and
  inspector rows (river size, fertility, travel cost, range).
- **Races, cultures and languages** defined in content files, with a name
  generator. The Peoples panel shows sample names and each culture's title
  ranks. (Peoples are placed in the world in a later version.)

### Fixed
- The speed buttons no longer jump sideways as the date grows at high speed.
- Plate collisions at sea rise as chains of islands instead of long, thin
  lines of land.
- Lakes are open water rather than long strings of lake tiles.

### Note
- The same seed gives a different world from v0.1.0.
