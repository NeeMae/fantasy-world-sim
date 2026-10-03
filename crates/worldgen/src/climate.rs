//! Rainfall from prevailing winds, after Dwarf Fortress.
//!
//! Winds blow along lines of latitude: trade winds westward near the
//! equator, westerlies eastward at mid-latitudes, polar easterlies westward
//! near the poles. Each row of hexes is swept downwind:
//!
//! * over sea, the air takes up moisture, more when warm;
//! * over land it rains out steadily, faster where latitude favours rain
//!   (the rising air of the equator and the mid-latitude storm belts) and
//!   slower where it doesn't (the sinking air of the subtropics and poles);
//! * climbing high ground wrings the air out (orographic rain), so the
//!   windward side of a range is wet and the lee lies in a rain shadow;
//! * plants hand some of the rain back to the air, so moisture reaches
//!   further inland than it otherwise would.
//!
//! Rates are per unit of distance in noise space, not per hex, so the same
//! world gets the same climate at any scale. Wrapping worlds circulate the
//! air round the globe until it settles.

use rayon::prelude::*;
use sim_core::{Topology, Wrap};

/// Rain-out rate over flat land, per noise-space unit, at latitude factor 1.
const RAIN: f64 = 1.2;
/// Rain-out per unit of height climbed.
const OROGRAPHIC: f64 = 2.5;
/// Moisture uptake rate over sea, per noise-space unit.
const EVAPORATION: f64 = 4.0;
/// Share of rain that plants and soil give back to the air.
const RECYCLING: f64 = 0.3;
/// Humidity of air arriving over an open map's upwind edge, as a share of
/// what it could hold (the world beyond is assumed middling).
const INFLOW: f64 = 0.5;
/// Distance (noise-space units) weather wanders off its track.
const BLUR: f64 = 0.03;
/// Distance (noise-space units) over which the air "feels" the ground
/// height. Lift is measured against this running average rather than the
/// previous hex, which also hides the half-hex zigzag of a hex row.
const TERRAIN_MEMORY: f64 = 0.03;

/// Per-hex inputs to the climate model.
pub struct ClimateInput<'a> {
    pub topology: &'a Topology,
    pub sea: &'a [bool],
    /// Land height above sea level, roughly 0..1.5 (0 at sea).
    pub height: &'a [f32],
    /// Latitude in degrees from the equator (0..90).
    pub latitude: &'a [f32],
    /// Temperature, 0 (frozen) to 1 (tropical): warm air holds more water.
    pub temperature: &'a [f32],
    /// Distance between neighbouring columns, in noise-space units.
    pub column_step: f64,
    /// Distance between neighbouring rows, in noise-space units.
    pub row_step: f64,
}

/// How favourable a latitude is to rain: wet at the equator and in the
/// mid-latitude storm belts, dry in the subtropics (~25°) and at the poles.
pub fn rain_belt(latitude: f64) -> f64 {
    0.35 + 1.25 * (-(latitude / 12.0).powi(2)).exp() + 0.65 * (-((latitude - 52.0) / 14.0).powi(2)).exp()
}

/// Share of the wind at a latitude blowing eastward (the westerlies); the
/// rest blows westward (trade winds and polar easterlies).
pub fn westerly_share(latitude: f64) -> f64 {
    let smooth = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    smooth(25.0, 35.0, latitude) * (1.0 - smooth(55.0, 65.0, latitude))
}

