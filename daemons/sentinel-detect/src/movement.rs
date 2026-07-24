//! SEN-4 movement geometry: turn a device's location fixes into the distinct-place
//! count and travelled distance the alert criteria consume.
//!
//! The daemon's epoch/movement model records WHERE (a coarse GeoClue fix) each
//! separated sighting fell; this module reduces those fixes to the two geometric
//! inputs [`crate::trigger::TrackerObservation`] needs: how many DISTINCT places the
//! tag was seen at (the load-bearing "travelling with the user" signal) and how far
//! it travelled. Pure great-circle geometry - no GeoClue, no clock. The stateful
//! epoch boundaries (suspend/resume, BSSID-set change, the home-anchor) stay in the
//! daemon; this is the geometry it calls.

/// Two fixes at least this far apart count as distinct places (Apple's own
/// TrackingAvoidance uses a comparable radius); it is also the epoch-boundary
/// distance the movement model uses.
pub const DISTINCT_LOCATION_METRES: f64 = 400.0;

/// Mean Earth radius in metres (WGS-84 mean), for the great-circle distance.
const EARTH_RADIUS_METRES: f64 = 6_371_008.8;

/// A coarse location fix (decimal degrees), as GeoClue reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    /// Latitude in decimal degrees.
    pub lat: f64,
    /// Longitude in decimal degrees.
    pub lon: f64,
}

/// Great-circle distance between two fixes in metres (haversine). Returns 0 for the
/// same point; the result is always finite for finite inputs (the central angle is
/// clamped to `[-1, 1]` before `asin`, so float error near antipodes cannot produce
/// a NaN).
pub fn haversine_metres(a: Fix, b: Fix) -> f64 {
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    // asin form; clamp guards float error pushing h just over 1.0.
    2.0 * EARTH_RADIUS_METRES * h.sqrt().clamp(0.0, 1.0).asin()
}

/// Whether two fixes are far enough apart to count as distinct places (>= the
/// distinct-location threshold).
pub fn is_distinct_place(a: Fix, b: Fix) -> bool {
    haversine_metres(a, b) >= DISTINCT_LOCATION_METRES
}

/// Count the DISTINCT places among a device's sighting fixes: greedily cluster fixes
/// within [`DISTINCT_LOCATION_METRES`] of an existing cluster's first fix into that
/// cluster, so a tag lingering in one spot (many fixes, one place) counts once, while
/// one seen across town counts as several. Order-stable, so the same fixes always
/// yield the same count. An empty list is 0 places.
pub fn count_distinct_locations(fixes: &[Fix]) -> u32 {
    let mut anchors: Vec<Fix> = Vec::new();
    for &fix in fixes {
        if !anchors.iter().any(|&a| !is_distinct_place(a, fix)) {
            anchors.push(fix);
        }
    }
    anchors.len() as u32
}

/// The total distance travelled along a device's fixes in order (the sum of
/// consecutive great-circle hops). 0 for fewer than two fixes. This is the
/// `travelled_metres` the alert criteria compare against, an over-estimate-free
/// path length (it never counts a hop the tag did not make).
pub fn travelled_metres(fixes: &[Fix]) -> f64 {
    fixes
        .windows(2)
        .map(|w| haversine_metres(w[0], w[1]))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(lat: f64, lon: f64) -> Fix {
        Fix { lat, lon }
    }

    #[test]
    fn haversine_is_zero_for_the_same_point_and_finite_far_apart() {
        let p = fix(47.2692, 11.4041); // Innsbruck
        assert_eq!(haversine_metres(p, p), 0.0);
        // Innsbruck -> Vienna is ~380 km; sanity-bound it.
        let vienna = fix(48.2082, 16.3738);
        let d = haversine_metres(p, vienna);
        assert!(d > 350_000.0 && d < 410_000.0, "expected ~380 km, got {d}");
        assert!(haversine_metres(fix(89.0, 0.0), fix(-89.0, 180.0)).is_finite());
    }

    #[test]
    fn distinct_place_uses_the_400m_threshold() {
        let a = fix(47.2692, 11.4041);
        // ~50 m east: not distinct.
        let near = fix(47.2692, 11.4048);
        assert!(!is_distinct_place(a, near), "{}", haversine_metres(a, near));
        // ~1 km away: distinct.
        let far = fix(47.2782, 11.4041);
        assert!(is_distinct_place(a, far));
    }

    #[test]
    fn one_lingering_spot_counts_once_two_places_count_two() {
        let a = fix(47.2692, 11.4041);
        let a2 = fix(47.2693, 11.4042); // a few metres from a
        let b = fix(47.2782, 11.4200); // ~1.5 km away
        assert_eq!(count_distinct_locations(&[a, a2, a]), 1);
        assert_eq!(count_distinct_locations(&[a, a2, b, a]), 2);
        assert_eq!(count_distinct_locations(&[]), 0);
    }

    #[test]
    fn travelled_sums_consecutive_hops() {
        let a = fix(47.2692, 11.4041);
        let b = fix(47.2782, 11.4041);
        let hop = haversine_metres(a, b);
        // a -> b -> a is two hops of the same length.
        assert!((travelled_metres(&[a, b, a]) - 2.0 * hop).abs() < 1e-6);
        assert_eq!(travelled_metres(&[a]), 0.0);
        assert_eq!(travelled_metres(&[]), 0.0);
    }
}
