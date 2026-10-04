//! Rivers and lakes, from the simulated rainfall.
//!
//! 1. **Drainage.** Every sink is filled (priority flood, after Barnes et
//!    al.) so water can always flow downhill to the sea or off an open edge.
//!    Each land hex drains to its lowest neighbour on the filled surface.
//! 2. **Lakes.** Where filling had to raise the ground noticeably, the
//!    basin holds a lake, provided enough water flows into it to keep it
//!    from drying out. The lake rises from the basin floor until
//!    evaporation balances its inflow, then is trimmed to open water so a
//!    flooded valley floor stays a river.
//! 3. **Discharge.** Rain is gathered downhill: each hex passes everything
//!    it receives, plus its own rainfall times its area, to the hex it
//!    drains into. Big catchments become big rivers.
//!
//! Areas are measured in noise-space units, so the same rivers appear at
//! any scale (finer hexes just trace them more closely).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use sim_core::{Axial, HexId, NO_DRAIN, RIVER_DISCHARGE, Topology, Wrap};
/// How much filling (in elevation percentile) makes a basin a lake.
const LAKE_DEPTH: f64 = 0.0015;
/// Fewer hexes than this is a pond, too small to show: the river just
/// runs through.
const MIN_LAKE: usize = 3;
/// The most hexes a lake without open water in its middle can have.
const MAX_POND: usize = 6;
/// The largest share of its catchment a lake can cover before evaporation
/// from its surface outpaces its inflow.
const LAKE_SHARE: f64 = 0.05;
/// Ground filled more than this lies under a basin's water (well above the
/// tiny slopes the fill adds across flats).
const SUBMERGED: f64 = 0.0003;
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
    let mut drain: Vec<u32> = (0..n)
        .map(|i| if sea[i] { NO_DRAIN } else { downhill(topo, elevation, &filled, i, |_| true) })
        .collect();

    // 3. Gather rain downhill.
    let discharge = accumulate(&drain, sea, rain, cell_area);

    // 2. Lakes. A lake is a whole flooded basin, not the hexes a river
    // happens to cross: find each connected stretch of submerged ground and
    // flood all of it if it's deep enough somewhere and a river's worth of
    // water flows out of it.
    let submerged = |i: usize| !sea[i] && filled[i] - elevation[i] as f64 > SUBMERGED;
    let mut lake = vec![false; n];
    let mut seen = vec![false; n];
    let mut basins = Vec::new();
    for start in 0..n {
        if seen[start] || !submerged(start) {
            continue;
        }
        seen[start] = true;
        let mut basin = vec![start];
        let mut next = 0;
        while next < basin.len() {
            let i = basin[next];
            next += 1;
            for nb in topo.neighbors(HexId(i as u32)) {
                let j = nb.index();
                if !seen[j] && submerged(j) {
                    seen[j] = true;
                    basin.push(j);
                }
            }
        }
        basins.push(basin.clone());
        let depth = basin.iter().map(|&i| filled[i] - elevation[i] as f64).fold(0.0, f64::max);
        let outflow = basin.iter().map(|&i| discharge[i]).fold(0.0, f32::max);
        if depth <= LAKE_DEPTH || outflow <= RIVER_DISCHARGE * LAKE_INFLOW {
            continue;
        }
        // The water rises from the basin's lowest point until evaporation
        // off its surface balances the water flowing in, or it spills.
        let catchment = outflow as f64 / cell_area;
        let room = ((catchment * LAKE_SHARE) as usize).min(basin.len());
        if room < MIN_LAKE {
            continue;
        }
        let lowest =
            *basin.iter().min_by(|&&a, &&b| elevation[a].total_cmp(&elevation[b])).expect("non-empty");
        let mut rising = BinaryHeap::new();
        rising.push(key(elevation[lowest] as f64, lowest));
        lake[lowest] = true;
        let mut size = 0;
        while let Some(Reverse((_, i))) = rising.pop() {
            let i = i as usize;
            size += 1;
            if size == room {
                break;
            }
            for nb in topo.neighbors(HexId(i as u32)) {
                let j = nb.index();
                if submerged(j) && !lake[j] {
                    lake[j] = true;
                    rising.push(key(elevation[j] as f64, j));
                }
            }
        }
        // Hexes queued but not reached stay dry.
        for Reverse((_, i)) in rising {
            lake[i as usize] = false;
        }
    }
    shape_lakes(topo, &mut lake);

    // 4. Water in a basin runs down into its lake (or, in a basin too dry
    // to hold one, its lowest point) rather than straight across the filled
    // surface to the outlet: the outflow from there keeps its course, and
    // the rest of the basin is flooded again from the lake and that
    // outflow, following the real terrain.
    let mut rerouted = vec![false; n];
    let mut level = vec![f64::INFINITY; n];
    let mut queue = BinaryHeap::new();
    for basin in &basins {
        for &i in basin {
            rerouted[i] = true;
        }
        let mut sinks: Vec<usize> = basin.iter().copied().filter(|&i| lake[i]).collect();
        if sinks.is_empty() {
            sinks.extend(basin.iter().copied().min_by(|&a, &b| elevation[a].total_cmp(&elevation[b])));
        }
        for start in sinks {
            // Follow the outflow until it leaves the basin. Drains only go
            // down the filled surface, so this always ends.
            let mut at = start;
            while level[at].is_infinite() {
                level[at] = elevation[at] as f64;
                queue.push(key(level[at], at));
                match drain[at] {
                    NO_DRAIN => break,
                    d if !rerouted[d as usize] => break,
                    d => at = d as usize,
                }
            }
        }
    }
    let fixed: Vec<bool> = level.iter().map(|l| l.is_finite()).collect();
    while let Some(Reverse((bits, i))) = queue.pop() {
        let i = i as usize;
        let at = f64::from_bits(bits);
        if at > level[i] {
            continue;
        }
        for nb in topo.neighbors(HexId(i as u32)) {
            let j = nb.index();
            if !rerouted[j] || fixed[j] {
                continue;
            }
            let raised = (elevation[j] as f64).max(at + EPSILON);
            if raised < level[j] {
                level[j] = raised;
                queue.push(key(raised, j));
            }
        }
    }
    for i in 0..n {
        if rerouted[i] && !fixed[i] {
            drain[i] = downhill(topo, elevation, &level, i, |j| rerouted[j]);
        }
    }
    let discharge = accumulate(&drain, sea, rain, cell_area);

    Hydrology { drain, discharge, lake }
}

