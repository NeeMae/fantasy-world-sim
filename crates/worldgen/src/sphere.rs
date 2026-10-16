//! Geometry helpers for fields on the sphere grid.
//!
//! Distances in world generation are in noise-space units: the planet is a
//! sphere of radius [`Shape::radius`] in noise space, so rates and lengths
//! mean the same at any grid frequency.

use rayon::prelude::*;
use sim_core::{HexId, Topology, dot};

/// The planet's size in noise space and the grid's spacing on it.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Radius of the planet in noise space.
    pub radius: f64,
    /// Distance between neighbouring cell centres.
    pub spacing: f64,
    /// Area of one cell.
    pub cell_area: f64,
}

impl Shape {
    pub fn new(topo: &Topology, radius: f64) -> Self {
        let spacing = topo.spacing() * radius;
        let cell_area = 4.0 * std::f64::consts::PI * radius * radius / topo.len() as f64;
        Shape { radius, spacing, cell_area }
    }
}

/// Smooths a field over about `distance` (noise-space units): repeated
/// averaging with neighbours, which spreads like a Gaussian of that width.
pub fn smooth(topo: &Topology, shape: &Shape, values: &[f32], distance: f64) -> Vec<f32> {
    // Each half-and-half averaging step spreads a value by about a quarter
    // of a cell spacing squared (variance); a Gaussian of σ = distance / 2
    // needs about this many.
    let sigma = distance / 2.0 / shape.spacing;
    let steps = ((sigma * sigma * 4.0).ceil() as usize).clamp(1, 600);
    let mut current = values.to_vec();
    for _ in 0..steps {
        current = (0..topo.len())
            .into_par_iter()
            .map(|i| {
                let (mut sum, mut count) = (0.0f32, 0.0f32);
                for nb in topo.neighbors(HexId(i as u32)) {
                    sum += current[nb.index()];
                    count += 1.0;
                }
                0.5 * current[i] + 0.5 * sum / count
            })
            .collect();
    }
    current
}

/// Rings of cells round the planet along lines of latitude, each listed
/// west to east, from the south pole up: the grid's equivalent of map rows.
/// Each ring comes with the distance between its cells.
pub fn rings(topo: &Topology, shape: &Shape) -> Vec<(Vec<usize>, f64)> {
    let band = topo.spacing(); // latitude height of a ring, radians
    let count = ((std::f64::consts::PI / band).ceil() as usize).max(1);
    let mut rings: Vec<Vec<(f64, usize)>> = vec![Vec::new(); count];
    for id in topo.ids() {
        let (lat, lon) = topo.lat_lon(id);
        let r = (((lat + std::f64::consts::FRAC_PI_2) / band) as usize).min(count - 1);
        rings[r].push((lon, id.index()));
    }
    rings
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|mut r| {
            r.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mean_lat =
                r.iter().map(|&(_, i)| topo.lat_lon(HexId(i as u32)).0).sum::<f64>() / r.len() as f64;
            let circumference = std::f64::consts::TAU * shape.radius * mean_lat.cos().max(0.05);
            let step = circumference / r.len() as f64;
            (r.into_iter().map(|(_, i)| i).collect(), step)
        })
        .collect()
}

/// Interpolates a field from one sphere grid onto another (usually finer).
pub fn resample(from: &Topology, values: &[f32], to: &Topology) -> Vec<f32> {
    let reach = from.spacing() * 1.5;
    (0..to.len())
        .into_par_iter()
        .map(|i| {
            let p = to.position(HexId(i as u32));
            let near = from.nearest(p, None);
            let (mut sum, mut weight) = (0.0f64, 0.0f64);
            for c in std::iter::once(near).chain(from.neighbors(near)) {
                let d = dot(from.position(c), p).clamp(-1.0, 1.0).acos();
                let w = (1.0 - d / reach).max(0.0).powi(2);
                sum += values[c.index()] as f64 * w;
                weight += w;
            }
            if weight > 0.0 { (sum / weight) as f32 } else { values[near.index()] }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_spreads_a_spike_and_keeps_its_total() {
        let topo = Topology::new(20);
        let shape = Shape::new(&topo, 1.0);
        let mut v = vec![0.0f32; topo.len()];
        v[100] = 1000.0;
        let s = smooth(&topo, &shape, &v, shape.spacing * 4.0);
        assert!(s[100] < 200.0);
        let total: f32 = s.iter().sum();
        assert!((total - 1000.0).abs() < 60.0, "roughly conserved ({total})");
    }

    #[test]
    fn rings_cover_every_cell_once_in_longitude_order() {
        let topo = Topology::new(16);
        let shape = Shape::new(&topo, 1.0);
        let rings = rings(&topo, &shape);
        let mut seen = vec![false; topo.len()];
        for (ring, step) in &rings {
            assert!(*step > 0.0);
            let lons: Vec<f64> = ring.iter().map(|&i| topo.lat_lon(HexId(i as u32)).1).collect();
            assert!(lons.windows(2).all(|w| w[0] <= w[1]));
            for &i in ring {
                assert!(!seen[i]);
                seen[i] = true;
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn resampling_keeps_a_smooth_field() {
        let (a, b) = (Topology::new(10), Topology::new(25));
        let field = |t: &Topology| t.ids().map(|id| t.position(id)[2] as f32).collect::<Vec<_>>();
        let r = resample(&a, &field(&a), &b);
        for (x, y) in r.iter().zip(field(&b)) {
            assert!((x - y).abs() < 0.05);
        }
    }
}
