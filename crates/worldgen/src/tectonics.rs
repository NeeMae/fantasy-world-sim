//! Plate tectonics on the planet.
//!
//! Plates are grown over a coarse sphere grid of their own (its size set by
//! the planet's, never by the map's detail, so the same plates come out at
//! any detail). Each plate grows from a seed at its own rate, through
//! ground that's rougher in some places than others, so plates come in
//! varied sizes with irregular outlines.
//!
//! Neighbouring plates are gathered into groups that drift together, like
//! the plates of a supercontinent: boundaries inside a group are quiet old
//! sutures (worn hill country), while boundaries between groups are where
//! the great ranges rise. Microplates are carved out along those active
//! boundaries, moving half on their own, and add their own smaller ranges
//! and rifts there.
//!
//! Plates don't decide where land is: as on Earth, continents sit inside
//! plates, and most coasts are not plate boundaries. The caller supplies a
//! continental-crust field; plates only deform the land where they meet.
//! Each plate turns about its own pole, and at a boundary their relative
//! motion, together with the crust on each side, decides the landform:
//!
//! | Meeting | Converging (colliding) | Diverging (pulling apart) |
//! |---|---|---|
//! | continent + continent | great range on both sides | rift valley |
//! | continent + ocean | coastal range inland, trench offshore | rift / ridge |
//! | ocean + ocean | volcanic island arc on one side | mid-ocean ridge |
//!
//! Landforms are functions of the signed distance from a boundary, which
//! is kept per plate on the plate grid and interpolated, so they're smooth
//! wherever they're sampled, even where three plates meet.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::f64::consts::{PI, TAU};

use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rand::RngExt;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sim_core::{HexId, Topology, cross, normalize, rng, tangent_basis};

/// A plate's motion set by hand: which way it heads at its centre and how
/// fast, overriding the generated motion.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlateMotion {
    pub plate: u16,
    /// Degrees clockwise from north.
    pub heading: f64,
    /// About 0.5..1.2 for generated plates.
    pub speed: f64,
}

/// How the plates are laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateSettings {
    /// Number of plates, not counting microplates.
    pub count: u32,
    /// 0 for plates of much the same size; 1 for a few giants among many
    /// small plates.
    pub variety: f64,
    /// 0 for none; 1 for about one per plate.
    pub microplates: f64,
    /// How closely plates in a group move together: 0 each on its own, 1
    /// as one.
    pub grouping: f64,
    pub motions: Vec<PlateMotion>,
}

impl Default for PlateSettings {
    fn default() -> Self {
        PlateSettings { count: 18, variety: 0.6, microplates: 0.6, grouping: 0.4, motions: Vec::new() }
    }
}

/// One plate.
#[derive(Clone, Copy, Debug)]
pub struct PlateInfo {
    /// Rotation vector: the plate's velocity at unit point `u` is
    /// `spin × u`.
    pub spin: [f64; 3],
    pub group: u16,
    pub micro: bool,
    /// Centre of the plate on the unit sphere.
    pub centre: [f64; 3],
    /// Breaks ties between two oceanic plates (which one subducts).
    rank: u64,
}

/// Heading (degrees clockwise from north) and speed of a plate turning by
/// `spin`, at `centre`.
pub fn heading_speed(spin: [f64; 3], centre: [f64; 3]) -> (f64, f64) {
    let v = cross(spin, centre);
    let (east, north) = tangent_basis(centre);
    let (e, n) = (dot(v, east), dot(v, north));
    (e.atan2(n).to_degrees().rem_euclid(360.0), (e * e + n * n).sqrt())
}

/// The rotation that moves `centre` along `heading` at `speed`.
pub fn spin_for(centre: [f64; 3], heading: f64, speed: f64) -> [f64; 3] {
    let (east, north) = tangent_basis(centre);
    let h = heading.to_radians();
    let t = [0, 1, 2].map(|k| north[k] * h.cos() + east[k] * h.sin());
    cross(centre, t).map(|v| v * speed)
}

#[derive(Clone, Copy)]
struct Entry {
    plate: u16,
    /// Signed distance from the plate's edge (noise-space units): negative
    /// inside it.
    d: f32,
    /// Its gradient on the unit sphere.
    grad: [f32; 3],
}

