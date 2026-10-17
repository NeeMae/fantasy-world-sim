# Changelog

## v0.3.0

The last release of the flat map: river drawing, an Esc menu, image
export and an experimental simulated geology. (Development now moves to a
world generated on a globe.)

### New
- **River tool** (R): drag the way the water should flow to draw a stream,
  river, big river or great river. Where you stop, it carries on downhill
  along the land's drainage to the sea; crossing another river takes that
  river's water along. Erase mode dries rivers up (the rivers below them
  shrink or vanish). Undo and redo cover river edits, and they're saved.
- **Esc menu**: Resume, Save, Save as…, Load…, Export image…, New random
  world and Quit, with the world paused while it's open. Loading, quitting
  or starting a new world asks first if there are unsaved changes; saves
  can be deleted from the Load page. It replaces the sidebar's Saves panel.
  Ctrl+S still quick-saves.
- **Export image**: the whole map, in the view on screen, as a PNG at one
  of four sizes, optionally with hex grid lines. Images go to an `exports`
  folder beside the saves.
- **Simulated geology** (experimental, World panel → Geology): instead of
  sculpting mountains directly, the land is raised by the plates over many
  steps while rivers cut valleys and slopes creep, so ranges come out as
  carved ridges and valleys. Slower (several seconds); Classic stays the
  default and is unchanged.
- New worlds generate in the background, with a window showing each stage
  and a live preview; the old world stays up until the new one is ready.
- **Rivers** setting: from only the great rivers to every sizeable stream.
  The default now shows about half as many rivers as v0.2.0.
- **Lakes** setting: from none to big lakes.
- Closed basins: a lake too small to fill its basin has no outlet, so its
  rivers end there (like the Great Salt Lake) instead of climbing out of the
  basin to reach the sea.

### Fixed
- Edits (painting, rivers) now show when the map is zoomed out, not only
  once zoomed in.
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
