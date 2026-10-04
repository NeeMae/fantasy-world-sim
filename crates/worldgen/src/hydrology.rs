//! Rivers and lakes, from the simulated rainfall.
//!
//! 1. **Drainage.** Every sink is filled (priority flood, after Barnes et
//!    al.) so water can always flow downhill to the sea or off an open edge.
//!    Each land hex drains to its lowest neighbour on the filled surface.
//! 2. **Lakes.** Where filling had to raise the ground noticeably, the
//!    basin holds a lake, provided enough water flows into it to keep it
//!    from drying out.
//! 3. **Discharge.** Rain is gathered downhill: each hex passes everything
//!    it receives, plus its own rainfall times its area, to the hex it
//!    drains into. Big catchments become big rivers.
//!
//! Areas are measured in noise-space units, so the same rivers appear at
//! any scale (finer hexes just trace them more closely).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use sim_core::{HexId, NO_DRAIN, RIVER_DISCHARGE, Topology, Wrap};
/// How much filling (in elevation percentile) makes a basin a lake.
const LAKE_DEPTH: f64 = 0.0015;
/// A basin needs this share of a river's discharge flowing in to stay wet.
const LAKE_INFLOW: f32 = 0.25;
/// The smallest rise forced between a hex and the one it drains into, so
/// filled flats still slope towards their outlet.
const EPSILON: f64 = 1e-7;

pub struct Hydrology {
    /// The hex each hex drains into, or [`NO_DRAIN`].
    pub drain: Vec<u32>,
    /// Water flowing out of each hex.
    pub discharge: Vec<f32>,
    /// Hexes under a lake.
    pub lake: Vec<bool>,
}