/// The planet's plates.
pub struct Plates {
    grid: Topology,
    /// Grid spacing in radians.
    step: f64,
    /// Owning plate of each grid cell.
    owner: Vec<u16>,
    /// Plates within reach of each cell: `entries[start[c]..start[c + 1]]`.
    start: Vec<u32>,
    entries: Vec<Entry>,
    /// Distance fields are kept only this far (noise-space units) from a
    /// plate's edge; beyond, a cell is just inside or outside.
    cap: f64,
    pub info: Vec<PlateInfo>,
}

const NONE: u16 = u16::MAX;

/// Plate-grid frequency for a planet: about 23 000 cells on an Earth-like
/// one, more on bigger planets so boundaries stay as sharp.
fn grid_frequency(radius: f64) -> u32 {
    ((radius / 1.27 * 48.0).round() as u32).clamp(24, 160)
}

fn random_unit(rng: &mut rng::SimRng) -> [f64; 3] {
    let z = rng.random::<f64>() * 2.0 - 1.0;
    let a = rng.random::<f64>() * TAU;
    let r = (1.0 - z * z).sqrt();
    [r * a.cos(), r * a.sin(), z]
}

fn random_spin(rng: &mut rng::SimRng) -> [f64; 3] {
    let axis = random_unit(rng);
    let rate = 0.6 + 0.8 * rng.random::<f64>();
    axis.map(|v| v * rate)
}

fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}

/// Grows plates from `sources` over cells `open` allows, cheapest first;
/// stops a plate once it has `limit` cells.
fn grow(
    grid: &Topology,
    owner: &mut [u16],
    sources: &[(u32, u16)],
    cost: impl Fn(usize, u16) -> f64,
    open: impl Fn(u16) -> bool,
    limit: usize,
) {
    let mut heap = BinaryHeap::new();
    let mut size: HashMap<u16, usize> = HashMap::new();
    for &(cell, plate) in sources {
        heap.push(Reverse((0u64, cell, plate)));
    }
    while let Some(Reverse((key, cell, plate))) = heap.pop() {
        let c = cell as usize;
        if owner[c] == plate || !open(owner[c]) {
            continue;
        }
        let n = size.entry(plate).or_default();
        if *n >= limit {
            continue;
        }
        *n += 1;
        owner[c] = plate;
        let here = f64::from_bits(key);
        for nb in grid.neighbors(HexId(cell)) {
            if owner[nb.index()] != plate && open(owner[nb.index()]) {
                let step = angle(grid.position(HexId(cell)), grid.position(nb)) * cost(nb.index(), plate);
                heap.push(Reverse(((here + step).to_bits(), nb.0, plate)));
            }
        }
    }
}