/// The neighbour hex `i` drains into: one lower on the `surface` (which
/// guarantees every path ends), preferring the biggest drop in the original
/// ground. On open slopes that's the steepest way down, and across filled
/// basins it follows the old valley floor. Each candidate's drop is scaled
/// by a hashed factor, so on smooth, even slopes rivers wander a little
/// instead of running dead straight.
fn downhill(
    topo: &Topology,
    elevation: &[f32],
    surface: &[f64],
    i: usize,
    allowed: impl Fn(usize) -> bool,
) -> u32 {
    let score = |nb: HexId| {
        let wobble = (sim_core::rng::mix(i as u64, nb.0 as u64, 0, 7) >> 11) as f64 / (1u64 << 53) as f64;
        (elevation[i] - elevation[nb.index()]) as f64 * (0.25 + 1.5 * wobble)
    };
    topo.neighbors(HexId(i as u32))
        .filter(|nb| allowed(nb.index()) && surface[nb.index()] < surface[i])
        .max_by(|&a, &b| score(a).total_cmp(&score(b)).then(b.0.cmp(&a.0)))
        .map_or(NO_DRAIN, |nb| nb.0)
}

/// Gathers rain downhill: each hex passes on its own rain times its area
/// plus everything flowing into it. Hexes are visited once all their
/// upstream hexes are done.
fn accumulate(drain: &[u32], sea: &[bool], rain: &[f32], cell_area: f64) -> Vec<f32> {
    let n = drain.len();
    let mut inflows = vec![0u32; n];
    for &d in drain {
        if d != NO_DRAIN {
            inflows[d as usize] += 1;
        }
    }
    let mut discharge: Vec<f32> =
        (0..n).map(|i| if sea[i] { 0.0 } else { rain[i].max(0.0) * cell_area as f32 }).collect();
    let mut ready: Vec<usize> = (0..n).filter(|&i| inflows[i] == 0).collect();
    while let Some(i) = ready.pop() {
        let d = drain[i];
        if d != NO_DRAIN {
            let d = d as usize;
            discharge[d] += discharge[i];
            inflows[d] -= 1;
            if inflows[d] == 0 {
                ready.push(d);
            }
        }
    }
    discharge
}

