# The world on a globe (`globe-view` branch)

This branch generates the world on a sphere instead of a flat hex map. It's
an experiment to compare with `master`, not yet the plan of record.

## What changed

**The grid** (`sim-core/src/hex.rs`). The world is an icosahedron whose 20
faces are each divided into a triangular lattice and pushed out onto a
sphere. Each lattice point is a cell: hexagons everywhere except 12
pentagons at the icosahedron's corners. A grid of frequency *f* has
10·f² + 2 cells (Normal is f = 130, about 169 000 cells). Cells keep their
neighbours in order round them; there are no rows, columns or edges.
`Topology` offers positions, latitude/longitude, neighbours, nearest-cell
lookup (a greedy walk), `within`, `line` and cell outlines. A save stores
only the frequency and rebuilds the grid (0.05 s at Normal, 0.2 s for 676 000
cells).

**World generation** (`worldgen`). Every layer is sampled at the cells' 3D
positions on a sphere in noise space whose pole-to-pole distance is the
*planet size*, so there are no seams and no pinched poles:

- Plates are centred on the planet's surface, so their borders are great
  circles and they turn about Euler poles.
- Wind-driven rainfall and ocean currents, which swept along map rows, now
  sweep along rings of latitude.
- Blurs are neighbour averaging over a distance in noise space.
- Erosion still runs on a coarse copy of the world (a frequency-61 sphere) and
  is resampled onto the map, so every detail level gets the same valleys.
- Rivers, lakes, inland seas and biomes only ever walked neighbours, so they
  carry over unchanged; the sea is the only outlet (there are no map edges).

Settings that only made sense for a flat map are gone: canvas shape and
size, open or ocean edges, east–west wrap and the regional climate.

**Drawing** (`map-raster`). Each pixel finds the point of the planet under
it and the nearest cell (starting from its neighbour's, so it's quick).
Coasts, plate borders and the grid come from how close the pixel is to the
edge between neighbouring cells; rivers from the distance to the segments
between cell centres. The same renderer draws a lit globe or a flat
(equirectangular) map, which pans and zooms too. Relief symbols are drawn at
cells whose hash beats all their neighbours'.

**The app**. The planet fills the window as an egui image, redrawn whenever
the view or the world changes, drawn at no more than about 7 pixels per hex
and scaled up, so zooming in enlarges the pixel art as the flat map did.
Right-drag or WASD turns the globe; G switches to the flat map; outlines for
hover, brush and selection are drawn over it. The 2D camera and chunked map
are gone.

## Compared with `master`

| | `master` (flat) | `globe-view` |
|---|---|---|
| Shape | A rectangle, optionally wrapping east–west | A whole planet |
| Distortion | None on the map, but a wrapped world can't close at the poles | None on the globe; the flat map stretches the poles, as Earth's do |
| Regional maps | Yes (*World size* zooms to a region) | No: always a whole planet (zoom in to look at a region) |
| Grid | Hexes in rows; simple maths | Hexes plus 12 pentagons; neighbour lists |
| Generation (Normal) | ~0.7 s for 177 000 hexes | ~1 s for 169 000 hexes |
| Drawing | Pre-drawn texture chunks, camera moves | Redrawn on the CPU when the view moves |
| Saves | Format 3 | Format 4 (flat saves don't load) |

## Open questions

- **Regional maps.** A "region" setting could generate only a patch of a
  larger planet, or keep the whole planet and start zoomed in.
- **Drawing speed.** Turning the globe redraws on the CPU (at half
  resolution while moving). Fine here; a GPU shader would make it free if
  slower machines struggle.
- **Pentagons.** Twelve cells have five neighbours. Everything handles it,
  but later systems (movement, settlements) must not assume six.
- **Rivers** are a little more numerous than on `master` at the default
  setting, and lakes rarer; both are tuning.
