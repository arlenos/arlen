//! The management ObjectManager surface for `org.arlen.Accounts1` (online-accounts-
//! plan.md §3.1, slice 1 of per-caller object visibility). The raw
//! `GetManagedObjects` tree is exposed ONLY to the Settings management app (the
//! canonical `settings` app-id), never to every same-uid app: a vanilla D-Bus
//! ObjectManager is globally readable, which would re-open the exact ambient
//! account-enumeration hole this daemon exists to close (an app learning which
//! accounts exist, even ungranted ones).
//!
//! Enforcement is IN-CODE, not the bus-policy `.conf` alone: on the session bus
//! every app shares one uid, so a bus-policy cannot distinguish the Settings
//! caller from any other same-uid app. A non-management caller receives an EMPTY
//! tree, never a leak; unprivileged apps enumerate via the caller-filtered
//! `ListAccounts` instead. The per-account property maps are returned INLINE here;
//! separately-served live `/Accounts/{id}` objects (whose property reads would each
//! need the same gate) and the directed `InterfacesAdded`/`Removed` signals are the
//! follow-on slice (#570).

use crate::config::AccountConfig;
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// The canonical app-id of the Settings management app (identity.rs resolves the
/// Settings binary to this). The only caller allowed the full account tree.
pub const MANAGEMENT_APP_ID: &str = "settings";

/// The D-Bus interface name carried by each per-account object entry.
pub const ACCOUNT_IFACE: &str = "org.arlen.Accounts1.Account";

/// The `GetManagedObjects` reply shape (`a{oa{sa{sv}}}`): object path -> interface
/// name -> property name -> value.
pub type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The object path of the per-account entry for `id`. `None` for an id that would
/// make a malformed or traversing path (ids are config-file stems; keep the path
/// well-formed and traversal-free, mirroring the daemon's fail-closed discipline).
fn account_object_path(id: &str) -> Option<OwnedObjectPath> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return None;
    }
    OwnedObjectPath::try_from(format!("/org/arlen/Accounts1/Accounts/{id}")).ok()
}

/// The non-secret metadata property map for one account. Never a token or secret:
/// only the id, provider, login identity, presentation name and service labels the
/// management UI renders. `Presentation` falls back to the identity when unset.
fn account_property_map(a: &AccountConfig) -> HashMap<String, OwnedValue> {
    let str_val = |s: &str| OwnedValue::try_from(Value::from(s.to_string())).ok();
    let services: Vec<String> = a.services.iter().map(|s| s.as_key().to_string()).collect();
    let mut m = HashMap::new();
    if let Some(v) = str_val(&a.id) {
        m.insert("Id".to_string(), v);
    }
    if let Some(v) = str_val(&a.provider) {
        m.insert("Provider".to_string(), v);
    }
    if let Some(v) = str_val(&a.identity) {
        m.insert("Identity".to_string(), v);
    }
    let presentation = a.presentation.as_deref().unwrap_or(&a.identity);
    if let Some(v) = str_val(presentation) {
        m.insert("Presentation".to_string(), v);
    }
    if let Ok(v) = OwnedValue::try_from(Value::from(services)) {
        m.insert("Services".to_string(), v);
    }
    m
}

/// Build the full managed-objects tree from every loaded account config. NOT gated;
/// use [`managed_objects_gated`] at the D-Bus surface.
pub fn build_managed_objects(configs: &[AccountConfig]) -> ManagedObjects {
    let mut out = ManagedObjects::new();
    for a in configs {
        let Some(path) = account_object_path(&a.id) else {
            continue;
        };
        let mut ifaces = HashMap::new();
        ifaces.insert(ACCOUNT_IFACE.to_string(), account_property_map(a));
        out.insert(path, ifaces);
    }
    out
}

/// The settings-gated managed-objects tree: the full tree ONLY for the Settings
/// management app, an EMPTY tree for every other caller (no account is enumerable
/// by a same-uid app). A fail-closed empty caller id (resolution failed) is not the
/// management app, so it too gets nothing.
pub fn managed_objects_gated(caller_app_id: &str, configs: &[AccountConfig]) -> ManagedObjects {
    if caller_app_id == MANAGEMENT_APP_ID {
        build_managed_objects(configs)
    } else {
        ManagedObjects::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AccountConfig, Service};

    fn account(id: &str) -> AccountConfig {
        AccountConfig {
            id: id.to_string(),
            provider: "nextcloud".to_string(),
            identity: "user@example.com".to_string(),
            presentation: None,
            services: vec![Service::Files, Service::Calendar],
            grants: vec![],
            files: None,
        }
    }

    #[test]
    fn the_management_app_gets_the_full_tree() {
        let configs = vec![account("work"), account("personal")];
        let tree = managed_objects_gated("settings", &configs);
        assert_eq!(tree.len(), 2);
        let path = account_object_path("work").unwrap();
        let iface = &tree[&path][ACCOUNT_IFACE];
        assert_eq!(iface["Id"], OwnedValue::try_from(Value::from("work")).unwrap());
        assert!(iface.contains_key("Services"));
    }

    #[test]
    fn a_non_management_caller_gets_nothing() {
        // The security property: no same-uid app enumerates another's accounts.
        let configs = vec![account("work")];
        assert!(managed_objects_gated("com.example.app", &configs).is_empty());
    }

    #[test]
    fn a_failed_caller_resolution_empty_id_gets_nothing() {
        // resolve_caller_app_id maps any failure to an empty id -> fail-closed.
        assert!(managed_objects_gated("", &[account("work")]).is_empty());
    }

    #[test]
    fn a_traversing_account_id_is_dropped_from_the_tree() {
        let configs = vec![account("../../etc/x"), account("ok")];
        let tree = managed_objects_gated("settings", &configs);
        assert_eq!(tree.len(), 1);
        assert!(tree.contains_key(&account_object_path("ok").unwrap()));
    }

    #[test]
    fn presentation_falls_back_to_identity_when_unset() {
        let tree = managed_objects_gated("settings", &[account("a")]);
        let iface = &tree[&account_object_path("a").unwrap()][ACCOUNT_IFACE];
        assert_eq!(
            iface["Presentation"],
            OwnedValue::try_from(Value::from("user@example.com")).unwrap()
        );
    }
}
