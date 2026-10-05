//! Plate tectonics, as a continuous field.
//!
//! Plates are cells of a Voronoi diagram whose sites sit one per cell of a
//! hashed grid, jittered. Because sites come from a hash rather than a list,
//! plates exist everywhere: past the edges of an open map, and seamlessly
//! round a wrapping one. Everything here is a pure function of position, so
//! the same world comes out at any scale.
//!
//! Plates don't decide where land is: as on Earth, continents sit inside
//! plates, and most coasts are not plate boundaries. The caller supplies a
//! continental-crust field; plates only deform the land where they meet.
//! Each plate drifts in some direction, and at a boundary their relative
//! motion, together with the crust on each side, decides the landform:
//!
//! | Meeting | Converging (colliding) | Diverging (pulling apart) |
//! |---|---|---|
//! | continent + continent | great range on both sides | rift valley |
//! | continent + ocean | coastal range inland, trench offshore | rift / ridge |
//! | ocean + ocean | volcanic island arc on one side | mid-ocean ridge |
//!
//! Two layers of plates are used: major plates carry the continents and
//! raise the great ranges, and smaller microplates add secondary ranges and
//! hill country, so even a small region usually has some relief.

use sim_core::rng;

/// One plate site.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    /// Grid cell the site belongs to; identifies the plate.
    pub cell: [i32; 3],
    pos: [f64; 3],
    velocity: [f64; 3],
    /// Breaks ties between two oceanic plates (which one subducts).
    rank: u64,
}

/// One layer of plates.
pub struct PlateLayer {
    seed: u64,
    /// Plate spacing in noise-space units (one unit is a region's height).
    cell: f64,
    /// Whether sites spread in 3D (wrapping maps sample a cylinder) or lie
    /// flat in the map plane.
    three_d: bool,
    /// Sites precomputed over the area the map samples (see
    /// [`PlateLayer::prepare`]); anything outside is computed on demand.
    table: Option<SiteTable>,
}

struct SiteTable {
    min: [i32; 3],
    size: [i32; 3],
    sites: Vec<Site>,
}

/// Cells searched either side of a point's own cell. Two is enough that any
/// site outside the window is provably beyond [`MEMBERSHIP_LIMIT`].
const REACH: i32 = 2;

/// Squared-distance excess (in cells²) beyond which a plate's membership is
/// exactly zero. With sites jittered within the middle 70% of their cells,
/// any site outside a ±2-cell window is at least 2.15 cells away along one
/// axis, while the nearest site is within 0.85·√3 ≈ 1.47 cells, so its
/// excess is at least 2.15² − 1.47² ≈ 2.46 cells².
const MEMBERSHIP_LIMIT: f64 = 2.4;

impl PlateLayer {
    pub fn new(seed: u64, layer: u64, cell: f64, three_d: bool) -> Self {
        PlateLayer { seed: rng::mix(seed, 1, layer, rng::purpose::WORLDGEN), cell, three_d, table: None }
    }

