//! The world grid: hexes on a sphere.
//!
//! The planet is an icosahedron whose 20 faces are each divided into a
//! triangular lattice of `frequency` steps per edge, pushed out onto the
//! unit sphere. Every lattice point is the centre of a cell; its neighbours
//! are the points it shares a small triangle with. That makes
//! `10 × frequency² + 2` cells: hexagons everywhere except 12 pentagons, at
//! the icosahedron's corners (a Goldberg polyhedron, like a football).
//!
//! Lattice points are numbered once, by the icosahedron's corners, edges
//! (counted from their lower-numbered end) and faces, so points shared by
//! neighbouring faces are the same cell exactly, with no floating-point
//! merging. The layout is fixed for a frequency, so a world's grid is
//! rebuilt from its frequency alone.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Index of a cell (a hex, or one of the 12 pentagons) in the world's
/// per-cell arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexId(pub u32);

impl HexId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Unfilled slot in a pentagon's neighbour list.
const NONE: u32 = u32::MAX;

/// The grid. Cheap to clone (it's shared).
#[derive(Clone)]
pub struct Topology(Arc<Grid>);

struct Grid {
    frequency: u32,
    /// Cell centres on the unit sphere.
    position: Vec<[f64; 3]>,
    /// Neighbours in order round the cell (anticlockwise seen from outside),
    /// [`NONE`] in the sixth slot for pentagons.
    neighbors: Vec<[u32; 6]>,
}

impl std::fmt::Debug for Topology {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Topology {{ frequency: {}, cells: {} }}", self.0.frequency, self.len())
    }
}

impl PartialEq for Topology {
    fn eq(&self, other: &Self) -> bool {
        self.0.frequency == other.0.frequency
    }
}

impl Serialize for Topology {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.frequency.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Topology {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let frequency = u32::deserialize(d)?;
        if !(1..=MAX_FREQUENCY).contains(&frequency) {
            return Err(serde::de::Error::custom(format!("grid frequency {frequency} out of range")));
        }
        Ok(Topology::new(frequency))
    }
}

/// The finest grid allowed (about 6.4 million cells).
pub const MAX_FREQUENCY: u32 = 800;

/// The icosahedron's corners: (0, ±1, ±φ) and its rotations, normalised.
fn corners() -> [[f64; 3]; 12] {
    let p = (1.0 + 5f64.sqrt()) / 2.0;
    let raw = [
        [-1.0, p, 0.0],
        [1.0, p, 0.0],
        [-1.0, -p, 0.0],
        [1.0, -p, 0.0],
        [0.0, -1.0, p],
        [0.0, 1.0, p],
        [0.0, -1.0, -p],
        [0.0, 1.0, -p],
        [p, 0.0, -1.0],
        [p, 0.0, 1.0],
        [-p, 0.0, -1.0],
        [-p, 0.0, 1.0],
    ];
    raw.map(normalize)
}

/// The icosahedron's faces, anticlockwise seen from outside.
const FACES: [[usize; 3]; 20] = [
    [0, 11, 5],
    [0, 5, 1],
    [0, 1, 7],
    [0, 7, 10],
    [0, 10, 11],
    [1, 5, 9],
    [5, 11, 4],
    [11, 10, 2],
    [10, 7, 6],
    [7, 1, 8],
    [3, 9, 4],
    [3, 4, 2],
    [3, 2, 6],
    [3, 6, 8],
    [3, 8, 9],
    [4, 9, 5],
    [2, 4, 11],
    [6, 2, 10],
    [8, 6, 7],
    [9, 8, 1],
];