impl Plates {
    /// Lays out the plates of a planet of `radius` (noise-space units).
    pub fn new(seed: u64, radius: f64, settings: &PlateSettings) -> Self {
        let grid = Topology::new(grid_frequency(radius));
        let step = grid.spacing();
        let len = grid.len();
        let n = settings.count.clamp(2, 200) as usize;
        let mut rng = rng::stream(seed, 0, 30, rng::purpose::WORLDGEN);

        // Seeds spread out by best-candidate sampling: each the farthest
        // from those before of a few random tries, so plates don't start
        // bunched up.
        let mut seeds: Vec<[f64; 3]> = Vec::with_capacity(n);
        for _ in 0..n {
            let mut best = (f64::NEG_INFINITY, [0.0, 0.0, 1.0]);
            for _ in 0..8 {
                let p = random_unit(&mut rng);
                let gap = seeds.iter().map(|&s| angle(s, p)).fold(PI, f64::min);
                if gap > best.0 {
                    best = (gap, p);
                }
            }
            seeds.push(best.1);
        }
        // Growth rates: a plate's area goes roughly with the square.
        let variety = settings.variety.clamp(0.0, 1.0);
        let rates: Vec<f64> = (0..n)
            .map(|_| {
                let (u1, u2) = (rng.random::<f64>().max(1e-12), rng.random::<f64>());
                let normal = (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos();
                (variety * 0.65 * normal).exp()
            })
            .collect();
        // Some ground is slower to cross, so outlines wander.
        let typical = radius * (4.0 * PI / n as f64).sqrt();
        let rough_noise = Fbm::<Perlin>::new(rng.random::<u32>()).set_octaves(4).set_frequency(2.5 / typical);
        let rough: Vec<f64> = (0..len)
            .into_par_iter()
            .map(|i| {
                let p = grid.position(HexId(i as u32)).map(|v| v * radius);
                (1.0 + 0.8 * rough_noise.get(p)).max(0.2)
            })
            .collect();

        let mut owner = vec![NONE; len];
        let sources: Vec<(u32, u16)> =
            seeds.iter().enumerate().map(|(i, &s)| (grid.nearest(s, None).0, i as u16)).collect();
        grow(&grid, &mut owner, &sources, |c, p| rough[c] / rates[p as usize], |o| o == NONE, usize::MAX);

        // Groups: centres picked far apart among the plates, largest first;
        // every plate joins the nearest.
        let (centres, _) = centres_and_shares(&grid, &owner, n);
        let groups = ((n as f64 / 3.5).round() as usize).clamp(1, n);
        let sizes: Vec<usize> = (0..n).map(|p| owner.iter().filter(|&&o| o as usize == p).count()).collect();
        let mut heads = vec![(0..n).max_by_key(|&p| (sizes[p], Reverse(p))).unwrap_or(0)];
        while heads.len() < groups {
            let far = (0..n)
                .filter(|p| !heads.contains(p))
                .max_by(|&a, &b| {
                    let gap =
                        |p: usize| heads.iter().map(|&h| angle(centres[h], centres[p])).fold(PI, f64::min);
                    gap(a).total_cmp(&gap(b)).then(b.cmp(&a))
                })
                .unwrap_or(0);
            heads.push(far);
        }
        let group: Vec<u16> = (0..n)
            .map(|p| {
                (0..heads.len())
                    .min_by(|&a, &b| {
                        angle(centres[heads[a]], centres[p]).total_cmp(&angle(centres[heads[b]], centres[p]))
                    })
                    .unwrap_or(0) as u16
            })
            .collect();

        // Microplates, carved out along boundaries between groups (or any
        // boundary, if there's only one group).
        let micros = (settings.microplates.clamp(0.0, 2.0) * n as f64).round() as usize;
        let mut micro_host = Vec::new();
        if micros > 0 {
            let between = |a: u16, b: u16| groups == 1 || group[a as usize] != group[b as usize];
            let mut candidates: Vec<u32> = (0..len as u32)
                .filter(|&c| {
                    let o = owner[c as usize];
                    grid.neighbors(HexId(c)).any(|nb| owner[nb.index()] != o && between(o, owner[nb.index()]))
                })
                .collect();
            let micro_size = len as f64 / n as f64;
            let spread = 1.8 * (micro_size * 0.1 * 4.0 * PI / len as f64).sqrt();
            let mut placed: Vec<[f64; 3]> = Vec::new();
            for _ in 0..micros {
                let mut seed_cell = None;
                for _ in 0..40 {
                    if candidates.is_empty() {
                        break;
                    }
                    let c = candidates.swap_remove(rng.random_range(0..candidates.len()));
                    let p = grid.position(HexId(c));
                    if (owner[c as usize] as usize) < n && placed.iter().all(|&q| angle(p, q) > spread) {
                        seed_cell = Some(c);
                        break;
                    }
                }
                let target = (micro_size * (0.04 + 0.1 * rng.random::<f64>())).max(3.0) as usize;
                let Some(c) = seed_cell else { continue };
                placed.push(grid.position(HexId(c)));
                let id = (n + micro_host.len()) as u16;
                micro_host.push((id, owner[c as usize]));
                grow(&grid, &mut owner, &[(c, id)], |c, _| rough[c], |o| (o as usize) < n, target);
            }
        }
        let total = n + micro_host.len();
        // Motion: each group turns about its own pole; its plates share
        // that turn as far as `grouping` says, the rest their own.
        let grouping = settings.grouping.clamp(0.0, 1.0);
        let group_spin: Vec<[f64; 3]> = (0..groups).map(|_| random_spin(&mut rng)).collect();
        let mut spin: Vec<[f64; 3]> = (0..n)
            .map(|p| {
                let own = random_spin(&mut rng);
                let g = group_spin[group[p] as usize];
                [0, 1, 2].map(|k| grouping * g[k] + (1.0 - grouping) * own[k])
            })
            .collect();
        let mut groups_of: Vec<u16> = group.clone();
        for &(_, host) in &micro_host {
            let own = random_spin(&mut rng);
            let h = spin[host as usize];
            spin.push([0, 1, 2].map(|k| 0.5 * h[k] + 0.5 * own[k]));
            groups_of.push(group[host as usize]);
        }
        let ranks: Vec<u64> = (0..total).map(|_| rng.random::<u64>()).collect();
        let (centres, _) = centres_and_shares(&grid, &owner, total);
        for m in &settings.motions {
            let p = m.plate as usize;
            if p < total {
                spin[p] = spin_for(centres[p], m.heading, m.speed);
            }
        }
        let info: Vec<PlateInfo> = (0..total)
            .map(|p| PlateInfo {
                spin: spin[p],
                group: groups_of[p],
                micro: p >= n,
                centre: centres[p],
                rank: ranks[p],
            })
            .collect();

        // Signed distance from each plate's edge, near its edge.
        let unit = step * radius;
        let cap = FAR * WIDTH + 3.0 * unit;
        let fields: Vec<Vec<(u32, f32)>> = (0..total as u16)
            .into_par_iter()
            .map(|p| distance_field(&grid, &owner, p, radius, cap))
            .collect();
        let mut per_cell: Vec<Vec<(u16, f32)>> = vec![Vec::new(); len];
        for (p, field) in fields.into_iter().enumerate() {
            for (c, d) in field {
                per_cell[c as usize].push((p as u16, d));
            }
        }
        let value = |c: usize, p: u16| -> f64 {
            per_cell[c].iter().find(|e| e.0 == p).map(|e| e.1 as f64).unwrap_or(if owner[c] == p {
                -cap
            } else {
                cap
            })
        };
        let cells: Vec<Vec<Entry>> = (0..len)
            .into_par_iter()
            .map(|c| {
                let pc = grid.position(HexId(c as u32));
                per_cell[c]
                    .iter()
                    .map(|&(p, d)| {
                        // Least-squares slope from the neighbours.
                        let (mut g, mut norm) = ([0.0f64; 3], 0.0);
                        for nb in grid.neighbors(HexId(c as u32)) {
                            let dp = sub(grid.position(nb), pc);
                            let dd = value(nb.index(), p) - d as f64;
                            for k in 0..3 {
                                g[k] += dd * dp[k];
                            }
                            norm += dot(dp, dp);
                        }
                        let grad = g.map(|v| (v / norm.max(1e-12)) as f32);
                        Entry { plate: p, d, grad }
                    })
                    .collect()
            })
            .collect();
        let mut start = Vec::with_capacity(len + 1);
        let mut entries = Vec::new();
        for list in cells {
            start.push(entries.len() as u32);
            entries.extend(list);
        }
        start.push(entries.len() as u32);
        Plates { grid, step, owner, start, entries, cap, info }
    }

    fn entries(&self, c: usize) -> &[Entry] {
        &self.entries[self.start[c] as usize..self.start[c + 1] as usize]
    }

    /// Signed distance and its slope for plate `p` at cell `c`.
    fn field(&self, c: usize, p: u16) -> (f64, [f64; 3]) {
        match self.entries(c).iter().find(|e| e.plate == p) {
            Some(e) => (e.d as f64, e.grad.map(|v| v as f64)),
            None => (if self.owner[c] == p { -self.cap } else { self.cap }, [0.0; 3]),
        }
    }

    /// Nearby plates at unit point `u`, with their interpolated signed
    /// distances and slopes.
    fn near(&self, u: [f64; 3]) -> Vec<(u16, f64, [f64; 3])> {
        let c = self.grid.nearest(u, None);
        // Within one spacing of the point, every cell is the nearest or one
        // of its neighbours, so the stencil never drops a weighted cell.
        let reach = self.step;
        let mut stencil: Vec<(usize, f64)> = Vec::with_capacity(7);
        for cell in std::iter::once(c).chain(self.grid.neighbors(c)) {
            let w = (1.0 - angle(self.grid.position(cell), u) / reach).max(0.0).powi(2);
            if w > 0.0 {
                stencil.push((cell.index(), w));
            }
        }
        if stencil.is_empty() {
            stencil.push((c.index(), 1.0));
        }
        let mut plates: Vec<u16> = Vec::with_capacity(4);
        for &(cell, _) in &stencil {
            for p in self.entries(cell).iter().map(|e| e.plate).chain(std::iter::once(self.owner[cell])) {
                if !plates.contains(&p) {
                    plates.push(p);
                }
            }
        }
        let total: f64 = stencil.iter().map(|s| s.1).sum();
        plates
            .into_iter()
            .map(|p| {
                let (mut d, mut g) = (0.0, [0.0; 3]);
                for &(cell, w) in &stencil {
                    let (dc, gc) = self.field(cell, p);
                    d += w * dc;
                    for k in 0..3 {
                        g[k] += w * gc[k];
                    }
                }
                (p, d / total, g.map(|v| v / total))
            })
            .collect()
    }
}

/// Centre and share of the surface of each plate.
fn centres_and_shares(grid: &Topology, owner: &[u16], plates: usize) -> (Vec<[f64; 3]>, Vec<f64>) {
    let mut sum = vec![[0.0f64; 3]; plates];
    let mut count = vec![0usize; plates];
    for (c, &o) in owner.iter().enumerate() {
        if (o as usize) < plates {
            let p = grid.position(HexId(c as u32));
            for k in 0..3 {
                sum[o as usize][k] += p[k];
            }
            count[o as usize] += 1;
        }
    }
    let centres = sum.into_iter().map(normalize).collect();
    let shares = count.into_iter().map(|n| n as f64 / owner.len() as f64).collect();
    (centres, shares)
}

/// Signed distance (noise-space units) from plate `p`'s edge for cells
/// within `cap` of it: positive outside, negative inside. Measured from
/// the nearest cell on the far side of the edge, less half a cell, so it's
/// ±half a cell either side of the edge and zero on it.
fn distance_field(grid: &Topology, owner: &[u16], p: u16, radius: f64, cap: f64) -> Vec<(u32, f32)> {
    let half = grid.spacing() * radius * 0.5;
    let mut out = Vec::new();
    for inside in [false, true] {
        // Cells on the near side of the edge are the sources; distances
        // spread into the far side.
        let far = |c: usize| (owner[c] == p) == inside;
        let mut heap = BinaryHeap::new();
        let mut done: HashMap<u32, ()> = HashMap::new();
        for c in 0..owner.len() {
            if far(c) {
                continue;
            }
            let id = HexId(c as u32);
            if grid.neighbors(id).any(|nb| far(nb.index())) {
                for nb in grid.neighbors(id).filter(|nb| far(nb.index())) {
                    let d = angle(grid.position(id), grid.position(nb)) * radius;
                    heap.push(Reverse((d.to_bits(), nb.0, c as u32)));
                }
            }
        }
        while let Some(Reverse((key, cell, source))) = heap.pop() {
            if done.insert(cell, ()).is_some() {
                continue;
            }
            let d = f64::from_bits(key) - half;
            out.push((cell, if inside { -d } else { d } as f32));
            let from = grid.position(HexId(source));
            for nb in grid.neighbors(HexId(cell)) {
                if far(nb.index()) && !done.contains_key(&nb.0) {
                    let nd = angle(from, grid.position(nb)) * radius;
                    if nd - half <= cap {
                        heap.push(Reverse((nd.to_bits(), nb.0, source)));
                    }
                }
            }
        }
    }
    out
}

/// Volcanic hot spots scattered over the sea floor: one jittered site per
/// grid cell, some dormant. Where mountain building happens under open ocean
/// (an island arc, a microplate ridge), each site near the boundary raises a
/// round island, so arcs become chains of islands with open water between
/// rather than unbroken lines of land.
pub struct Volcanoes {
    seed: u64,
    /// Spacing of sites, in noise-space units.
    cell: f64,
    /// Island radius, in noise-space units.
    radius: f64,
    three_d: bool,
}

impl Volcanoes {
    pub fn new(seed: u64, cell: f64, three_d: bool) -> Self {
        Volcanoes { seed: rng::mix(seed, 2, 0, rng::purpose::WORLDGEN), cell, radius: cell * 0.38, three_d }
    }

