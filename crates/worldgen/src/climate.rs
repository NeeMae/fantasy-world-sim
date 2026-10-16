//! Rainfall from prevailing winds, after Dwarf Fortress.
//!
//! Winds blow along lines of latitude: trade winds westward near the
//! equator, westerlies eastward at mid-latitudes, polar easterlies westward
//! near the poles. Each ring of hexes round a line of latitude is swept
//! downwind:
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
//! world gets the same climate at any scale. The air circles the globe
//! until it settles.

use rayon::prelude::*;
use sim_core::Topology;

use crate::sphere::{Shape, smooth};

/// Rain-out rate over flat land, per noise-space unit, at latitude factor 1.
const RAIN: f64 = 1.2;
/// Rain-out per unit of height climbed.
const OROGRAPHIC: f64 = 2.5;
/// Moisture uptake rate over sea, per noise-space unit.
const EVAPORATION: f64 = 4.0;
/// Share of rain that plants and soil give back to the air.
const RECYCLING: f64 = 0.3;
/// Distance (noise-space units) weather wanders off its track.
const BLUR: f64 = 0.03;
/// Distance (noise-space units) over which the air "feels" the ground
/// height. Lift is measured against this running average rather than the
/// previous hex, which also hides how a ring of hexes zigzags in latitude.
const TERRAIN_MEMORY: f64 = 0.03;
/// Humidity of air before its first lap round a ring, as a share of what it
/// could hold; the second lap starts from where the first left off.
const INFLOW: f64 = 0.5;

/// Per-hex inputs to the climate model.
pub struct ClimateInput<'a> {
    pub topology: &'a Topology,
    pub shape: &'a Shape,
    /// Rings of latitude (see [`crate::sphere::rings`]).
    pub rings: &'a [(Vec<usize>, f64)],
    pub sea: &'a [bool],
    /// Land height above sea level, roughly 0..1.5 (0 at sea).
    pub height: &'a [f32],
    /// Latitude in degrees from the equator (0..90).
    pub latitude: &'a [f32],
    /// Temperature, 0 (frozen) to 1 (tropical): warm air holds more water.
    pub temperature: &'a [f32],
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
    let rings: Vec<Vec<f32>> = input
        .rings
        .par_iter()
        .map(|(ids, step)| {
            let east = sweep(input, ids, *step, true);
            let west = sweep(input, ids, *step, false);
            ids.iter()
                .enumerate()
                .map(|(c, &i)| {
                    let share = westerly_share(input.latitude[i] as f64);
                    (east[c] as f64 * share + west[c] as f64 * (1.0 - share)) as f32
                })
                .collect()
        })
        .collect();
    let mut rain = vec![0.0f32; input.topology.len()];
    for ((ids, _), values) in input.rings.iter().zip(rings) {
        for (&i, v) in ids.iter().zip(values) {
            rain[i] = v;
        }
    }
    // Weather wanders off its track.
    smooth(input.topology, input.shape, &rain, BLUR)
}

/// Carries air round one ring of hexes, given in west-to-east order, blowing
/// eastward if `eastward` (else westward), `dx` apart. Returns rain per hex.
fn sweep(input: &ClimateInput, ids: &[usize], dx: f64, eastward: bool) -> Vec<f32> {
    let n = ids.len();
    let order: Vec<usize> = if eastward { (0..n).collect() } else { (0..n).rev().collect() };
    let capacity = |i: usize| 0.25 + 0.75 * input.temperature[i].clamp(0.0, 1.0) as f64;

    let mut rain = vec![0.0f32; n];
    let first = ids[order[0]];
    let mut humidity = INFLOW * capacity(first);
    // Running average of the ground the air has crossed.
    let mut ground = input.height[ids[order[n - 1]]] as f64;
    let follow = 1.0 - (-dx / TERRAIN_MEMORY).exp();
    // Each ring is circled twice so the air arriving at the start has
    // itself come round the world.
    let laps = 2;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphere::rings;
    use sim_core::HexId;

    /// A planet of sea with one continent, 60° of longitude wide, with a
    /// north-south ridge down its middle.
    fn planet(latitude_of: impl Fn(f64) -> f64) -> (Topology, Vec<f32>) {
        let topo = Topology::new(40);
        let shape = Shape::new(&topo, 1.0);
        let lon = |id: HexId| topo.lat_lon(id).1.to_degrees();
        let sea: Vec<bool> = topo.ids().map(|id| lon(id).abs() > 30.0).collect();
        let height: Vec<f32> = topo
            .ids()
            .map(|id| {
                if lon(id).abs() > 30.0 {
                    0.0
                } else if lon(id).abs() < 3.0 {
                    1.2
                } else {
                    0.1
                }
            })
            .collect();
        let latitude: Vec<f32> =
            topo.ids().map(|id| latitude_of(topo.lat_lon(id).0.to_degrees()) as f32).collect();
        let temperature = vec![0.6; topo.len()];
        let rings = rings(&topo, &shape);
        let rain = rainfall(&ClimateInput {
            topology: &topo,
            shape: &shape,
            rings: &rings,
            sea: &sea,
            height: &height,
            latitude: &latitude,
            temperature: &temperature,
        });
        (topo, rain)
    }

    fn mean(topo: &Topology, rain: &[f32], lons: std::ops::Range<f64>) -> f32 {
        let cells: Vec<usize> = topo
            .ids()
            .filter(|&id| {
                let (lat, lon) = topo.lat_lon(id);
                lat.to_degrees().abs() < 20.0 && lons.contains(&lon.to_degrees())
            })
            .map(|id| id.index())
            .collect();
        cells.iter().map(|&i| rain[i]).sum::<f32>() / cells.len() as f32
    }

    #[test]
    fn westerlies_leave_a_rain_shadow_east_of_a_ridge() {
        // Everywhere in the westerlies.
        let (topo, rain) = planet(|_| 45.0);
        let windward = mean(&topo, &rain, -12.0..-3.0);
        let lee = mean(&topo, &rain, 5.0..25.0);
        assert!(windward > 2.0 * lee, "windward {windward} vs lee {lee}");
    }

    #[test]
    fn trade_winds_blow_the_other_way() {
        let (topo, rain) = planet(|_| 10.0);
        let east_side = mean(&topo, &rain, 3.0..12.0);
        let west_side = mean(&topo, &rain, -25.0..-5.0);
        assert!(east_side > 1.5 * west_side, "the wet side faces east: {east_side} vs {west_side}");
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