/// `elevation` is the percentile elevation; `sea` marks sea hexes; `rain`
/// is rainfall intensity; `cell_area` is one hex's area in noise space.
pub fn compute(topo: &Topology, elevation: &[f32], sea: &[bool], rain: &[f32], cell_area: f64) -> Hydrology {
    let n = topo.len();
    let (w, h) = (topo.width() as i32, topo.height() as i32);
    let on_edge = |id: HexId| {
        let (col, row) = topo.offset(id);
        row == 0 || row == h - 1 || (topo.wrap() == Wrap::None && (col == 0 || col == w - 1))
    };

    // 1. Priority flood from every outlet: the sea, and land at open edges
    // (the rest of the world lies beyond them).
    let mut filled = vec![f64::INFINITY; n];
    let mut done = vec![false; n];
    let mut queue = BinaryHeap::new();
    let key = |v: f64, i: usize| Reverse((v.to_bits(), i as u32)); // v >= 0, so bits order like values
    for id in topo.ids() {
        let i = id.index();
        if sea[i] || on_edge(id) {
            filled[i] = elevation[i] as f64;
            queue.push(key(filled[i], i));
        }
    }
    while let Some(Reverse((bits, i))) = queue.pop() {
        let i = i as usize;
        if done[i] {
            continue;
        }
        done[i] = true;
        let level = f64::from_bits(bits);
        for nb in topo.neighbors(HexId(i as u32)) {
            let j = nb.index();
            if done[j] || sea[j] {
                continue;
            }
            let raised = (elevation[j] as f64).max(level + EPSILON);
            if raised < filled[j] {
                filled[j] = raised;
                queue.push(key(raised, j));
            }
        }
    }

    // Each land hex drains to a neighbour lower on the filled surface, which
    // guarantees every path reaches an outlet. Among those it prefers the
    // biggest drop in the *original* ground: on open slopes that's the
    // steepest way down, and across filled basins it follows the old valley
    // floor instead of a dead-straight line to the outlet. Edge hexes with
    // no lower neighbour drain off the map.
    let drain: Vec<u32> = (0..n)
        .map(|i| {
            if sea[i] {
                return NO_DRAIN;
            }
            // Each candidate's drop is scaled by a hashed factor, so on smooth,
            // even slopes rivers wander a little instead of running dead
            // straight, while still always heading downhill.
            let score = |nb: HexId| {
                let wobble =
                    (sim_core::rng::mix(i as u64, nb.0 as u64, 0, 7) >> 11) as f64 / (1u64 << 53) as f64;
                (elevation[i] - elevation[nb.index()]) as f64 * (0.25 + 1.5 * wobble)
            };
            topo.neighbors(HexId(i as u32))
                .filter(|nb| filled[nb.index()] < filled[i])
                .max_by(|&a, &b| score(a).total_cmp(&score(b)).then(b.0.cmp(&a.0)))
                .map_or(NO_DRAIN, |nb| nb.0)
        })
        .collect();

    // 3. Gather rain downhill, highest first.
    let mut order: Vec<usize> = (0..n).filter(|&i| !sea[i]).collect();
    order.sort_by(|&a, &b| filled[b].total_cmp(&filled[a]).then(a.cmp(&b)));
    let mut discharge = vec![0.0f32; n];
    for &i in &order {
        discharge[i] += rain[i].max(0.0) * cell_area as f32;
        if drain[i] != NO_DRAIN {
            let d = discharge[i];
            discharge[drain[i] as usize] += d;
        }
    }

    // 2. Lakes: deep enough basins with enough water flowing through.
    let lake = (0..n)
        .map(|i| {
            !sea[i]
                && filled[i] - elevation[i] as f64 > LAKE_DEPTH
                && discharge[i] > RIVER_DISCHARGE * LAKE_INFLOW
        })
        .collect();

    Hydrology { drain, discharge, lake }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valley sloping down to the sea in the west, with a sink in it.
    fn valley() -> (Topology, Vec<f32>, Vec<bool>) {
        let topo = Topology::new(30, 9, Wrap::None);
        let mut elevation = vec![0.0; topo.len()];
        let mut sea = vec![false; topo.len()];
        for id in topo.ids() {
            let (c, r) = topo.offset(id);
            let across = (r as f32 - 4.0).abs() * 0.02; // valley floor in row 4
            elevation[id.index()] = 0.5 + c as f32 * 0.01 + across;
            if c < 3 {
                sea[id.index()] = true;
                elevation[id.index()] = 0.2;
            }
        }
        // A basin halfway up the valley.
        for (c, r) in [(15, 4), (16, 4), (15, 3), (16, 3)] {
            elevation[topo.at(c, r).unwrap().index()] = 0.55;
        }
        (topo, elevation, sea)
    }

    #[test]
    fn everything_drains_to_the_sea_or_off_the_map() {
        let (topo, elevation, sea) = valley();
        let rain = vec![1.0; topo.len()];
        let h = compute(&topo, &elevation, &sea, &rain, 1.0);
        for id in topo.ids() {
            let mut at = id.index();
            for _ in 0..topo.len() {
                if h.drain[at] == NO_DRAIN {
                    break;
                }
                at = h.drain[at] as usize;
            }
            assert_eq!(h.drain[at], NO_DRAIN, "no cycles: every path ends");
        }
    }

    #[test]
    fn discharge_grows_downstream_and_the_basin_fills() {
        let (topo, elevation, sea) = valley();
        let rain = vec![1.0; topo.len()];
        let h = compute(&topo, &elevation, &sea, &rain, 1.0);
        let at = |c, r| topo.at(c, r).unwrap().index();
        assert!(h.discharge[at(5, 4)] > h.discharge[at(20, 4)], "more water lower down the valley");
        assert!(h.discharge[at(5, 4)] > 50.0, "the valley floor collects most of the catchment");
        assert!(h.lake[at(15, 4)], "the basin holds a lake");
        assert!(!h.lake[at(25, 4)]);
    }

    #[test]
    fn dry_basins_stay_dry() {
        let (topo, elevation, sea) = valley();
        let rain = vec![1e-6; topo.len()];
        let h = compute(&topo, &elevation, &sea, &rain, 1.0);
        assert!(h.lake.iter().all(|&l| !l));
    }
}