    /// How strongly the sea floor at `p` rises as an island, 0..~1.3.
    pub fn get(&self, p: [f64; 3]) -> f64 {
        let base = p.map(|v| (v / self.cell).floor() as i32);
        let z_range = if self.three_d { -1..=1 } else { 0..=0 };
        let mut best: f64 = 0.0;
        for dz in z_range {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let cell = [base[0] + dx, base[1] + dy, base[2] + dz];
                    let key = |salt: u64| {
                        let packed = (cell[0] as u32 as u64) | (cell[1] as u32 as u64) << 32;
                        let h = rng::mix(self.seed, packed, cell[2] as u32 as u64, salt);
                        (h >> 11) as f64 / (1u64 << 53) as f64
                    };
                    // About a third of sites are dormant; the rest vary in size.
                    let strength = key(0);
                    if strength < 0.35 {
                        continue;
                    }
                    let jitter =
                        |axis: usize, salt: u64| (cell[axis] as f64 + 0.1 + 0.8 * key(salt)) * self.cell;
                    let site = [jitter(0, 1), jitter(1, 2), if self.three_d { jitter(2, 3) } else { 0.0 }];
                    let r = self.radius * (0.6 + 0.6 * strength);
                    best = best.max((0.8 + 0.5 * strength) * bell(dist2(p, site).sqrt() / r));
                }
            }
        }
        best
    }
}

