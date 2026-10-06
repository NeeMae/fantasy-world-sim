# Changelog

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
