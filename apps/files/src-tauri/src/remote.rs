//! FM-R15: the "Network / Remote" sidebar places. Fetches the configured
//! online-accounts / rclone mounts from the online-accounts daemon's capability-gated
//! `org.arlen.Accounts1.ListAccounts` and maps them to the places the PlacesSidebar
//! renders. The mapping (classification, display name, order) is the tested
//! `arlen_file_browser_core::remote` logic; this is only the D-Bus fetch.

use arlen_file_browser_core::remote::{remote_places_from_listings, RemotePlace};

/// What the caller may render, and the reason it is not a plain `Vec`.
///
/// An empty list used to mean two different things - the accounts subsystem is not on
/// this system, or it is and you have no accounts - and a sidebar cannot tell them
/// apart. The first reads to a person as "you have no accounts", which is a missing
/// subsystem wearing the costume of an answer. The online-accounts daemon is built,
/// CI-tested and installed by nothing, so on this image the first case is the ONLY
/// case, and the empty list would have been a lie every time.
///
/// Shipping the daemon is not the fix (three daemons and a subsystem, put in front of
/// people looking functional while incomplete); saying so is. A caller must match on
/// the state, so it cannot render an absence as a result by accident.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum NetworkPlaces {
    /// The subsystem did not answer: no accounts daemon on this system, or no bus.
    /// Render the section as unavailable, or not at all - never as an empty list.
    /// The bus answered and refused this app: the accounts exist, we may not see
    /// them. Distinct from `Unavailable` because the fix is a permission, not an
    /// installation.
    Denied {
        /// What the bus said, verbatim; for the log, not the screen.
        reason: String,
    },
    Unavailable {
        /// Why, for the log and for a tooltip. Not a user-facing sentence.
        reason: String,
    },
    /// The subsystem answered. `places` MAY be empty, and that means what it says:
    /// the user has configured none.
    Configured {
        /// The granted mounts, already capability-filtered by the daemon.
        places: Vec<RemotePlace>,
    },
}

/// The online-accounts / rclone mounts the files app is granted, as sidebar places.
#[tauri::command]
pub async fn network_places() -> NetworkPlaces {
    classify(fetch_account_listings().await)
}

/// Turn a fetch result into what the sidebar may render. Split from the D-Bus call so
/// the distinction that matters - absent subsystem versus no accounts - is a test
/// rather than a comment.
fn classify(
    fetched: Result<Vec<(String, String, String, String)>, zbus::Error>,
) -> NetworkPlaces {
    match fetched {
        Ok(listings) => NetworkPlaces::Configured {
            places: remote_places_from_listings(&listings),
        },
        // A refusal is not an absence, and D-Bus says which is which. `AccessDenied`
        // is the bus policy or the daemon turning this app away - the accounts are
        // there. Everything else (no owner, no bus, a transport fault) is the
        // subsystem not being reachable at all.
        Err(zbus::Error::MethodError(name, detail, _))
            if name.as_str().ends_with(".AccessDenied") =>
        {
            let reason = detail.unwrap_or_else(|| name.as_str().to_owned());
            log::debug!("network_places: online-accounts refused this app: {reason}");
            NetworkPlaces::Denied { reason }
        }
        Err(e) => {
            log::warn!("network_places: online-accounts unavailable: {e}");
            NetworkPlaces::Unavailable {
                reason: e.to_string(),
            }
        }
    }
}

/// Call `org.arlen.Accounts1.ListAccounts` and return its `(id, provider, identity,
/// presentation)` tuples (already capability-filtered to this app's grant by the
/// daemon). Errors when the session bus or the daemon is unreachable.
async fn fetch_account_listings() -> Result<Vec<(String, String, String, String)>, zbus::Error> {
    let conn = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.arlen.Accounts1",
        "/org/arlen/Accounts1",
        "org.arlen.Accounts1",
    )
    .await?;
    proxy.call("ListAccounts", &()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_subsystem_is_not_an_empty_list() {
        // The whole point: these two must not be the same value. A sidebar that
        // cannot tell them apart tells a person they have no accounts when the
        // truth is that this image has no accounts daemon.
        let absent = classify(Err(zbus::Error::InvalidReply));
        let none_configured = classify(Ok(Vec::new()));
        assert!(matches!(absent, NetworkPlaces::Unavailable { .. }));
        assert_eq!(none_configured, NetworkPlaces::Configured { places: Vec::new() });
        assert_ne!(absent, none_configured);
    }

    #[test]
    fn a_refusal_is_not_an_absence_either() {
        // The third state, and the one that only appears once the daemon exists:
        // with nobody owning the name every error is ServiceUnknown, so today this
        // arm is reached only here. It is in the contract because the contract is
        // what the sidebar will read when accountsd does arrive - telling a person
        // "not available on this system" about accounts they can see in Settings
        // sends them to install something that is already installed.
        let denied = classify(Err(zbus::Error::MethodError(
            "org.freedesktop.DBus.Error.AccessDenied"
                .try_into()
                .unwrap(),
            Some("not granted to this app".into()),
            zbus::message::Message::method_call("/", "Whatever")
                .unwrap()
                .build(&())
                .unwrap(),
        )));
        assert!(matches!(denied, NetworkPlaces::Denied { .. }), "{denied:?}");
        assert_ne!(denied, classify(Err(zbus::Error::InvalidReply)));
    }

    #[test]
    fn a_configured_account_still_reaches_the_sidebar() {
        let one = classify(Ok(vec![(
            "acct-1".to_string(),
            "nextcloud".to_string(),
            "someone@example.org".to_string(),
            "Files".to_string(),
        )]));
        match one {
            NetworkPlaces::Configured { places } => assert_eq!(places.len(), 1),
            other => panic!("expected the mapped place, got {other:?}"),
        }
    }

    #[test]
    fn the_wire_shape_names_the_state() {
        // The caller matches on `state`, so an absence cannot be destructured as a
        // list by accident - which is the property, not the JSON.
        let json = serde_json::to_string(&classify(Err(zbus::Error::InvalidReply))).unwrap();
        assert!(json.contains("\"state\":\"unavailable\""), "{json}");
        let json = serde_json::to_string(&classify(Ok(Vec::new()))).unwrap();
        assert!(json.contains("\"state\":\"configured\""), "{json}");
    }
}
