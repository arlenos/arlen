//! The coarse location feed (`tracker-sentinel-plan.md` §2.3, §6 job 1).
//!
//! SEN-4's load-bearing signal is "this tag was at two DIFFERENT places", so a
//! sighting needs a fix on it. This is where that fix comes from, and the whole
//! module is about asking for as little as will answer the question.
//!
//! **Coarse on purpose, and the constant is measured rather than guessed.** The plan
//! left the exact `GCLUE_ACCURACY_LEVEL_*` value to be confirmed against installed
//! headers because the published docs were unreachable during research. Read off
//! `/usr/include/libgeoclue-2.0/gclue-enums.h`: the enum is NOT contiguous - `NONE`
//! 0, `COUNTRY` 1, `CITY` 4, `NEIGHBORHOOD` 5, `STREET` 6, `EXACT` 8 - so a version
//! that counted up from zero would have asked for street precision while believing
//! it had asked for city. We request [`ACCURACY_CITY`]. The sentinel needs "am I
//! somewhere else", not metres, and the threshold that matters is 400 m.
//!
//! **The detector switch is the gate.** A fix is only ever requested while the
//! tracker detector is on. Turning it off stops the request, and this module holds
//! no fix across that: a location the person switched off the collection of is not
//! one to keep in memory for later.
//!
//! **Every failure is soft.** No GeoClue on the bus, a refused client, a location
//! object that will not read: all of it answers `None`. A sentinel that cannot place
//! a sighting loses the place criterion for that sighting, which weakens detection;
//! a sentinel that refuses to start because a location service is absent protects
//! nobody at all.

use arlen_sentinel_detect::movement::Fix;

/// City-level accuracy: `GCLUE_ACCURACY_LEVEL_CITY`, measured off
/// `gclue-enums.h` on 13 September 2026.
pub const ACCURACY_CITY: u32 = 4;

/// What GeoClue is told this client is. It appears in the location agent's prompt,
/// so it is the sentinel's own id rather than a borrowed one.
pub const DESKTOP_ID: &str = "arlen-sentineld";

/// The bus name, fixed by GeoClue. The manager path is fixed too and is the
/// proxy's own default; only this one is needed again, to build the per-client and
/// per-location proxies at the paths the manager hands back.
const GEOCLUE_BUS: &str = "org.freedesktop.GeoClue2";

/// Why no fix was produced. Every variant is soft: the caller records a sighting
/// without a place rather than dropping it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoFix {
    /// The tracker detector is switched off, so nothing was asked.
    DetectorOff,
    /// GeoClue is not on the bus, or refused a client.
    Unavailable,
    /// A client exists but has no location yet (the usual answer for the first
    /// seconds after start, and the permanent one where no Wi-Fi geolocation
    /// backend can place the machine).
    NoLocationYet,
}

