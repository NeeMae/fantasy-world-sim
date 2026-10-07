//! Simulated geology: mountains raised and carved over time.
//!
//! A small landscape-evolution model, in the spirit of Braun & Willett
//! (2013). Starting from the low, broad relief of the continental crust, the
//! plates lift the land a little each step while rivers cut it down:
//!
//! * **Uplift.** Each hex rises by its share of the tectonic uplift, spread
//!   evenly over the run.
//! * **River incision** (stream power). Each land hex wears down towards the
//!   hex it drains into at a rate growing with √(upstream area) × slope,
//!   solved implicitly from the coast upwards so even big steps are stable.
//!   Big rivers cut deep valleys; ranges break into branching ridges.
//! * **Hillslope creep.** A little diffusion rounds off ridge crests and
//!   valley sides.
//!
//! The sea is the base level rivers cut down towards. Everything runs on a
//! fixed-resolution grid (the caller's), so the same world comes out at any
//! map scale.

use rayon::prelude::*;
use sim_core::{HexId, NO_DRAIN, Topology};

use crate::hydrology;

/// Steps of uplift and erosion.
pub const STEPS: usize = 200;
/// Stream-power erodibility, per unit time and √area.
const ERODIBILITY: f64 = 12.0;
/// Hillslope diffusivity (per unit time, area units).
const CREEP: f64 = 2e-6;

pub struct Input<'a> {
    pub topology: &'a Topology,
    /// Starting heights (raw units).
    pub base: &'a [f32],
    /// Total uplift over the whole run, per hex.
    pub uplift: &'a [f32],
    /// Raw height of the sea surface.
    pub sea_level: f32,
    /// One hex's area and the distance between hex centres, noise-space units.
    pub cell_area: f64,
    pub spacing: f64,
    /// Scales erosion: 0 is uplift alone, 1 normal, 2 deeply carved.
    pub erosion: f64,
    /// Steps to run, each a 1/[`STEPS`] of the full run: [`STEPS`] for the
    /// whole history, fewer to settle a surface briefly.
    pub steps: usize,
}

/// Runs the model and returns the final heights. `progress` is called now
/// and then with the fraction done and the heights so far.
pub fn evolve(input: &Input, progress: &mut dyn FnMut(f32, &[f32])) -> Vec<f32> {
    let topo = input.topology;
    let n = topo.len();
    let dt = 1.0 / STEPS as f64;
    let steps = input.steps;
    let mut h: Vec<f64> = input.base.iter().map(|&b| b as f64).collect();
    let rain = vec![1.0f32; n];
    let routing = hydrology::Routing { merge: 0.0, ..Default::default() };
    let k = ERODIBILITY * input.erosion.max(0.0) * dt / input.spacing;
    let creep = (CREEP * input.erosion.max(0.0) * dt / (input.spacing * input.spacing)).min(0.15);
    let sea = input.sea_level as f64;

    for step in 0..steps {
        for (h, u) in h.iter_mut().zip(input.uplift) {
            *h += *u as f64 * dt;
        }
        if k > 0.0 {
            let floor = h.iter().copied().fold(f64::INFINITY, f64::min);
            let shifted: Vec<f32> = h.iter().map(|v| (v - floor) as f32).collect();
            let below: Vec<bool> = h.iter().map(|&v| v < sea).collect();
            let flow = hydrology::drainage(topo, &shifted, &below, &rain, input.cell_area, routing);
            // Receivers before donors: drains always go down the filled
            // surface, so sorting by it gives a valid order.
            let mut order: Vec<u32> = (0..n as u32).filter(|&i| !below[i as usize]).collect();
            order.par_sort_unstable_by(|&a, &b| {
                flow.filled[a as usize].total_cmp(&flow.filled[b as usize]).then(a.cmp(&b))
            });
            for &i in &order {
                let i = i as usize;
                let r = flow.drain[i];
                if r == NO_DRAIN {
                    continue;
                }
                let base = h[r as usize].max(sea.min(h[i]));
                if h[i] <= base {
                    continue;
                }
                // Implicit stream power (n = 1): solve
                // (h' - h) / dt = -K √A (h' - h_r) / dx for h'.
                let f = k * (flow.discharge[i] as f64).sqrt();
                h[i] = (h[i] + f * base) / (1.0 + f);
            }
        }
        if creep > 0.0 {
            let old = h.clone();
            h = (0..n)
                .into_par_iter()
                .map(|i| {
                    if old[i] < sea {
                        return old[i];
                    }
                    let (mut sum, mut count) = (0.0, 0.0);
                    for nb in topo.neighbors(HexId(i as u32)) {
                        sum += old[nb.index()];
                        count += 1.0;
                    }
                    old[i] + creep * (sum - count * old[i])
                })
                .collect();
        }
        if step % 10 == 9 {
            let snapshot: Vec<f32> = h.iter().map(|&v| v as f32).collect();
            progress((step + 1) as f32 / steps as f32, &snapshot);
        }
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// How steep the ground around each hex is: the largest height difference to
/// a neighbour, per noise-space unit.
pub fn slope(topo: &Topology, h: &[f32], spacing: f64) -> Vec<f32> {
    (0..topo.len())
        .into_par_iter()
        .map(|i| {
            let mut best = 0.0f32;
            for nb in topo.neighbors(HexId(i as u32)) {
                best = best.max((h[i] - h[nb.index()]).abs());
            }
            (best as f64 / spacing) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::Wrap;

    #[test]
    fn uplift_raises_a_range_and_rivers_carve_it() {
        // An island lifted in a band down the middle.
        let topo = Topology::new(60, 40, Wrap::None);
        let n = topo.len();
        let mut base = vec![0.0f32; n];
        let mut uplift = vec![0.0f32; n];
        for id in topo.ids() {
            let (c, r) = topo.offset(id);
            let inland = (c.min(59 - c).min(r).min(39 - r)) as f32;
            base[id.index()] = -0.2 + 0.03 * inland;
            uplift[id.index()] = (-(((c as f32 - 30.0) / 6.0).powi(2))).exp();
        }
        let input = |erosion| Input {
            topology: &topo,
            base: &base,
            uplift: &uplift,
            sea_level: 0.0,
            cell_area: 1e-4,
            spacing: 0.01,
            erosion,
            steps: STEPS,
        };
        let raised = evolve(&input(0.0), &mut |_, _| {});
        let carved = evolve(&input(1.0), &mut |_, _| {});
        let mid = topo.at(30, 20).unwrap().index();
        assert!(raised[mid] > 0.9, "uplift alone raises the range");
        let worn = (0..n).filter(|&i| carved[i] < raised[i] - 0.05).count();
        assert!(worn > n / 10, "rivers wear the range down ({worn} hexes)");
        assert!(carved[mid] > 0.0, "the range still stands");
        // Valleys: along the crest, heights now vary.
        let crest: Vec<f32> = (2..38).map(|r| carved[topo.at(30, r).unwrap().index()]).collect();
        let (lo, hi) = crest.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 0.1, "the crest is cut by valleys ({lo}..{hi})");
    }
}
