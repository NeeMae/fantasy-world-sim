//! Telling inland seas from the open ocean.

use sim_core::{HexId, Topology};

/// The most of the world's sea an inland sea can hold; anything bigger is
/// an ocean in its own right, enclosed or not.
const MAX_SHARE: f64 = 0.2;

/// Marks sea hexes in bodies of water cut off from the open ocean (the
/// largest body).
pub fn inland(topo: &Topology, sea: &[bool]) -> Vec<bool> {
    let n = topo.len();
    let leads_on = |_: HexId| false;
    let mut body = vec![u32::MAX; n];
    // (size, reaches an open edge) per body.
    let mut bodies: Vec<(usize, bool)> = Vec::new();
    for start in 0..n {
        if !sea[start] || body[start] != u32::MAX {
            continue;
        }
        let id = bodies.len() as u32;
        body[start] = id;
        let mut stack = vec![start];
        let (mut size, mut edge) = (0, false);
        while let Some(i) = stack.pop() {
            size += 1;
            edge |= leads_on(HexId(i as u32));
            for nb in topo.neighbors(HexId(i as u32)) {
                let j = nb.index();
                if sea[j] && body[j] == u32::MAX {
                    body[j] = id;
                    stack.push(j);
                }
            }
        }
        bodies.push((size, edge));
    }
    let total: usize = bodies.iter().map(|b| b.0).sum();
    let largest = bodies.iter().map(|b| b.0).max().unwrap_or(0);
    let enclosed: Vec<bool> = bodies
        .iter()
        .map(|&(size, edge)| !edge && size < largest && (size as f64) < MAX_SHARE * total as f64)
        .collect();
    (0..n).map(|i| sea[i] && enclosed[body[i] as usize]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosed_seas_are_inland_and_the_ocean_is_not() {
        // Ocean over most of the planet, a band of land round the north,
        // and a small sea inside that land.
        let topo = Topology::new(20);
        let lat = |id: HexId| topo.lat_lon(id).0.to_degrees();
        let lon = |id: HexId| topo.lat_lon(id).1.to_degrees();
        let small_sea = |id: HexId| (lat(id) - 60.0).abs() < 6.0 && lon(id).abs() < 15.0;
        let sea: Vec<bool> = topo.ids().map(|id| lat(id) < 40.0 || small_sea(id)).collect();
        let inland = inland(&topo, &sea);
        let probe = |lat: f64, lon: f64| {
            inland[topo.nearest(sim_core::from_lat_lon(lat.to_radians(), lon.to_radians()), None).index()]
        };
        assert!(probe(60.0, 0.0), "the enclosed sea");
        assert!(!probe(0.0, 0.0), "the ocean");
        assert!(!probe(70.0, 120.0), "land");
    }
}