    /// Precomputes the sites for noise-space points between `min` and `max`,
    /// so lookups there are a table read instead of a hash.
    pub fn prepare(&mut self, min: [f64; 3], max: [f64; 3]) {
        let lo = min.map(|v| (v / self.cell).floor() as i32 - REACH - 1);
        let hi = max.map(|v| (v / self.cell).floor() as i32 + REACH + 1);
        let (lo, hi) = if self.three_d { (lo, hi) } else { ([lo[0], lo[1], 0], [hi[0], hi[1], 0]) };
        let size = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
        let mut sites = Vec::with_capacity((size[0] * size[1] * size[2]) as usize);
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    sites.push(self.site([x, y, z]));
                }
            }
        }
        self.table = Some(SiteTable { min: lo, size, sites });
    }

    fn lookup(&self, cell: [i32; 3]) -> Site {
        if let Some(t) = &self.table {
            let local = [cell[0] - t.min[0], cell[1] - t.min[1], cell[2] - t.min[2]];
            if (0..3).all(|i| local[i] >= 0 && local[i] < t.size[i]) {
                return t.sites[((local[2] * t.size[1] + local[1]) * t.size[0] + local[0]) as usize];
            }
        }
        self.site(cell)
    }

    fn site(&self, cell: [i32; 3]) -> Site {
        let key = |salt: u64| {
            let packed = (cell[0] as u32 as u64) | (cell[1] as u32 as u64) << 32;
            rng::mix(self.seed, packed, cell[2] as u32 as u64, salt)
        };
        // Uniform in [0, 1) from the top 53 bits.
        let unit = |salt: u64| (key(salt) >> 11) as f64 / (1u64 << 53) as f64;
        let jitter = |axis: usize, salt: u64| (cell[axis] as f64 + 0.15 + 0.7 * unit(salt)) * self.cell;
        let pos = [jitter(0, 1), jitter(1, 2), if self.three_d { jitter(2, 3) } else { 0.0 }];
        // Drift: a random direction (in the map plane for flat maps) and speed.
        let angle = unit(4) * std::f64::consts::TAU;
        let speed = 0.35 + 0.65 * unit(5);
        let tilt = if self.three_d { unit(6) * 2.0 - 1.0 } else { 0.0 };
        let flat = (1.0 - tilt * tilt).sqrt();
        let velocity = [angle.cos() * flat * speed, angle.sin() * flat * speed, tilt * speed];
        Site { cell, pos, velocity, rank: key(8) }
    }

    /// Sites near `p` with their squared distances, nearest first.
    #[cfg(test)]
    fn neighbourhood(&self, p: [f64; 3]) -> Vec<(Site, f64)> {
        let base = p.map(|v| (v / self.cell).floor() as i32);
        let z_range = if self.three_d { -REACH..=REACH } else { 0..=0 };
        let mut sites = Vec::with_capacity(125);
        for dz in z_range {
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let site = self.lookup([base[0] + dx, base[1] + dy, base[2] + dz]);
                    sites.push((site, dist2(p, site.pos)));
                }
            }
        }
        sites.sort_by(|a, b| a.1.total_cmp(&b.1));
        sites
    }

    /// Soft membership of each nearby plate at `p`: 1 for the nearest,
    /// falling off smoothly for plates whose boundary is further away.
    /// Plates that contribute (nearly) nothing are dropped.
    fn soft(&self, p: [f64; 3]) -> Vec<(Site, f64)> {
        // Falls to 1/e about 1.5 feature-widths from a boundary, and is
        // shifted to reach exactly zero at MEMBERSHIP_LIMIT, so no plate is
        // cut off by the edge of the search window.
        let tau = 3.0 * WIDTH * self.cell;
        let limit = MEMBERSHIP_LIMIT * self.cell * self.cell;
        let floor = (-limit / tau).exp();
        // Two passes over the window: find the nearest site, then keep the
        // few within reach of it. No sorting or per-site allocation.
        let base = p.map(|v| (v / self.cell).floor() as i32);
        let z = if self.three_d { REACH } else { 0 };
        let cells = (-z..=z).flat_map(move |dz| {
            (-REACH..=REACH).flat_map(move |dy| {
                (-REACH..=REACH).map(move |dx| [base[0] + dx, base[1] + dy, base[2] + dz])
            })
        });
        let nearest = cells.clone().map(|c| dist2(p, self.lookup(c).pos)).fold(f64::INFINITY, f64::min);
        let mut out: Vec<(Site, f64)> = Vec::with_capacity(4);
        for c in cells {
            let site = self.lookup(c);
            let excess = dist2(p, site.pos) - nearest;
            if excess >= limit {
                continue;
            }
            let m = ((-excess / tau).exp() - floor).max(0.0) / (1.0 - floor);
            if m > 1e-4 {
                out.push((site, m));
            }
        }
        // The nearest plate first: callers treat it as the point's own.
        if let Some(i) = out.iter().position(|(_, m)| *m >= 1.0 - 1e-12) {
            out.swap(0, i);
        }
        out
    }
}

/// The geometry of one plate pair at a point, oriented from the higher-ranked
/// plate (`hi`), so it's the same whichever plate the point lies in.
struct Pair {
    /// Signed distance from the boundary: positive on `hi`'s side.
    x: f64,
    /// Nearest point on the boundary.
    foot: [f64; 3],
    /// Unit vector from `lo` towards `hi`.
    toward_hi: [f64; 3],
    /// Closing speed: positive when the plates collide.
    convergence: f64,
}