/// What the plates do to the land at one point.
pub struct Tectonics {
    /// The plate the point lies on.
    pub plate: u16,
    /// Signed elevation change from boundary activity (before ridging).
    pub uplift: f64,
    /// Positive uplift that should become rugged ground (mountains/hills).
    pub orogeny: f64,
    /// Signed boundary stress for display: + colliding, − pulling apart.
    pub stress: f64,
}

/// Width of boundary features, in noise-space units.
const WIDTH: f64 = 0.075;

/// Beyond this many widths from a boundary its landforms are negligible.
const FAR: f64 = 4.0;

fn bell(x: f64) -> f64 {
    (-x * x).exp()
}

/// Plate effects at noise-space point `p`. `crust(x)` is the
/// continental-crust field: positive where the crust is continental.
///
/// Each pair of nearby plates contributes the landform of its boundary,
/// weighted by a soft measure of how near both plates are. A pair's
/// landform is written in terms of the signed distance from its boundary,
/// oriented from the higher-ranked plate, with crust types sampled at fixed
/// probe points either side and blended softly. So every term is the same
/// whichever plate the point is in, and nothing switches abruptly.
///
/// `islands` is a 0..1 field of where the sea floor breaks the surface.
/// Mountain building under open ocean (island arcs) is scaled by it, so it
/// rises as chains of separate islands rather than one unbroken line of
/// land along the boundary.
pub fn evaluate(plates: &Plates, p: [f64; 3], crust: impl Fn([f64; 3]) -> f64, islands: f64) -> Tectonics {
    let w = WIDTH;
    let continental = |x: [f64; 3]| smoothstep(-0.08, 0.08, crust(x));
    let land_here = continental(p);
    let surfaces = land_here + (1.0 - land_here) * islands;
    let mut uplift = 0.0;
    let mut orogeny = 0.0;
    let mut stress = 0.0;
    let mut ridge = 0.0;

    let u = normalize(p);
    let near = plates.near(u);
    let own = near.iter().min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0))).map(|n| (n.0, n.1));
    let (own, nearest) = own.unwrap_or((0, 0.0));
    // Soft membership: 1 for the plate the point is in, falling to 1/e
    // about 1.5 feature-widths past a boundary, and exactly zero at FAR.
    let tau = 3.0 * w;
    let limit = 2.0 * FAR * w;
    let floor = (-limit / tau).exp();
    let member = |d: f64| ((((nearest - d) / tau).exp() - floor).max(0.0) / (1.0 - floor)).min(1.0);
    for (i, &(a, da, ga)) in near.iter().enumerate() {
        let ma = member(da);
        if ma <= 1e-4 {
            continue;
        }
        for &(b, db, gb) in &near[i + 1..] {
            let mb = member(db);
            if mb <= 1e-4 {
                continue;
            }
            // A point outside both plates is only near their boundary if
            // it's near both: x alone would put it on a boundary halfway
            // between any two plates it's equally far from.
            let outside = da.min(db).max(0.0);
            let weight = ma * mb * bell(outside / (0.5 * w));
            if weight <= 1e-6 {
                continue;
            }
            let (ia, ib) = (&plates.info[a as usize], &plates.info[b as usize]);
            let ((hi, dh, gh), (lo, dl, gl)) =
                if ia.rank > ib.rank { ((ia, da, ga), (ib, db, gb)) } else { ((ib, db, gb), (ia, da, ga)) };
            // Positive on hi's side.
            let x = (dl - dh) / 2.0;
            if x.abs() > FAR * w {
                continue;
            }
            let across = sub(gl, gh);
            let across = sub(across, u.map(|v| v * dot(across, u)));
            let len = dot(across, across).sqrt();
            if len < 1e-9 {
                continue;
            }
            let toward_hi = across.map(|v| v / len);
            // Across a real boundary the two plates' distances rise in
            // opposite directions; where they rise together the pair's
            // boundary is somewhere else entirely.
            let opposed = len / (dot(gl, gl).sqrt() + dot(gh, gh).sqrt()).max(1e-9);
            let weight = weight * smoothstep(0.3, 0.8, opposed);
            // Colliding when hi moves towards lo faster than lo towards hi.
            let convergence = -dot(sub(cross(hi.spin, u), cross(lo.spin, u)), toward_hi);
            let foot = [0, 1, 2].map(|k| p[k] - toward_hi[k] * x);
            let probe = |t: f64| [0, 1, 2].map(|k| foot[k] + toward_hi[k] * t);
            let c = convergence;
            if c > 0.0 {
                let (hi_cont, lo_cont) = (continental(probe(1.5 * w)), continental(probe(-1.5 * w)));
                // Each case weighted by how well the crust either side fits.
                let both = hi_cont * lo_cont;
                let hi_over_ocean = hi_cont * (1.0 - lo_cont);
                let lo_over_ocean = (1.0 - hi_cont) * lo_cont;
                let oceans = (1.0 - hi_cont) * (1.0 - lo_cont);
                let mut o = 0.0;
                let mut up = 0.0;
                // Continents collide: the greatest ranges, on both sides.
                o += both * 0.8 * bell(x / (1.4 * w));
                // Ocean dives under continent: a range set back from the
                // coast, and a trench offshore.
                o += hi_over_ocean * 0.7 * bell((x - 0.8 * w) / w);
                up -= hi_over_ocean * 0.12 * bell((x + 0.3 * w) / (0.6 * w));
                o += lo_over_ocean * 0.7 * bell((x + 0.8 * w) / w);
                up -= lo_over_ocean * 0.12 * bell((x - 0.3 * w) / (0.6 * w));
                // Ocean meets ocean: the lower plate dives, the higher grows
                // an island arc, broken into separate volcanic islands.
                o += oceans * 0.6 * bell((x - 0.6 * w) / w);
                up -= oceans * 0.1 * bell((x + 0.2 * w) / (0.6 * w));
                orogeny += weight * c * o;
                uplift += weight * c * up;
            } else {
                // Pulling apart: a rift valley on land, a ridge under the sea
                // (which only surfaces at hot spots, like Iceland).
                let land = continental(foot);
                uplift += weight * c * land * 0.1 * bell(x / (0.7 * w));
                ridge -= weight * c * (1.0 - land) * 0.08 * bell(x / w);
            }
            // Plates of one group were welded together long ago: worn hill
            // country along the old seam, on land (the Urals).
            if hi.group == lo.group && !hi.micro && !lo.micro {
                let both = continental(probe(1.5 * w)) * continental(probe(-1.5 * w));
                orogeny += weight * both * 0.18 * bell(x / (1.8 * w));
            }
            stress += weight * c * bell(x / (1.4 * w));
        }
    }

    // Under the sea a ridge lifts the floor everywhere, but only enough to
    // matter at hot spots: elsewhere it stays well below the surface.
    let uplift = uplift + ridge * (0.1 + 0.9 * islands.min(1.0)) * (1.0 - land_here) + ridge * land_here;
    Tectonics { plate: own, uplift, orogeny: orogeny * surfaces, stress }
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIUS: f64 = 1.27;

    fn sizes(plates: &Plates) -> Vec<f64> {
        let (_, shares) = centres_and_shares(&plates.grid, &plates.owner, plates.info.len());
        plates.info.iter().zip(shares).filter(|(p, _)| !p.micro).map(|(_, s)| s).collect()
    }

    #[test]
    fn plates_are_deterministic_and_seeded() {
        let s = PlateSettings::default();
        let (a, b, c) = (Plates::new(1, RADIUS, &s), Plates::new(1, RADIUS, &s), Plates::new(2, RADIUS, &s));
        assert_eq!(a.owner, b.owner);
        assert_ne!(a.owner, c.owner);
    }

    #[test]
    fn every_plate_has_ground_and_microplates_are_small() {
        let s = PlateSettings { microplates: 1.0, ..PlateSettings::default() };
        let plates = Plates::new(5, RADIUS, &s);
        assert!(plates.owner.iter().all(|&o| (o as usize) < plates.info.len()));
        let micro = plates.info.iter().filter(|p| p.micro).count();
        assert!(micro >= 8, "{micro} microplates");
        let mean_major = sizes(&plates).iter().sum::<f64>() / s.count as f64;
        let (_, shares) = centres_and_shares(&plates.grid, &plates.owner, plates.info.len());
        for (p, share) in plates.info.iter().zip(shares).filter(|(p, _)| p.micro) {
            let _ = p;
            assert!(share > 0.0 && share < mean_major * 0.5, "microplate share {share}");
        }
        let none = Plates::new(5, RADIUS, &PlateSettings { microplates: 0.0, ..PlateSettings::default() });
        assert!(none.info.iter().all(|p| !p.micro));
    }

    #[test]
    fn variety_spreads_plate_sizes() {
        let spread = |variety: f64| {
            let s = PlateSettings { variety, microplates: 0.0, ..PlateSettings::default() };
            let mut v = sizes(&Plates::new(9, RADIUS, &s));
            v.sort_by(f64::total_cmp);
            v[v.len() - 1] / v[0].max(1e-6)
        };
        let (even, varied) = (spread(0.0), spread(1.0));
        assert!(varied > even * 2.0, "even {even:.1}×, varied {varied:.1}×");
    }

    #[test]
    fn grouped_plates_move_together() {
        let s = PlateSettings { grouping: 1.0, microplates: 0.0, ..PlateSettings::default() };
        let plates = Plates::new(3, RADIUS, &s);
        for a in &plates.info {
            for b in &plates.info {
                if a.group == b.group {
                    assert!(dist2(a.spin, b.spin) < 1e-12);
                }
            }
        }
    }

    #[test]
    fn motions_can_be_set_by_hand() {
        let s = PlateSettings {
            motions: vec![PlateMotion { plate: 2, heading: 135.0, speed: 0.8 }],
            ..PlateSettings::default()
        };
        let plates = Plates::new(4, RADIUS, &s);
        let (heading, speed) = heading_speed(plates.info[2].spin, plates.info[2].centre);
        assert!((heading - 135.0).abs() < 1e-6 && (speed - 0.8).abs() < 1e-9, "{heading} {speed}");
        // The plates themselves don't change.
        assert_eq!(plates.owner, Plates::new(4, RADIUS, &PlateSettings::default()).owner);
    }

    #[test]
    fn effects_are_continuous_across_boundaries() {
        // Walk a fine great circle across many plates: neighbouring samples
        // must never jump, whatever boundary they straddle.
        let plates = Plates::new(11, RADIUS, &PlateSettings::default());
        let crust = |x: [f64; 3]| (x[0] * 1.3).sin() * 0.3 + (x[1] * 0.7).cos() * 0.2;
        let steps = 40_000;
        let mut prev: Option<(f64, f64)> = None;
        let mut worst: f64 = 0.0;
        for i in 0..steps {
            let a = i as f64 / steps as f64 * TAU;
            let p = normalize([a.cos(), a.sin() * 0.8, a.sin() * 0.6]).map(|v| v * RADIUS);
            let t = evaluate(&plates, p, crust, 1.0);
            let v = (t.uplift, t.orogeny);
            if let Some(pv) = prev {
                worst = worst.max((v.0 - pv.0).abs()).max((v.1 - pv.1).abs());
            }
            prev = Some(v);
        }
        assert!(worst < 0.03, "largest jump between neighbouring samples was {worst}");
    }
}
