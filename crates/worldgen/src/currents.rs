//! Ocean currents, as the great wind-driven gyres.
//!
//! In each ocean basin the subtropical gyre (roughly 15–45° of latitude)
//! carries warm water poleward along the basin's western side, off eastern
//! coasts (the Gulf Stream, the Kuroshio), and returns it cold towards the
//! equator along the eastern side, off western coasts (the California,
//! Canary, Humboldt and Benguela currents). Further poleward (about 45–70°)
//! the subpolar gyres turn the other way: warm water reaches the eastern
//! side of the basin (the North Atlantic Drift that warms north-west Europe)
//! and cold water runs down the western side (the Labrador current).
//!
//! The model only needs to know, for each sea hex, how near the land is to
//! its west and to its east along its ring of latitude: near land on the west means the
//! western side of a basin. That gives a sea-surface temperature anomaly,
//! strongest along coasts and fading towards open ocean.
//!
//! Distances are in noise-space units, so currents look the same at any
//! scale.

use rayon::prelude::*;
use sim_core::Topology;

use crate::sphere::{Shape, smooth};

/// How far from a coast (noise-space units) a boundary current reaches.
const REACH: f64 = 0.3;

/// The gyre at a latitude: +1 where the western side of a basin runs warm
/// (subtropical), −1 where the eastern side does (subpolar), 0 near the
/// equator and poles.
pub fn gyre(latitude: f64) -> f64 {
    let bell = |x: f64| (-x * x).exp();
    bell((latitude - 30.0) / 13.0) - 0.8 * bell((latitude - 58.0) / 10.0)
}

/// How far currents spread and mix (noise-space units), smoothing out the
/// ring-by-ring estimate so small islands don't leave streaks.
const MIXING: f64 = 0.05;

/// Sea-surface temperature anomaly for every hex, about −1 (cold current)
/// to +1 (warm current); 0 on land. `latitude` is in degrees from the
/// equator (either hemisphere: the gyres mirror each other, so the warm
/// side is the west in both).
pub fn anomaly(
    topo: &Topology,
    shape: &Shape,
    rings: &[(Vec<usize>, f64)],
    sea: &[bool],
    latitude: &[f32],
) -> Vec<f32> {
    let raw = by_ring(topo.len(), rings, sea, latitude);
    // Average over nearby sea only (land doesn't dilute the coast).
    let wet: Vec<f32> = sea.iter().map(|&s| s as u8 as f32).collect();
    let sum = smooth(topo, shape, &raw, MIXING);
    let weight = smooth(topo, shape, &wet, MIXING);
    (0..topo.len()).map(|i| if sea[i] { sum[i] / weight[i].max(1e-6) } else { 0.0 }).collect()
}

/// The anomaly from each ring of latitude on its own.
fn by_ring(n: usize, rings: &[(Vec<usize>, f64)], sea: &[bool], latitude: &[f32]) -> Vec<f32> {
    let values: Vec<Vec<f32>> = rings
        .par_iter()
        .map(|(ids, step)| {
            let west = land_distance(ids, sea, true, false);
            let east = land_distance(ids, sea, true, true);
            ids.iter()
                .enumerate()
                .map(|(c, &i)| {
                    if !sea[i] {
                        return 0.0;
                    }
                    let near = |d: Option<usize>| d.map_or(0.0, |d| (-(d as f64 * step) / REACH).exp());
                    (gyre(latitude[i] as f64) * (near(west[c]) - near(east[c]))) as f32
                })
                .collect()
        })
        .collect();
    let mut out = vec![0.0; n];
    for ((ids, _), values) in rings.iter().zip(values) {
        for (&i, v) in ids.iter().zip(values) {
            out[i] = v;
        }
    }
    out
}

/// For each hex in a ring (west to east), how many cells away the nearest
/// land is looking west (or east if `eastward`), if any.
fn land_distance(ids: &[usize], sea: &[bool], wraps: bool, eastward: bool) -> Vec<Option<usize>> {
    let n = ids.len();
    let order: Vec<usize> = if eastward { (0..n).rev().collect() } else { (0..n).collect() };
    let mut out = vec![None; n];
    let mut last: Option<usize> = None;
    // A wrapping row is walked twice so land past the seam is seen.
    let laps = if wraps { 2 } else { 1 };
    for lap in 0..laps {
        for (step, &c) in order.iter().enumerate() {
            let at = lap * n + step;
            if !sea[ids[c]] {
                last = Some(at);
            }
            out[c] = last.map(|l| at - l);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphere::rings;
    use sim_core::HexId;

    /// An ocean between two continents (each 30° of longitude wide).
    fn basin() -> (Topology, Vec<f32>) {
        let topo = Topology::new(30);
        let shape = Shape::new(&topo, 1.5);
        let lon = |id: HexId| topo.lat_lon(id).1.to_degrees();
        let sea: Vec<bool> =
            topo.ids().map(|id| !(lon(id).abs() < 15.0 || (lon(id) - 120.0).abs() < 15.0)).collect();
        let lat: Vec<f32> = topo.ids().map(|id| topo.lat_lon(id).0.to_degrees().abs() as f32).collect();
        let a = anomaly(&topo, &shape, &rings(&topo, &shape), &sea, &lat);
        (topo, a)
    }

    fn at(topo: &Topology, a: &[f32], lat: f64, lon: f64) -> f32 {
        a[topo.nearest(sim_core::from_lat_lon(lat.to_radians(), lon.to_radians()), None).index()]
    }

    #[test]
    fn subtropical_basins_run_warm_in_the_west_and_cold_in_the_east() {
        let (topo, a) = basin();
        // The basin between the continents runs from 15° to 105° east.
        assert!(at(&topo, &a, 30.0, 20.0) > 0.3, "warm off the western continent's east coast");
        assert!(at(&topo, &a, 30.0, 100.0) < -0.3, "cold off the eastern continent's west coast");
        assert!(at(&topo, &a, -30.0, 20.0) > 0.3, "the same in the south");
        assert_eq!(at(&topo, &a, 30.0, 0.0), 0.0, "land has no sea temperature");
    }

    #[test]
    fn subpolar_basins_turn_the_other_way() {
        let (topo, a) = basin();
        assert!(at(&topo, &a, 58.0, 100.0) > 0.2, "the drift warms the eastern side");
        assert!(at(&topo, &a, 58.0, 20.0) < -0.2);
    }
}