#[zbus::proxy(
    interface = "org.freedesktop.GeoClue2.Manager",
    default_service = "org.freedesktop.GeoClue2",
    default_path = "/org/freedesktop/GeoClue2/Manager"
)]
trait Manager {
    fn get_client(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[zbus::proxy(interface = "org.freedesktop.GeoClue2.Client", default_service = "org.freedesktop.GeoClue2")]
trait Client {
    fn start(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn set_desktop_id(&self, id: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn set_requested_accuracy_level(&self, level: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn location(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[zbus::proxy(interface = "org.freedesktop.GeoClue2.Location", default_service = "org.freedesktop.GeoClue2")]
trait Location {
    #[zbus(property)]
    fn latitude(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn longitude(&self) -> zbus::Result<f64>;
}

/// A started GeoClue client asking for city-level fixes.
pub struct LocationFeed {
    client: ClientProxy<'static>,
}

impl LocationFeed {
    /// Ask GeoClue for a client, tell it who is asking and how coarse an answer
    /// will do, and start it.
    ///
    /// The accuracy level is set BEFORE `Start`, which is not a style choice: the
    /// level is what the location agent shows the person when it asks, and a client
    /// that starts first has already asked for whatever the default was.
    pub async fn open(connection: &zbus::Connection) -> Result<Self, NoFix> {
        let manager = ManagerProxy::new(connection).await.map_err(|_| NoFix::Unavailable)?;
        let path = manager.get_client().await.map_err(|_| NoFix::Unavailable)?;
        let client = ClientProxy::builder(connection)
            .destination(GEOCLUE_BUS)
            .map_err(|_| NoFix::Unavailable)?
            .path(path)
            .map_err(|_| NoFix::Unavailable)?
            .build()
            .await
            .map_err(|_| NoFix::Unavailable)?;
        client.set_desktop_id(DESKTOP_ID).await.map_err(|_| NoFix::Unavailable)?;
        client
            .set_requested_accuracy_level(ACCURACY_CITY)
            .await
            .map_err(|_| NoFix::Unavailable)?;
        client.start().await.map_err(|_| NoFix::Unavailable)?;
        Ok(Self { client })
    }

    /// The current coarse fix, or why there is none.
    pub async fn fix(&self, connection: &zbus::Connection) -> Result<Fix, NoFix> {
        let path = self.client.location().await.map_err(|_| NoFix::Unavailable)?;
        // GeoClue answers "no fix yet" with the root path rather than an error.
        if path.as_str() == "/" {
            return Err(NoFix::NoLocationYet);
        }
        let location = LocationProxy::builder(connection)
            .destination(GEOCLUE_BUS)
            .map_err(|_| NoFix::Unavailable)?
            .path(path)
            .map_err(|_| NoFix::Unavailable)?
            .build()
            .await
            .map_err(|_| NoFix::Unavailable)?;
        let lat = location.latitude().await.map_err(|_| NoFix::NoLocationYet)?;
        let lon = location.longitude().await.map_err(|_| NoFix::NoLocationYet)?;
        if !lat.is_finite() || !lon.is_finite() {
            // A non-finite fix would poison every distance it is measured against,
            // and `should_alert` already refuses a non-finite distance. Refuse it
            // here too, where it can still be reported as an absent fix.
            return Err(NoFix::NoLocationYet);
        }
        Ok(Fix { lat, lon })
    }

    /// Stop asking. Best-effort: the client goes away with the connection anyway,
    /// and a stop that fails must not stop the caller.
    pub async fn close(self) {
        let _ = self.client.stop().await;
    }
}

/// Open a feed only while the tracker detector is on.
///
/// The switch is read at the call rather than captured once, so turning the
/// detector off stops the next request instead of the one after a restart.
pub async fn feed_if_enabled(
    connection: &zbus::Connection,
    tracker_on: bool,
) -> Result<LocationFeed, NoFix> {
    if !tracker_on {
        return Err(NoFix::DetectorOff);
    }
    LocationFeed::open(connection).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The constant the plan asked to have confirmed. `CITY` is 4, and the enum
    /// skips 2 and 3, so a fourth variant counted from zero would be street-level.
    #[test]
    fn the_accuracy_level_is_city_not_the_fourth_variant() {
        assert_eq!(ACCURACY_CITY, 4);
        // Street is 6 and exact is 8; neither is what a sentinel needs and neither
        // is one away from what we ask for.
        assert_ne!(ACCURACY_CITY, 6);
        assert_ne!(ACCURACY_CITY, 8);
    }

    /// A switched-off detector asks nothing at all - no bus call, no prompt.
    #[tokio::test]
    async fn a_switched_off_detector_never_reaches_the_bus() {
        // A connection that cannot be used: if the gate leaks, the call fails with
        // `Unavailable` instead, which is the assertion.
        let Ok(connection) = zbus::Connection::session().await else {
            // No session bus in this environment; the gate is still the thing under
            // test and it returns before touching the connection.
            return;
        };
        assert_eq!(feed_if_enabled(&connection, false).await.err(), Some(NoFix::DetectorOff));
    }
}
