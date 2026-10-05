//! Telling inland seas from the open ocean.

use sim_core::{HexId, Topology, Wrap};

/// The most of the world's sea an inland sea can hold; anything bigger is
/// an ocean in its own right, enclosed or not.
const MAX_SHARE: f64 = 0.2;

/// Marks sea hexes in bodies of water cut off from the open ocean. The
/// ocean is the largest body, and any body reaching an edge of the map
/// that leads on (every edge of an open map, the north and south of a
/// wrapping one), since it may join the ocean beyond.
pub fn inland(topo: &Topology, sea: &[bool], open_edges: bool) -> Vec<bool> {
    let n = topo.len();
    let (w, h) = (topo.width() as i32, topo.height() as i32);
    let leads_on = |id: HexId| {
        let (c, r) = topo.offset(id);
        open_edges && (r == 0 || r == h - 1 || (topo.wrap() == Wrap::None && (c == 0 || c == w - 1)))
    };
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
        // Ocean in the west; a walled sea in the east.
        let topo = Topology::new(40, 20, Wrap::None);
        let sea: Vec<bool> = topo
            .ids()
            .map(|id| {
                let (c, r) = topo.offset(id);
                c < 15 || ((25..32).contains(&c) && (6..12).contains(&r))
            })
            .collect();
        let inland = inland(&topo, &sea, true);
        let at = |c, r| inland[topo.at(c, r).unwrap().index()];
        assert!(at(28, 8));
        assert!(!at(5, 5), "the ocean");
        assert!(!at(20, 5), "land");
    }

    #[test]
    fn a_sea_reaching_an_open_edge_may_join_the_ocean_beyond() {
        let topo = Topology::new(40, 20, Wrap::None);
        let sea: Vec<bool> = topo
            .ids()
            .map(|id| {
                let (c, r) = topo.offset(id);
                c < 15 || ((25..32).contains(&c) && r < 6)
            })
            .collect();
        assert!(!inland(&topo, &sea, true)[topo.at(28, 2).unwrap().index()]);
        // With an ocean ring the edge is just more of the map.
        assert!(inland(&topo, &sea, false)[topo.at(28, 2).unwrap().index()]);
    }
}
