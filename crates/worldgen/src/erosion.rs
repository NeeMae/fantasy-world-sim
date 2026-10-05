//! Erosion: rivers carving valleys, slopes slumping, basins silting up.
//!
//! A few rounds of a stream-power model on the raw terrain, before it's
//! ranked into percentiles. Each round finds where water flows (with
//! uniform rain: climate comes later), then:
//!
//! * **Incision.** Each hex wears down towards the one it drains into, by a
//!   share of the drop growing with the water flowing through it
//!   (√discharge), so steep beds cut fastest. Big rivers cut valleys; ranges break up into ridges and spurs.
//! * **Slumping.** Ground steeper than the angle of repose slides downhill
//!   onto its lowest neighbour.
//! * **Silting.** Sinks fill partway towards their spill level, so closed
//!   basins flatten into plains rather than pits.
//!
//! Rates are per noise-space unit, so the same world erodes the same way at
//! any scale.

use rayon::prelude::*;
use sim_core::{HexId, NO_DRAIN, RIVER_DISCHARGE, Topology};

use crate::hydrology;

/// Rounds of erosion.
const ROUNDS: usize = 10;
/// Share of the drop to its drain a hex at river size loses per round;
/// more for bigger rivers (growing with √discharge), up to [`MAX_CUT`].
const INCISION: f64 = 0.15;
const MAX_CUT: f64 = 0.7;
/// The deepest a river cuts in one round (raw height), so cliffs are worn
/// back gradually rather than notched.
const MAX_DEPTH: f64 = 0.02;
/// Discharge at which gathered water starts cutting a channel: streams
/// well below river size, so mountain valleys are carved too.
const CHANNEL: f64 = RIVER_DISCHARGE as f64 * 0.1;
/// Steepest stable slope (height per noise-space unit).
const REPOSE: f64 = 2.5;
/// Share of the excess a too-steep slope sheds per round.
const SLUMP: f64 = 0.25;
/// Share of the way a sink silts up towards its spill level per round.
const SILT: f64 = 0.15;

/// Erodes `elevation` (raw heights) in place. `sea_level` is the raw height
/// of the coast; the sea floor isn't eroded. `cell_area` is one hex's area
/// and `spacing` the distance between hex centres, in noise-space units.
/// `strength` scales everything (0 turns erosion off).
pub fn erode(
    topo: &Topology,
    elevation: &mut [f32],
    sea_level: f32,
    cell_area: f64,
    spacing: f64,
    strength: f64,
) {
    if strength <= 0.0 {
        return;
    }
    let n = topo.len();
    let rain = vec![1.0f32; n];
    let floor = elevation.iter().copied().fold(f32::INFINITY, f32::min);
    for _ in 0..ROUNDS {
        let sea: Vec<bool> = elevation.iter().map(|&e| e < sea_level).collect();
        // Drainage needs non-negative heights only for its own bookkeeping;
        // shifting doesn't change where water goes.
        let shifted: Vec<f32> = elevation.iter().map(|e| e - floor).collect();
        let flow = hydrology::drainage(topo, &shifted, &sea, &rain, cell_area);
        let old = elevation.to_vec();
        let next: Vec<f32> = (0..n)
            .into_par_iter()
            .map(|i| {
                let e = old[i] as f64;
                if sea[i] {
                    return old[i];
                }
                let mut change = 0.0;
                // Incision towards the drain, never below it.
                let d = flow.drain[i];
                if d != NO_DRAIN {
                    let drop = (e - old[d as usize] as f64).max(0.0);
                    // Only gathered water cuts: rills on open slopes don't.
                    let size = ((flow.discharge[i] as f64 / CHANNEL).sqrt() - 1.0).max(0.0);
                    change -= (drop * (INCISION * strength * size).min(MAX_CUT)).min(MAX_DEPTH * strength);
                }
                // Silting in sinks.
                let fill = flow.filled[i] + floor as f64 - e;
                if fill > 0.0 {
                    change += fill * SILT * strength.min(1.0);
                }
                // Slumping: shed onto lower neighbours where too steep, and
                // receive from higher ones (symmetric, so mass is kept).
                for nb in topo.neighbors(HexId(i as u32)) {
                    let o = old[nb.index()] as f64;
                    let excess = ((e - o).abs() - REPOSE * spacing).max(0.0);
                    if excess > 0.0 && !sea[nb.index()] {
                        change += if o > e { 1.0 } else { -1.0 } * excess * SLUMP / 6.0;
                    }
                }
                (e + change) as f32
            })
            .collect();
        elevation.copy_from_slice(&next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::Wrap;

    #[test]
    fn rivers_cut_valleys_and_the_sea_floor_is_left_alone() {
        // A cone of land rising from the sea.
        let topo = Topology::new(40, 40, Wrap::None);
        let centre = topo.at(20, 20).unwrap();
        let mut elevation: Vec<f32> =
            topo.ids().map(|id| 1.0 - topo.distance(centre, id) as f32 * 0.06).collect();
        let before = elevation.clone();
        erode(&topo, &mut elevation, 0.0, 0.0004, 0.02, 1.0);
        let lowered = (0..topo.len()).filter(|&i| elevation[i] < before[i] - 1e-4).count();
        assert!(lowered > 50, "erosion wears the land down ({lowered} hexes)");
        for i in 0..topo.len() {
            if before[i] < 0.0 {
                assert_eq!(elevation[i], before[i], "the sea floor is untouched");
            }
        }
        // The peak stays the peak.
        let top = (0..topo.len()).max_by(|&a, &b| elevation[a].total_cmp(&elevation[b])).unwrap();
        assert!(topo.distance(centre, HexId(top as u32)) <= 2);
    }
}