/// Rainfall intensity for every hex (arbitrary units; compare, don't read).
pub fn rainfall(input: &ClimateInput) -> Vec<f32> {
    let topo = input.topology;
    let (w, h) = (topo.width() as usize, topo.height() as usize);
    // Odd columns sit half a row lower than even ones, so a row of hexes
    // zigzags up and down. Height seen along a row is taken at the even
    // columns' level: odd columns average with the hex above.
    let aligned: Vec<f32> = topo
        .ids()
        .map(|id| {
            let (col, row) = topo.offset(id);
            let here = input.height[id.index()];
            if col & 1 == 0 {
                return here;
            }
            match (topo.at(col, row - 1), topo.at(col, row + 1)) {
                (Some(above), _) => (here + input.height[above.index()]) / 2.0,
                // Top row: extrapolate half a row up from the hex below.
                (None, Some(below)) => 1.5 * here - 0.5 * input.height[below.index()],
                (None, None) => here,
            }
        })
        .collect();
    let input = &ClimateInput { height: &aligned, ..*input };
    let rows: Vec<Vec<f32>> = (0..h)
        .into_par_iter()
        .map(|row| {
            let ids: Vec<usize> =
                (0..w).map(|col| topo.at(col as i32, row as i32).expect("on map").index()).collect();
            let east = sweep(input, &ids, true);
            let west = sweep(input, &ids, false);
            ids.iter()
                .enumerate()
                .map(|(c, &i)| {
                    let share = westerly_share(input.latitude[i] as f64);
                    (east[c] as f64 * share + west[c] as f64 * (1.0 - share)) as f32
                })
                .collect()
        })
        .collect();

    let mut rain = vec![0.0f32; topo.len()];
    for (row, values) in rows.into_iter().enumerate() {
        for (col, v) in values.into_iter().enumerate() {
            rain[topo.at(col as i32, row as i32).expect("on map").index()] = v;
        }
    }
    // Weather wanders off its track: blur evenly in both directions over a
    // fixed distance, so it's the same at any scale.
    let rows = ((BLUR / input.row_step).round() as i32).max(1);
    let cols = ((BLUR / input.column_step).round() as i32).max(1);
    for _ in 0..2 {
        rain = blur(topo, &rain, cols, 0);
        rain = blur(topo, &rain, 0, rows);
    }
    rain
}

/// Carries air along one row of hexes, given in west-to-east order, blowing
/// eastward if `eastward` (else westward). Returns rain per hex.
fn sweep(input: &ClimateInput, ids: &[usize], eastward: bool) -> Vec<f32> {
    let n = ids.len();
    let dx = input.column_step;
    let order: Vec<usize> = if eastward { (0..n).collect() } else { (0..n).rev().collect() };
    let wraps = input.topology.wrap() == Wrap::X;
    let capacity = |i: usize| 0.25 + 0.75 * input.temperature[i].clamp(0.0, 1.0) as f64;

    let mut rain = vec![0.0f32; n];
    let first = ids[order[0]];
    let mut humidity = INFLOW * capacity(first);
    // Running average of the ground the air has crossed.
    let mut ground = if wraps { input.height[ids[order[n - 1]]] as f64 } else { input.height[first] as f64 };
    let follow = 1.0 - (-dx / TERRAIN_MEMORY).exp();
    // A wrapping row is circled twice so the air arriving at the start has
    // itself come round the world.
    let laps = if wraps { 2 } else { 1 };
    for lap in 0..laps {
        for &c in &order {
            let i = ids[c];
            let belt = rain_belt(input.latitude[i] as f64);
            let previous = ground;
            ground += (input.height[i] as f64 - ground) * follow;
            let intensity = if input.sea[i] {
                humidity += (capacity(i) - humidity) * (1.0 - (-EVAPORATION * dx).exp());
                humidity * RAIN * belt
            } else {
                let lift = (ground - previous).max(0.0);
                let fallen = humidity * (1.0 - (-(RAIN * belt * dx + OROGRAPHIC * lift)).exp());
                humidity += fallen * RECYCLING - fallen;
                fallen / dx
            };
            if lap == laps - 1 {
                rain[c] = intensity as f32;
            }
        }
    }
    rain
}

