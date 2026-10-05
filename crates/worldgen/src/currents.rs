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
//! its west and to its east along the row: near land on the west means the
//! western side of a basin. That gives a sea-surface temperature anomaly,
//! strongest along coasts and fading towards open ocean.
//!
//! Distances are in noise-space units, so currents look the same at any
//! scale.

use rayon::prelude::*;
use sim_core::{Topology, Wrap};

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
/// row-by-row estimate so small islands don't leave streaks.
const MIXING: f64 = 0.05;

/// Sea-surface temperature anomaly for every hex, about −1 (cold current)
/// to +1 (warm current); 0 on land.
pub fn anomaly(topo: &Topology, sea: &[bool], latitude: &[f32], column_step: f64, row_step: f64) -> Vec<f32> {
    let raw = rows(topo, sea, latitude, column_step);
    // Average over nearby sea only (land doesn't dilute the coast).
    let wet: Vec<f32> = sea.iter().map(|&s| s as u8 as f32).collect();
    let (mut sum, mut weight) = (raw, wet);
    let reach = |step: f64| ((MIXING / step).round() as i32).max(1);
    for _ in 0..2 {
        for (dc, dr) in [(reach(column_step), 0), (0, reach(row_step))] {
            sum = crate::climate::blur(topo, &sum, dc, dr);
            weight = crate::climate::blur(topo, &weight, dc, dr);
        }
    }
    (0..topo.len()).map(|i| if sea[i] { sum[i] / weight[i].max(1e-6) } else { 0.0 }).collect()
}

/// The anomaly from each row on its own.
fn rows(topo: &Topology, sea: &[bool], latitude: &[f32], column_step: f64) -> Vec<f32> {
    let (w, h) = (topo.width() as usize, topo.height() as usize);
    let wraps = topo.wrap() == Wrap::X;
    let rows: Vec<Vec<f32>> = (0..h)
        .into_par_iter()
        .map(|row| {
            let ids: Vec<usize> =
                (0..w).map(|col| topo.at(col as i32, row as i32).expect("on map").index()).collect();
            let west = land_distance(&ids, sea, wraps, false);
            let east = land_distance(&ids, sea, wraps, true);
            ids.iter()
                .enumerate()
                .map(|(c, &i)| {
                    if !sea[i] {
                        return 0.0;
                    }
                    let near =
                        |d: Option<usize>| d.map_or(0.0, |d| (-(d as f64 * column_step) / REACH).exp());
                    (gyre(latitude[i] as f64) * (near(west[c]) - near(east[c]))) as f32
                })
                .collect()
        })
        .collect();
    let mut out = vec![0.0; topo.len()];
    for (row, values) in rows.into_iter().enumerate() {
        for (col, v) in values.into_iter().enumerate() {
            out[topo.at(col as i32, row as i32).expect("on map").index()] = v;
        }
    }
    out
}

/// For each hex in a row (west to east), how many columns away the nearest
/// land is looking west (or east if `eastward`), if any. Off an open map's
/// edge counts as no land.
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

    /// An ocean between two continents, at one latitude.
    fn basin(latitude: f32) -> (Topology, Vec<f32>) {
        let topo = Topology::new(100, 3, Wrap::None);
        let sea: Vec<bool> = topo.ids().map(|id| (10..90).contains(&topo.offset(id).0)).collect();
        let lat = vec![latitude; topo.len()];
        let a = anomaly(&topo, &sea, &lat, 0.02, 0.02);
        (topo, a)
    }

    #[test]
    fn subtropical_basins_run_warm_in_the_west_and_cold_in_the_east() {
        let (topo, a) = basin(30.0);
        let at = |c| a[topo.at(c, 1).unwrap().index()];
        assert!(at(12) > 0.5, "warm current off the eastern coast of the western continent");
        assert!(at(87) < -0.5, "cold current off the western coast of the eastern continent");
        assert!(at(50).abs() < 0.1, "open ocean is barely affected");
        assert_eq!(at(5), 0.0, "land has no sea temperature");
    }

    #[test]
    fn subpolar_basins_turn_the_other_way() {
        let (topo, a) = basin(58.0);
        let at = |c| a[topo.at(c, 1).unwrap().index()];
        assert!(at(87) > 0.4, "the drift warms the eastern side");
        assert!(at(12) < -0.4);
    }
}