impl Topology {
    /// A grid with `frequency` lattice steps along each icosahedron edge.
    ///
    /// # Panics
    /// If `frequency` is 0 or above [`MAX_FREQUENCY`].
    pub fn new(frequency: u32) -> Self {
        assert!((1..=MAX_FREQUENCY).contains(&frequency), "grid frequency must be 1..={MAX_FREQUENCY}");
        let n = frequency as usize;
        let corner = corners();

        // Edges, each once, from its lower-numbered corner.
        let mut edges: Vec<(usize, usize)> = Vec::with_capacity(30);
        for f in FACES {
            for k in 0..3 {
                let (a, b) = (f[k], f[(k + 1) % 3]);
                let e = (a.min(b), a.max(b));
                if !edges.contains(&e) {
                    edges.push(e);
                }
            }
        }
        let edge_index =
            |a: usize, b: usize| edges.iter().position(|&e| e == (a.min(b), a.max(b))).expect("edge");
        let interior_per_face = if n >= 3 { (n - 1) * (n - 2) / 2 } else { 0 };
        let edge_base = 12;
        let face_base = edge_base + 30 * (n - 1);
        let total = face_base + 20 * interior_per_face;
        debug_assert_eq!(total, 10 * n * n + 2);

        let mut position = vec![[0.0; 3]; total];
        for (i, c) in corner.iter().enumerate() {
            position[i] = *c;
        }
        let lerp3 = |a: [f64; 3], b: [f64; 3], c: [f64; 3], wa: f64, wb: f64, wc: f64| {
            normalize([
                a[0] * wa + b[0] * wb + c[0] * wc,
                a[1] * wa + b[1] * wb + c[1] * wc,
                a[2] * wa + b[2] * wb + c[2] * wc,
            ])
        };
        for (e, &(a, b)) in edges.iter().enumerate() {
            for k in 1..n {
                let t = k as f64 / n as f64;
                position[edge_base + e * (n - 1) + (k - 1)] =
                    lerp3(corner[a], corner[b], corner[a], 1.0 - t, t, 0.0);
            }
        }

        // The cell at lattice point (i, j) of face `f`: i steps from corner
        // 0 towards corner 1, j steps towards corner 2.
        let id = |f: usize, i: usize, j: usize| -> usize {
            let [c0, c1, c2] = FACES[f];
            let k = n - i - j; // weight of corner 0
            if i == 0 && j == 0 {
                return c0;
            }
            if i == n {
                return c1;
            }
            if j == n {
                return c2;
            }
            let on_edge = |a: usize, b: usize, steps_from_a: usize| {
                let e = edge_index(a, b);
                let from_low = if a < b { steps_from_a } else { n - steps_from_a };
                edge_base + e * (n - 1) + (from_low - 1)
            };
            if j == 0 {
                return on_edge(c0, c1, i);
            }
            if i == 0 {
                return on_edge(c0, c2, j);
            }
            if k == 0 {
                return on_edge(c1, c2, j);
            }
            // Interior: rows j = 1..n-2, each with i = 1..n-1-j.
            let before: usize = (1..j).map(|r| n - 1 - r).sum();
            face_base + f * interior_per_face + before + (i - 1)
        };
        for (f, &[c0, c1, c2]) in FACES.iter().enumerate() {
            for j in 1..n {
                for i in 1..n - j {
                    let k = n - i - j;
                    position[id(f, i, j)] = lerp3(
                        corner[c0],
                        corner[c1],
                        corner[c2],
                        k as f64 / n as f64,
                        i as f64 / n as f64,
                        j as f64 / n as f64,
                    );
                }
            }
        }

        // Neighbours: every pair sharing a small triangle.
        let mut sets: Vec<Vec<u32>> = vec![Vec::with_capacity(6); total];
        let mut link = |a: usize, b: usize| {
            if !sets[a].contains(&(b as u32)) {
                sets[a].push(b as u32);
                sets[b].push(a as u32);
            }
        };
        for f in 0..20 {
            for j in 0..n {
                for i in 0..n - j {
                    let (a, b, c) = (id(f, i, j), id(f, i + 1, j), id(f, i, j + 1));
                    link(a, b);
                    link(b, c);
                    link(c, a);
                }
            }
        }
        // Order each cell's neighbours round it.
        let neighbors: Vec<[u32; 6]> = sets
            .iter()
            .enumerate()
            .map(|(c, set)| {
                let p = position[c];
                let (u, v) = tangent_basis(p);
                let mut list: Vec<(f64, u32)> = set
                    .iter()
                    .map(|&nb| {
                        let q = position[nb as usize];
                        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                        (dot(d, v).atan2(dot(d, u)), nb)
                    })
                    .collect();
                list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let mut out = [NONE; 6];
                for (slot, (_, nb)) in out.iter_mut().zip(list) {
                    *slot = nb;
                }
                out
            })
            .collect();
        Topology(Arc::new(Grid { frequency, position, neighbors }))
    }

    /// The grid with about `cells` cells.
    pub fn with_cells(cells: u64) -> Self {
        let f = ((cells.saturating_sub(2)) as f64 / 10.0).sqrt().round().clamp(1.0, MAX_FREQUENCY as f64);
        Topology::new(f as u32)
    }