fn pair(p: [f64; 3], a: &Site, b: &Site) -> Pair {
    let (hi, lo) = if a.rank > b.rank { (a, b) } else { (b, a) };
    let gap = sub(hi.pos, lo.pos);
    let len = dot(gap, gap).sqrt().max(1e-9);
    let toward_hi = gap.map(|g| g / len);
    let x = (dist2(p, lo.pos) - dist2(p, hi.pos)) / (2.0 * len);
    let foot = [p[0] - toward_hi[0] * x, p[1] - toward_hi[1] * x, p[2] - toward_hi[2] * x];
    // Colliding when hi moves towards lo faster than lo moves towards hi.
    let convergence = -dot(sub(hi.velocity, lo.velocity), toward_hi);
    Pair { x, foot, toward_hi, convergence }
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
    /// The major plate the point lies on.
    pub plate: Site,
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

/// Plate effects at `p`. `crust(x)` is the continental-crust field: positive
/// where the crust is continental.
///
/// Each pair of nearby plates contributes the landform of its boundary,
/// weighted by a soft measure of how near both plates are. A pair's
/// landform is written in terms of the signed distance from its boundary,
/// oriented from the higher-ranked plate, with crust types sampled at fixed
/// probe points either side and blended softly. So every term is the same
/// whichever plate the point is in, nothing switches abruptly, and the land
/// has no cliffs, even where three plates meet.
///
/// `islands` is a 0..1 field of where the sea floor breaks the surface.
/// Mountain building under open ocean (island arcs, microplate ridges) is
/// scaled by it, so it rises as chains of separate islands rather than one
/// unbroken line of land along the boundary.
pub fn evaluate(
    major: &PlateLayer,
    minor: &PlateLayer,
    p: [f64; 3],
    crust: impl Fn([f64; 3]) -> f64,
    islands: f64,
) -> Tectonics {
    let w = WIDTH;
    let continental = |x: [f64; 3]| smoothstep(-0.08, 0.08, crust(x));
    // Mountains raised from ocean floor (or crust that's barely continental)
    // only break the surface at volcanic islands.
    let land_here = continental(p);
    let surfaces = land_here + (1.0 - land_here) * islands;
    let mut uplift = 0.0;
    let mut orogeny = 0.0;
    let mut stress = 0.0;
    let mut ridge = 0.0;

    let near = major.soft(p);
    for (i, (a, ma)) in near.iter().enumerate() {
        for (b, mb) in &near[i + 1..] {
            let weight = ma * mb;
            let pr = pair(p, a, b);
            let (x, c) = (pr.x, pr.convergence);
            // Every landform below is shaped by a bell that's negligible
            // (< 0.001) this far from the boundary; skip the crust probes.
            if x.abs() > FAR * w {
                continue;
            }
            let probe = |t: f64| {
                let u = pr.toward_hi;
                [pr.foot[0] + u[0] * t, pr.foot[1] + u[1] * t, pr.foot[2] + u[2] * t]
            };
            if c > 0.0 {
                let (hi_cont, lo_cont) = (continental(probe(1.5 * w)), continental(probe(-1.5 * w)));
                // Each case weighted by how well the crust either side fits.
                let both = hi_cont * lo_cont;
                let hi_over_ocean = hi_cont * (1.0 - lo_cont);
                let lo_over_ocean = (1.0 - hi_cont) * lo_cont;
                let oceans = (1.0 - hi_cont) * (1.0 - lo_cont);
                let mut o = 0.0;
                let mut u = 0.0;
                // Continents collide: the greatest ranges, on both sides.
                o += both * 0.8 * bell(x / (1.4 * w));
                // Ocean dives under continent: a range set back from the
                // coast, and a trench offshore.
                o += hi_over_ocean * 0.7 * bell((x - 0.8 * w) / w);
                u -= hi_over_ocean * 0.12 * bell((x + 0.3 * w) / (0.6 * w));
                o += lo_over_ocean * 0.7 * bell((x + 0.8 * w) / w);
                u -= lo_over_ocean * 0.12 * bell((x - 0.3 * w) / (0.6 * w));
                // Ocean meets ocean: the lower plate dives, the higher grows
                // an island arc, broken into separate volcanic islands.
                o += oceans * 0.6 * bell((x - 0.6 * w) / w);
                u -= oceans * 0.1 * bell((x + 0.2 * w) / (0.6 * w));
                orogeny += weight * c * o;
                uplift += weight * c * u;
            } else {
                // Pulling apart: a rift valley on land, a ridge under the sea
                // (which only surfaces at hot spots, like Iceland).
                let land = continental(pr.foot);
                uplift += weight * c * land * 0.1 * bell(x / (0.7 * w));
                ridge -= weight * c * (1.0 - land) * 0.08 * bell(x / w);
            }
            stress += weight * c * bell(x / (1.4 * w));
        }
    }

    // Microplates: secondary ranges and old worn-down hill country, mostly
    // on continents (under open ocean they'd draw stray ridges).
    let on_land = 0.25 + 0.75 * land_here;
    let near_minor = minor.soft(p);
    for (i, (a, ma)) in near_minor.iter().enumerate() {
        for (b, mb) in &near_minor[i + 1..] {
            let pr = pair(p, a, b);
            if pr.x.abs() > FAR * w {
                continue;
            }
            let weight = ma * mb;
            // Shown (more faintly) in the stress map too, so their ranges
            // have a visible cause.
            stress += 0.5 * weight * pr.convergence * bell(pr.x / (0.8 * w));
            if pr.convergence > 0.0 {
                orogeny += on_land * weight * pr.convergence * 0.3 * bell(pr.x / (0.8 * w));
            } else {
                uplift += weight * pr.convergence * 0.06 * bell(pr.x / (0.6 * w));
            }
        }
    }

    // Under the sea a ridge lifts the floor everywhere, but only enough to
    // matter at hot spots: elsewhere it stays well below the surface.
    let uplift = uplift + ridge * (0.1 + 0.9 * islands.min(1.0)) * (1.0 - land_here) + ridge * land_here;
    Tectonics { plate: near[0].0, uplift, orogeny: orogeny * surfaces, stress }
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

    #[test]
    fn pair_geometry_is_the_same_from_either_plate() {
        let layer = PlateLayer::new(7, 0, 1.0, false);
        for i in 0..200 {
            let p = [i as f64 * 0.137 - 10.0, (i * 7 % 23) as f64 * 0.31 - 3.0, 0.0];
            let near = layer.neighbourhood(p);
            let (a, b) = (&near[0].0, &near[1].0);
            let (ab, ba) = (pair(p, a, b), pair(p, b, a));
            assert_eq!(ab.x, ba.x);
            assert_eq!(ab.convergence, ba.convergence);
            // The nearer plate's side of the boundary is where p lies.
            let on_a_side = if a.rank > b.rank { ab.x >= 0.0 } else { ab.x <= 0.0 };
            assert!(on_a_side);
        }
    }

    #[test]
    fn plates_are_deterministic_and_seeded() {
        let p = [0.3, 2.2, -1.4];
        let a = PlateLayer::new(1, 0, 1.0, true).neighbourhood(p);
        let b = PlateLayer::new(1, 0, 1.0, true).neighbourhood(p);
        let c = PlateLayer::new(2, 0, 1.0, true).neighbourhood(p);
        assert_eq!(a[0].0.cell, b[0].0.cell);
        assert_eq!(a[0].1, b[0].1);
        assert_ne!(a[0].1, c[0].1);
    }
}

#[cfg(test)]
mod continuity {
    use super::*;

    #[test]
    fn effects_are_continuous_across_boundaries() {
        // Walk a fine line across many plates: neighbouring samples must
        // never jump, whatever boundary they straddle.
        let major = PlateLayer::new(11, 0, 1.0, false);
        let minor = PlateLayer::new(11, 1, 0.38, false);
        let crust = |x: [f64; 3]| (x[0] * 1.3).sin() * 0.3 + (x[1] * 0.7).cos() * 0.2;
        let step = 0.0005;
        let mut prev: Option<(f64, f64)> = None;
        let mut worst: f64 = 0.0;
        for i in 0..40_000 {
            let p = [i as f64 * step - 10.0, 0.37 + i as f64 * step * 0.31, 0.0];
            let t = evaluate(&major, &minor, p, crust, 1.0);
            let v = (t.uplift, t.orogeny);
            if let Some(pv) = prev {
                worst = worst.max((v.0 - pv.0).abs()).max((v.1 - pv.1).abs());
            }
            prev = Some(v);
        }
        assert!(worst < 0.02, "largest jump between neighbouring samples was {worst}");
    }
}
