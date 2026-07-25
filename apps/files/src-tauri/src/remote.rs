//! FM-R15: the "Network / Remote" sidebar places. Fetches the configured
//! online-accounts / rclone mounts from the online-accounts daemon's capability-gated
//! `org.arlen.Accounts1.ListAccounts` and maps them to the places the PlacesSidebar
//! renders. The mapping (classification, display name, order) is the tested
//! `arlen_file_browser_core::remote` logic; this is only the D-Bus fetch.

use arlen_file_browser_core::remote::{remote_places_from_listings, RemotePlace};

/// The online-accounts / rclone mounts the files app is granted, as sidebar places.
/// Degrades to an empty list on any failure (the daemon is absent, or the app holds no
/// accounts grant so `ListAccounts` returns nothing) - the sidebar then shows no remote
/// places rather than surfacing an error, matching the "configured mounts, not a scan"
/// intent.
#[tauri::command]
pub async fn network_places() -> Vec<RemotePlace> {
    match fetch_account_listings().await {
        Ok(listings) => remote_places_from_listings(&listings),
        Err(e) => {
            log::warn!("network_places: online-accounts unavailable: {e}");
            Vec::new()
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