/// Averages each hex with the hexes up to `dc` columns east and west, or
/// `dr` rows north and south (wrapping east-west where the map does).
fn blur(topo: &Topology, values: &[f32], dc: i32, dr: i32) -> Vec<f32> {
    (0..topo.len() as u32)
        .into_par_iter()
        .map(|i| {
            let (col, row) = topo.offset(sim_core::HexId(i));
            let (mut sum, mut count) = (0.0, 0.0);
            for k in -(dc.max(dr))..=dc.max(dr) {
                let (c, r) = if dc > 0 { (col + k, row) } else { (col, row + k) };
                if let Some(h) = topo.at(c, r) {
                    sum += values[h.index()];
                    count += 1.0;
                }
            }
            sum / count
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip of hexes: sea in the west, then land with a ridge.
    fn strip(latitude: f32) -> (Topology, Vec<bool>, Vec<f32>, Vec<f32>, Vec<f32>) {
        let topo = Topology::new(60, 4, Wrap::None);
        let mut sea = vec![false; topo.len()];
        let mut height = vec![0.1; topo.len()];
        for id in topo.ids() {
            let (col, _) = topo.offset(id);
            if col < 15 {
                sea[id.index()] = true;
                height[id.index()] = 0.0;
            } else if (30..33).contains(&col) {
                height[id.index()] = 1.2; // the ridge
            }
        }
        let n = topo.len();
        (topo, sea, height, vec![latitude; n], vec![0.6; n])
    }

    fn mean(topo: &Topology, rain: &[f32], cols: std::ops::Range<i32>) -> f32 {
        let ids: Vec<_> = cols.flat_map(|c| (0..4).map(move |r| (c, r))).collect();
        ids.iter().map(|&(c, r)| rain[topo.at(c, r).unwrap().index()]).sum::<f32>() / ids.len() as f32
    }

    #[test]
    fn westerlies_leave_a_rain_shadow_east_of_a_ridge() {
        let (topo, sea, height, latitude, temperature) = strip(45.0);
        let input = ClimateInput {
            topology: &topo,
            sea: &sea,
            height: &height,
            latitude: &latitude,
            temperature: &temperature,
            column_step: 0.02,
            row_step: 0.02,
        };
        let rain = rainfall(&input);
        let windward = mean(&topo, &rain, 26..31);
        let lee = mean(&topo, &rain, 34..44);
        let coast = mean(&topo, &rain, 15..20);
        assert!(windward > 2.0 * lee, "windward {windward} vs lee {lee}");
        assert!(coast > lee, "the coast is wetter than the rain shadow");
    }

    #[test]
    fn same_climate_at_any_scale() {
        // Halving the hex size (and so the step) leaves rainfall roughly the
        // same at the same place.
        let (topo, sea, height, latitude, temperature) = strip(45.0);
        let coarse = rainfall(&ClimateInput {
            topology: &topo,
            sea: &sea,
            height: &height,
            latitude: &latitude,
            temperature: &temperature,
            column_step: 0.02,
            row_step: 0.02,
        });
        let fine_topo = Topology::new(120, 4, Wrap::None);
        let pick = |v: &Vec<f32>| -> Vec<f32> {
            fine_topo
                .ids()
                .map(|id| {
                    let (c, r) = fine_topo.offset(id);
                    v[topo.at(c / 2, r).unwrap().index()]
                })
                .collect()
        };
        let fine = rainfall(&ClimateInput {
            topology: &fine_topo,
            sea: &fine_topo
                .ids()
                .map(|id| {
                    let (c, r) = fine_topo.offset(id);
                    sea[topo.at(c / 2, r).unwrap().index()]
                })
                .collect::<Vec<_>>(),
            height: &pick(&height),
            latitude: &pick(&latitude),
            temperature: &pick(&temperature),
            column_step: 0.01,
            row_step: 0.02,
        });
        for col in [18, 25, 40, 55] {
            let a = coarse[topo.at(col, 1).unwrap().index()];
            let b = fine[fine_topo.at(col * 2, 1).unwrap().index()];
            assert!((a - b).abs() / a.max(1e-6) < 0.2, "col {col}: coarse {a} fine {b}");
        }
    }

    #[test]
    fn hex_row_zigzag_does_not_stripe_the_rain() {
        // Odd columns sit half a row lower, so on ground sloping north to
        // south the height alternates along a row. Rain must not.
        let topo = Topology::new(80, 4, Wrap::None);
        let n = topo.len();
        let sea: Vec<bool> = topo.ids().map(|id| topo.offset(id).0 < 10).collect();
        let height: Vec<f32> = topo
            .ids()
            .map(|id| {
                let (c, _) = topo.offset(id);
                let (_, r) = topo.offset(id);
                let y = r as f32 + if c % 2 == 1 { 0.5 } else { 0.0 };
                if c < 10 { 0.0 } else { 0.3 + 0.1 * y }
            })
            .collect();
        let latitude = vec![45.0; n];
        let temperature = vec![0.6; n];
        let rain = rainfall(&ClimateInput {
            topology: &topo,
            sea: &sea,
            height: &height,
            latitude: &latitude,
            temperature: &temperature,
            column_step: 0.01,
            row_step: 0.01,
        });
        for c in 30..70 {
            let (a, b) = (rain[topo.at(c, 1).unwrap().index()], rain[topo.at(c + 1, 1).unwrap().index()]);
            assert!((a - b).abs() / a.max(b) < 0.15, "columns {c} and {} differ: {a} vs {b}", c + 1);
        }
    }

    #[test]
    fn latitude_bands() {
        assert!(rain_belt(0.0) > rain_belt(25.0) * 3.0, "wet equator, dry subtropics");
        assert!(rain_belt(52.0) > rain_belt(25.0) * 2.0, "wet storm belt");
        assert_eq!(westerly_share(10.0), 0.0);
        assert_eq!(westerly_share(45.0), 1.0);
        assert_eq!(westerly_share(75.0), 0.0);
    }
}
