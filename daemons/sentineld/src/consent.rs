//! Ask before placing a sighting (`privacy-sentinel-plan.md` §6).
//!
//! The tracker holds two capabilities: `bt.scan.passive` to see a tag, and a
//! coarse `location.read` to say WHERE it saw one. The second is the one a person
//! would want back, and until this existed it was taken rather than granted: the
//! coarse feed asked GeoClue2 directly, so the sentinel appeared as no location
//! principal anywhere and there was nothing to revoke.
//!
//! **Denial is not an error, and it is not the detector going off.** §6 says
//! revoking location degrades the tracker to detector-d-only - it can still see
//! that a tag is nearby and can no longer confirm one is following you - fail-soft,
//! and it says so. That is what [`LocationConsent::Denied`] means here: the watch
//! runs, adverts are classified, and nothing is placed, which the criteria already
//! handle because a sighting with no fix is not recorded.
//!
//! **A broker that cannot be reached is a denial**, not a pass. A consent path
//! whose failure mode is "carry on" is not a consent path.
//!
//! The recipient is this daemon. Unlike modulesd asking on behalf of a module, the
//! sentinel is the principal that reads the location, so there is nobody to speak
//! for and `on_behalf_of` stays empty - the grant lands under the identity the
//! browser will show.

use std::path::{Path, PathBuf};

use arlen_consent_contract::{ConsentClass, ConsentOutcome, IntakeResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// The largest reply this will hold, matching the broker's own bound.
const MAX_FRAME: usize = 64 * 1024;

/// What the broker said about the coarse location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationConsent {
    /// Sightings may be placed.
    Granted,
    /// They may not. The watch still runs and sees tags; it cannot say where.
    Denied,
}

/// The broker's intake socket, mirroring its bind.
pub fn intake_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("consent-intake.sock")
}

/// The request body for the coarse location grant.
///
/// Separate from the sending so the words a person reads are testable without a
/// broker. The summary is the whole of what they have to judge, so it says what is
/// read, how coarse it is, and what it is for - not "grant location access".
pub fn location_request() -> arlen_consent_broker::RequestBody {
    arlen_consent_broker::RequestBody {
        class: ConsentClass::CapabilityGrant,
        kind: arlen_consent_broker::ActionKind::Ordinary,
        // A detector somebody switched on, never something a document asked for.
        triggered_by_external_content: false,
        summary: "The privacy sentinel wants your approximate location, so it can \
                  tell whether a tracking tag is following you between places."
            .to_string(),
        scope: Some("approximate location (city level), while the tracker watch runs".to_string()),
        recipient: None,
        preview: None,
        targets: Vec::new(),
        total: None,
        on_behalf_of: None,
    }
}

/// Ask the broker whether sightings may be placed.
///
/// Every failure answers [`LocationConsent::Denied`]: an unreachable broker, a
/// malformed reply, a refusal. The tracker degrades and says so rather than
/// quietly reading a location nobody agreed to.
pub async fn ask_for_location(socket: &Path) -> LocationConsent {
    match request(socket, &location_request()).await {
        Ok(IntakeResult::SilentGranted)
        | Ok(IntakeResult::Decided {
            outcome: ConsentOutcome::AllowedOnce | ConsentOutcome::AllowedRemembered,
        }) => LocationConsent::Granted,
        Ok(_) => LocationConsent::Denied,
        Err(why) => {
            tracing::info!("no location consent ({why}), so tags are seen but not placed");
            LocationConsent::Denied
        }
    }
}

/// One intake round trip, framed the way the broker frames.
async fn request(
    socket: &Path,
    body: &arlen_consent_broker::RequestBody,
) -> Result<IntakeResult, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("consent broker unreachable: {e}"))?;
    let payload = serde_json::to_vec(body).map_err(|e| format!("encoding request: {e}"))?;
    let len = u32::try_from(payload.len()).map_err(|_| "request too large".to_string())?;
    stream.write_all(&len.to_le_bytes()).await.map_err(|e| format!("writing request: {e}"))?;
    stream.write_all(&payload).await.map_err(|e| format!("writing request: {e}"))?;

    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.map_err(|e| format!("reading reply: {e}"))?;
    // Checked before allocating, so a corrupt length cannot make us reserve it.
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(format!("reply frame {len} exceeds {MAX_FRAME}"));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.map_err(|e| format!("reading reply: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("decoding reply: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentinel asks for itself, so the grant lands under the identity the
    /// browser shows rather than under somebody it spoke for.
    #[test]
    fn the_request_is_made_in_the_sentinels_own_name() {
        let body = location_request();
        assert!(body.on_behalf_of.is_none());
        assert_eq!(body.class, ConsentClass::CapabilityGrant);
        assert!(!body.triggered_by_external_content);
    }

    /// The summary is the whole of what a person judges, so it has to say what is
    /// read, how coarse it is, and what for.
    #[test]
    fn the_summary_says_what_it_is_for_and_how_coarse() {
        let body = location_request();
        let summary = body.summary.to_lowercase();
        assert!(summary.contains("approximate"), "{summary}");
        assert!(summary.contains("following you"), "{summary}");
        let scope = body.scope.unwrap_or_default().to_lowercase();
        assert!(scope.contains("city level"), "{scope}");
    }

    /// A broker that is not there is a denial. A consent path whose failure mode
    /// is "carry on" is not one.
    #[tokio::test]
    async fn an_unreachable_broker_denies() {
        let missing = std::path::Path::new("/nonexistent/arlen/consent-intake.sock");
        assert_eq!(ask_for_location(missing).await, LocationConsent::Denied);
    }
}