/// Trims lakes back to open water, so flooded valley floors one or two hexes
/// wide stay rivers. A lake keeps only the hexes in or next to its
/// *interior* (hexes surrounded by lake on all sides). Lakes too small to
/// have an interior stay if they're a compact pond, at most
/// [`MAX_POND`] hexes with no hex dangling off it.
fn shape_lakes(topo: &Topology, lake: &mut [bool]) {
    let ring = |lake: &[bool], i: usize| {
        let a = topo.axial(HexId(i as u32));
        Axial::DIRECTIONS.map(|d| topo.at_axial(a + d).is_some_and(|h| lake[h.index()]))
    };
    let n = lake.len();
    let interior: Vec<bool> = (0..n).map(|i| lake[i] && ring(lake, i).iter().all(|&l| l)).collect();
    let mut seen = vec![false; n];
    let mut keep = vec![false; n];
    for start in 0..n {
        if seen[start] || !lake[start] {
            continue;
        }
        seen[start] = true;
        let mut part = vec![start];
        let mut next = 0;
        while next < part.len() {
            let i = part[next];
            next += 1;
            for nb in topo.neighbors(HexId(i as u32)) {
                let j = nb.index();
                if lake[j] && !seen[j] {
                    seen[j] = true;
                    part.push(j);
                }
            }
        }
        if part.iter().any(|&i| interior[i]) {
            for &i in &part {
                keep[i] = interior[i] || topo.neighbors(HexId(i as u32)).any(|nb| interior[nb.index()]);
            }
        } else if part.len() <= MAX_POND
            && part.iter().all(|&i| {
                // Its lake neighbours form one arc of at least two hexes.
                let r = ring(lake, i);
                let count = r.iter().filter(|&&l| l).count();
                let arcs = (0..6).filter(|&k| r[k] && !r[(k + 1) % 6]).count();
                count >= 2 && arcs == 1
            })
        {
            for &i in &part {
                keep[i] = true;
            }
        }
    }
    lake.copy_from_slice(&keep);
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
        // A round basin halfway up the valley.
        let centre = topo.at(15, 4).unwrap();
        for id in topo.ids() {
            if topo.distance(centre, id) <= 2 {
                elevation[id.index()] = 0.55 + 0.002 * topo.distance(centre, id) as f32;
            }
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

    #[test]
    fn flooded_trenches_stay_rivers() {
        let (topo, mut elevation, sea) = valley();
        // Raise the basin back out, and cut a trench one hex wide instead.
        for id in topo.ids() {
            let (c, r) = topo.offset(id);
            elevation[id.index()] = 0.5 + c as f32 * 0.01 + (r as f32 - 4.0).abs() * 0.02;
            if r == 4 && (10..22).contains(&c) {
                elevation[id.index()] = 0.5;
            }
        }
        let rain = vec![1.0; topo.len()];
        let h = compute(&topo, &elevation, &sea, &rain, 1.0);
        assert!(h.lake.iter().all(|&l| !l), "a one-hex trench is a river, not a string of lake");
    }

    #[test]
    fn dry_ground_in_a_basin_drains_into_its_lake() {
        let (topo, elevation, sea) = valley();
        let rain = vec![1.0; topo.len()];
        let h = compute(&topo, &elevation, &sea, &rain, 0.05);
        let lake: Vec<usize> = (0..topo.len()).filter(|&i| h.lake[i]).collect();
        assert!(!lake.is_empty() && lake.len() < 19, "a small catchment fills only part of the basin");
        let centre = topo.at(15, 4).unwrap();
        // The basin spills to the west; ground on the upstream side flows
        // into the lake rather than across to the outlet.
        let upstream = |id: HexId| topo.offset(id).0 > 15 && topo.distance(centre, id) <= 2;
        for id in topo.ids().filter(|&id| upstream(id) && !h.lake[id.index()]) {
            let mut at = id.index();
            while !h.lake[at] && h.drain[at] != NO_DRAIN && topo.distance(centre, HexId(at as u32)) <= 2 {
                at = h.drain[at] as usize;
            }
            assert!(h.lake[at], "{:?} drains into the lake", topo.offset(id));
        }
    }
}