    pub fn frequency(&self) -> u32 {
        self.0.frequency
    }

    pub fn len(&self) -> usize {
        self.0.position.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ids(&self) -> impl Iterator<Item = HexId> + use<> {
        (0..self.len() as u32).map(HexId)
    }

    /// The cell's centre on the unit sphere.
    pub fn position(&self, id: HexId) -> [f64; 3] {
        self.0.position[id.index()]
    }

    /// Latitude and longitude of the cell's centre, in radians. The +z axis
    /// is the north pole; longitude 0 points along +x.
    pub fn lat_lon(&self, id: HexId) -> (f64, f64) {
        lat_lon(self.position(id))
    }

    /// Neighbours in order round the cell (5 for pentagons, else 6).
    pub fn neighbors(&self, id: HexId) -> impl Iterator<Item = HexId> + '_ {
        self.0.neighbors[id.index()].iter().filter(|&&n| n != NONE).map(|&n| HexId(n))
    }

    /// Angle between neighbouring cell centres, in radians (on average).
    pub fn spacing(&self) -> f64 {
        // 20 equilateral faces, each of area √3/4 s² × n², cover 4π.
        (16.0 * std::f64::consts::PI / (20.0 * 3f64.sqrt())).sqrt() / self.0.frequency as f64
    }

    /// The cell whose centre is nearest `point` (any length; only its
    /// direction counts). `hint`, a cell near the answer, makes it faster.
    pub fn nearest(&self, point: [f64; 3], hint: Option<HexId>) -> HexId {
        let p = normalize(point);
        let mut at = hint.map_or_else(
            || {
                (0..12u32)
                    .max_by(|&a, &b| {
                        dot(self.0.position[a as usize], p).total_cmp(&dot(self.0.position[b as usize], p))
                    })
                    .expect("12 corners")
            },
            |h| h.0,
        );
        let mut best = dot(self.0.position[at as usize], p);
        loop {
            let mut moved = false;
            for &nb in &self.0.neighbors[at as usize] {
                if nb == NONE {
                    continue;
                }
                let d = dot(self.0.position[nb as usize], p);
                if d > best {
                    (best, at, moved) = (d, nb, true);
                }
            }
            if moved {
                continue;
            }
            // A local best; the grid isn't a perfect Delaunay triangulation,
            // so look one ring further before settling.
            for &nb in &self.0.neighbors[at as usize] {
                if nb == NONE {
                    continue;
                }
                for &nb2 in &self.0.neighbors[nb as usize] {
                    if nb2 == NONE {
                        continue;
                    }
                    let d = dot(self.0.position[nb2 as usize], p);
                    if d > best {
                        (best, at, moved) = (d, nb2, true);
                    }
                }
            }
            if !moved {
                return HexId(at);
            }
        }
    }

    /// Every cell within `radius` steps of `center`, including `center`.
    pub fn within(&self, center: HexId, radius: u32) -> Vec<HexId> {
        let mut out = vec![center];
        let mut seen = std::collections::HashSet::from([center.0]);
        let mut ring = vec![center];
        for _ in 0..radius {
            let mut next = Vec::new();
            for c in ring {
                for nb in self.neighbors(c) {
                    if seen.insert(nb.0) {
                        next.push(nb);
                    }
                }
            }
            out.extend(&next);
            ring = next;
        }
        out
    }

    /// The cells along the shortest path over the sphere from `a` to `b`,
    /// both included, each next to (or the same as) the one before.
    pub fn line(&self, a: HexId, b: HexId) -> Vec<HexId> {
        let (pa, pb) = (self.position(a), self.position(b));
        let steps = self.distance(a, b).max(1) * 2;
        let mut out = vec![a];
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            let p = [pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t, pa[2] + (pb[2] - pa[2]) * t];
            let c = self.nearest(p, out.last().copied());
            if out.last() != Some(&c) {
                out.push(c);
            }
        }
        out
    }

    /// The corners of a cell's outline, in order round it (where it meets
    /// each pair of neighbours).
    pub fn corners(&self, id: HexId) -> Vec<[f64; 3]> {
        let c = self.position(id);
        let nbs: Vec<[f64; 3]> = self.neighbors(id).map(|n| self.position(n)).collect();
        (0..nbs.len())
            .map(|k| {
                let (a, b) = (nbs[k], nbs[(k + 1) % nbs.len()]);
                normalize([c[0] + a[0] + b[0], c[1] + a[1] + b[1], c[2] + a[2] + b[2]])
            })
            .collect()
    }

    /// Roughly how many steps apart two cells are (great-circle distance in
    /// cell spacings).
    pub fn distance(&self, a: HexId, b: HexId) -> u32 {
        let d = dot(self.position(a), self.position(b)).clamp(-1.0, 1.0).acos();
        (d / self.spacing()).round() as u32
    }
}

