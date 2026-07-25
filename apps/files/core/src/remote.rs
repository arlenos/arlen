//! FM-R15: the "Network / Remote" places (file-manager-plan.md section C). The file
//! manager surfaces the CONFIGURED online-accounts / rclone mounts as normal browsable
//! places in the PlacesSidebar - cloud drives (Google/Nextcloud/Dropbox/S3) and network
//! shares (SMB/SFTP/WebDAV/FTP). This is the pure model + mapping; the app's src-tauri
//! fetches the account list from the online-accounts daemon (its capability-gated
//! `ListAccounts`) and builds the [`RemoteAccount`] inputs, so `files/core` never
//! depends on the OA daemon crate. No new mount machinery here (that is OA-R1/R3); the
//! place surfaces CONFIGURED mounts, not a network scan, and the live connection/offline
//! row state is an OA-R3 overlay layered on top of these entries.

use serde::Serialize;

/// A configured remote account, as the src-tauri extracts it from the online-accounts
/// daemon's `ListAccounts` reply - the minimal shape the place mapping needs.
#[derive(Debug, Clone)]
pub struct RemoteAccount {
    /// The stable account id (the daemon's `AccountConfig.id`).
    pub id: String,
    /// The provider key (`google`, `nextcloud`, `dropbox`, `smb`, `sftp`, ...).
    pub provider: String,
    /// A human name for the row: the account's presentation name, else its identity.
    pub display_name: String,
}

/// Which broad kind a remote place is, for its icon and sidebar grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemotePlaceKind {
    /// A cloud storage provider (drive/object sync: Google, Nextcloud, Dropbox, S3).
    Cloud,
    /// A network file share (SMB/SFTP/WebDAV/FTP/NFS).
    NetworkShare,
}

/// One entry in the file manager's "Network / Remote" sidebar section: a configured
/// remote mount presented as a browsable place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemotePlace {
    /// The account id (the src-tauri uses it to resolve the cap-std mount path).
    pub id: String,
    /// The row label.
    pub name: String,
    /// Cloud vs network share (drives the icon).
    pub kind: RemotePlaceKind,
}

/// Classify a provider key into a place kind. The file-sharing PROTOCOL providers are
/// network shares; every other provider (a named cloud service, or an unknown one) is
/// treated as cloud storage - the conservative default keeps a new/unrecognised
/// provider VISIBLE as a cloud drive rather than silently dropped. Case-insensitive.
pub fn provider_kind(provider: &str) -> RemotePlaceKind {
    match provider.trim().to_ascii_lowercase().as_str() {
        "smb" | "cifs" | "sftp" | "ssh" | "ftp" | "ftps" | "webdav" | "dav" | "nfs" => {
            RemotePlaceKind::NetworkShare
        }
        _ => RemotePlaceKind::Cloud,
    }
}

/// Map the configured remote accounts to sidebar places, sorted by display name (then
/// id) for a stable order - the daemon's account-list order is an implementation
/// detail, so the sidebar must not depend on it.
pub fn remote_places(accounts: &[RemoteAccount]) -> Vec<RemotePlace> {
    let mut places: Vec<RemotePlace> = accounts
        .iter()
        .map(|a| RemotePlace {
            id: a.id.clone(),
            name: a.display_name.clone(),
            kind: provider_kind(&a.provider),
        })
        .collect();
    places.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    places
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: &str, provider: &str, name: &str) -> RemoteAccount {
        RemoteAccount {
            id: id.into(),
            provider: provider.into(),
            display_name: name.into(),
        }
    }

    #[test]
    fn share_protocols_are_network_shares_and_the_rest_is_cloud() {
        for p in ["smb", "CIFS", "sftp", "ssh", "ftp", "webdav", "dav", "nfs"] {
            assert_eq!(provider_kind(p), RemotePlaceKind::NetworkShare, "{p} is a share");
        }
        for p in ["google", "nextcloud", "dropbox", "s3", "onedrive", "somethingnew"] {
            assert_eq!(provider_kind(p), RemotePlaceKind::Cloud, "{p} is cloud");
        }
    }

    #[test]
    fn accounts_map_to_places_sorted_by_name() {
        let places = remote_places(&[
            acct("a2", "sftp", "Work SFTP"),
            acct("a1", "google", "Alice Drive"),
        ]);
        assert_eq!(places.len(), 2);
        // Sorted by name: "Alice Drive" before "Work SFTP".
        assert_eq!(places[0].name, "Alice Drive");
        assert_eq!(places[0].kind, RemotePlaceKind::Cloud);
        assert_eq!(places[1].name, "Work SFTP");
        assert_eq!(places[1].kind, RemotePlaceKind::NetworkShare);
    }

    #[test]
    fn the_mapping_is_order_independent() {
        let a = remote_places(&[acct("x", "smb", "Shared"), acct("y", "google", "Drive")]);
        let b = remote_places(&[acct("y", "google", "Drive"), acct("x", "smb", "Shared")]);
        assert_eq!(a, b, "sidebar order must not depend on the account-list order");
    }

    #[test]
    fn a_kind_serialises_kebab_case_for_the_frontend() {
        assert_eq!(
            serde_json::to_string(&RemotePlaceKind::NetworkShare).unwrap(),
            "\"network-share\""
        );
    }

    #[test]
    fn no_accounts_is_no_places() {
        assert!(remote_places(&[]).is_empty());
    }
}
