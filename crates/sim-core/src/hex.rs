//! Hex grid addressing.
//!
//! The map is a `width × height` grid of flat-topped hexes in "odd-q" offset
//! layout: columns run left to right, and odd columns sit half a hex lower.
//! Storage is row-major, so [`HexId`] is `row * width + col`.
//!
//! All neighbour and distance queries go through [`Topology`] so that the
//! bounded map and the east-west wrapping map share every system.

use serde::{Deserialize, Serialize};

/// Index of a hex in the world's per-hex arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexId(pub u32);

impl HexId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Axial hex coordinates, for arithmetic. `s` is implicit (`-q - r`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Axial {
    pub q: i32,
    pub r: i32,
}

impl Axial {
    /// The six neighbour offsets, clockwise from "lower right".
    pub const DIRECTIONS: [Axial; 6] = [
        Axial { q: 1, r: 0 },
        Axial { q: 0, r: 1 },
        Axial { q: -1, r: 1 },
        Axial { q: -1, r: 0 },
        Axial { q: 0, r: -1 },
        Axial { q: 1, r: -1 },
    ];

    pub fn new(q: i32, r: i32) -> Self {
        Axial { q, r }
    }

    pub fn distance(self, other: Axial) -> u32 {
        let dq = self.q - other.q;
        let dr = self.r - other.r;
        ((dq.abs() + dr.abs() + (dq + dr).abs()) / 2) as u32
    }

    pub fn from_offset(col: i32, row: i32) -> Self {
        Axial { q: col, r: row - (col - (col & 1)) / 2 }
    }

    pub fn to_offset(self) -> (i32, i32) {
        (self.q, self.r + (self.q - (self.q & 1)) / 2)
    }
}

impl std::ops::Add for Axial {
    type Output = Axial;
    fn add(self, o: Axial) -> Axial {
        Axial { q: self.q + o.q, r: self.r + o.r }
    }
}

/// Whether the map wraps around.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wrap {
    /// A bordered rectangle.
    #[default]
    None,
    /// A cylinder: the east and west edges are adjacent.
    X,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    width: u32,
    height: u32,
    wrap: Wrap,
}

impl Topology {
    /// # Panics
    /// If either dimension is zero, or if a wrapping map has an odd width
    /// (odd-q layout only tiles seamlessly across an even number of columns).
    pub fn new(width: u32, height: u32, wrap: Wrap) -> Self {
        assert!(width > 0 && height > 0, "map dimensions must be non-zero");
        assert!(wrap == Wrap::None || width.is_multiple_of(2), "a wrapping map needs an even width");
        assert!((width as u64) * (height as u64) <= u32::MAX as u64, "map too large");
        Topology { width, height, wrap }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn wrap(&self) -> Wrap {
        self.wrap
    }

    pub fn len(&self) -> usize {
        (self.width * self.height) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ids(&self) -> impl Iterator<Item = HexId> + use<> {
        (0..self.width * self.height).map(HexId)
    }

    /// The hex at offset coordinates, applying wrap. `None` if off the map.
    pub fn at(&self, col: i32, row: i32) -> Option<HexId> {
        if row < 0 || row >= self.height as i32 {
            return None;
        }
        let col = match self.wrap {
            Wrap::X => col.rem_euclid(self.width as i32),
            Wrap::None if col < 0 || col >= self.width as i32 => return None,
            Wrap::None => col,
        };
        Some(HexId(row as u32 * self.width + col as u32))
    }

    /// Offset coordinates `(col, row)` of a hex.
    pub fn offset(&self, id: HexId) -> (i32, i32) {
        ((id.0 % self.width) as i32, (id.0 / self.width) as i32)
    }

    pub fn axial(&self, id: HexId) -> Axial {
        let (col, row) = self.offset(id);
        Axial::from_offset(col, row)
    }

    pub fn at_axial(&self, a: Axial) -> Option<HexId> {
        let (col, row) = a.to_offset();
        self.at(col, row)
    }

    /// The up-to-six neighbours of `id` (fewer at the map's edges).
    pub fn neighbors(&self, id: HexId) -> impl Iterator<Item = HexId> + use<> {
        let a = self.axial(id);
        let topo = *self;
        Axial::DIRECTIONS.into_iter().filter_map(move |d| topo.at_axial(a + d))
    }

    /// Steps between two hexes, taking the short way round on wrapping maps.
    pub fn distance(&self, a: HexId, b: HexId) -> u32 {
        let (a, b) = (self.axial(a), self.axial(b));
        match self.wrap {
            Wrap::None => a.distance(b),
            Wrap::X => {
                // Shifting by an even number of columns moves q by `width`
                // and r by `-width / 2`.
                let w = self.width as i32;
                [-1, 0, 1]
                    .into_iter()
                    .map(|k| a.distance(Axial::new(b.q + k * w, b.r - k * w / 2)))
                    .min()
                    .unwrap_or(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_axial_round_trip() {
        for col in -5..5 {
            for row in -5..5 {
                assert_eq!(Axial::from_offset(col, row).to_offset(), (col, row));
            }
        }
    }

    #[test]
    fn interior_hex_has_six_mutual_neighbours() {
        let t = Topology::new(10, 10, Wrap::None);
        for id in [t.at(4, 4).unwrap(), t.at(5, 4).unwrap()] {
            let ns: Vec<_> = t.neighbors(id).collect();
            assert_eq!(ns.len(), 6);
            for n in ns {
                assert_eq!(t.distance(id, n), 1);
                assert!(t.neighbors(n).any(|m| m == id), "adjacency is symmetric");
            }
        }
    }

    #[test]
    fn bounded_edges_have_fewer_neighbours() {
        let t = Topology::new(10, 10, Wrap::None);
        assert_eq!(t.neighbors(t.at(0, 0).unwrap()).count(), 2);
        assert_eq!(t.neighbors(t.at(0, 5).unwrap()).count(), 4);
        assert_eq!(t.distance(t.at(0, 5).unwrap(), t.at(9, 5).unwrap()), 9);
    }

    #[test]
    fn wrapping_map_joins_east_and_west() {
        let t = Topology::new(10, 10, Wrap::X);
        let west = t.at(0, 5).unwrap();
        let east = t.at(9, 5).unwrap();
        assert_eq!(t.neighbors(west).count(), 6);
        assert!(t.neighbors(west).any(|n| n == east));
        assert!(t.neighbors(east).any(|n| n == west));
        assert_eq!(t.distance(west, east), 1);
        assert_eq!(t.at(-1, 5), Some(east));
    }
}