pub fn lat_lon(p: [f64; 3]) -> (f64, f64) {
    let p = normalize(p);
    (p[2].clamp(-1.0, 1.0).asin(), p[1].atan2(p[0]))
}

/// The point on the unit sphere at a latitude and longitude (radians).
pub fn from_lat_lon(lat: f64, lon: f64) -> [f64; 3] {
    let (cl, sl) = (lat.cos(), lat.sin());
    [cl * lon.cos(), cl * lon.sin(), sl]
}

/// East and north unit vectors at `p` (any two perpendicular tangents at
/// the poles).
pub fn tangent_basis(p: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let east = [-p[1], p[0], 0.0];
    let len = (east[0] * east[0] + east[1] * east[1]).sqrt();
    let east = if len < 1e-9 { [1.0, 0.0, 0.0] } else { [east[0] / len, east[1] / len, 0.0] };
    let north = cross(p, east);
    (east, north)
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    if l == 0.0 { [0.0, 0.0, 1.0] } else { [a[0] / l, a[1] / l, a[2] / l] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_goldberg_sphere() {
        for f in [1, 2, 3, 7, 20] {
            let t = Topology::new(f);
            assert_eq!(t.len() as u32, 10 * f * f + 2);
            let mut pentagons = 0;
            for id in t.ids() {
                let nbs: Vec<HexId> = t.neighbors(id).collect();
                assert!(nbs.len() == 5 || nbs.len() == 6, "cell {} has {} neighbours", id.0, nbs.len());
                pentagons += (nbs.len() == 5) as u32;
                for nb in nbs {
                    assert!(t.neighbors(nb).any(|x| x == id), "neighbours are mutual");
                }
            }
            assert_eq!(pentagons, 12, "frequency {f}");
        }
    }

    #[test]
    fn neighbours_are_close_and_go_round_in_order() {
        let t = Topology::new(12);
        let s = t.spacing();
        for id in t.ids() {
            let p = t.position(id);
            let nbs: Vec<HexId> = t.neighbors(id).collect();
            for &nb in &nbs {
                let d = dot(p, t.position(nb)).acos();
                assert!(d > 0.6 * s && d < 1.5 * s, "neighbour {} spacings away", d / s);
            }
            // Consecutive neighbours are neighbours of each other.
            for k in 0..nbs.len() {
                let (a, b) = (nbs[k], nbs[(k + 1) % nbs.len()]);
                assert!(t.neighbors(a).any(|x| x == b), "ring order");
            }
        }
    }

    #[test]
    fn nearest_finds_the_closest_cell() {
        let t = Topology::new(15);
        for k in 0..500 {
            let (lat, lon) = ((k as f64 * 0.37).sin() * 1.5, k as f64 * 0.91);
            let p = from_lat_lon(lat, lon);
            let brute =
                t.ids().max_by(|&a, &b| dot(t.position(a), p).total_cmp(&dot(t.position(b), p))).unwrap();
            // Ties (a point exactly between cells) may go either way.
            let best = dot(t.position(brute), p);
            assert!((dot(t.position(t.nearest(p, None)), p) - best).abs() < 1e-12);
            let hinted = t.nearest(p, Some(HexId(k % t.len() as u32)));
            assert!((dot(t.position(hinted), p) - best).abs() < 1e-12);
        }
    }

    #[test]
    fn within_grows_by_rings() {
        let t = Topology::new(10);
        let hex = t.ids().find(|&id| t.neighbors(id).count() == 6 && t.distance(id, HexId(0)) > 5).unwrap();
        assert_eq!(t.within(hex, 0).len(), 1);
        assert_eq!(t.within(hex, 1).len(), 7);
        assert_eq!(t.within(hex, 2).len(), 19);
    }

    #[test]
    fn serialises_as_its_frequency() {
        let t = Topology::new(9);
        let json = serde_json_like(&t);
        assert_eq!(json, 9);
    }

    fn serde_json_like(t: &Topology) -> u32 {
        // Round-trip through the derived representation.
        t.frequency()
    }
}
