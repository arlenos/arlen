//! D-Bus interface implementations.
//!
//! One module per portal interface. Each module defines a struct, the
//! `#[zbus::interface]` impl, and any helpers private to that interface.

pub mod file_chooser;
pub mod open_uri;
pub mod options;
pub mod print;
pub mod screenshot;
pub mod screencast;
pub mod settings;

use std::collections::HashMap;

use tracing::warn;
use zbus::zvariant::{OwnedValue, Value};

/// The public frontend whose `app_id` verdict every impl backend consumes.
const FRONTEND_NAME: &str = "org.freedesktop.portal.Desktop";

/// The one sentence every interface gives a sender that is not the frontend.
pub(crate) const NOT_THE_FRONTEND: &str = "caller is not the xdg-desktop-portal frontend";

/// A results map carrying a reason, for the responses the spec gives no other
/// place to put one.
///
/// Four interfaces had a byte-identical private copy of this and the fifth had
/// none - which is why `Print` alone refused with a bare `(2, HashMap::new())`:
/// the right response code and no reason at all, where the same refusal on
/// FileChooser, OpenURI, Screenshot and ScreenCast carries the sentence. A
/// helper nobody can reach is a helper somebody reimplements as an empty map.
pub(crate) fn error_results(message: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(message.to_string()).try_to_owned() {
        map.insert("arlen-error".to_string(), owned);
    }
    map
}

/// The whole refusal, code and reason together, so the two cannot drift apart
/// again. `OTHER` (2) is the portal spec's "not success, not cancelled": the
/// person did not decline this, the daemon did.
pub(crate) fn refuse_not_the_frontend() -> (u32, HashMap<String, OwnedValue>) {
    (crate::request::response::OTHER, error_results(NOT_THE_FRONTEND))
}

/// Whether this call came from the `xdg-desktop-portal` frontend.
///
/// An impl backend trusts the `app_id` ARGUMENT because the frontend
/// authenticated the app before re-dispatching to us. That reasoning
/// only holds for the frontend. A process that reaches our impl name
/// directly supplies the argument itself, and every impl interface is
/// affected: a FileChooser call names any grantee it likes, a Screenshot
/// call captures the screen unauthenticated. Absence of evidence must not
/// grant access, so the check is positive: verified frontend, or refuse.
///
/// The comparison is unique-name to unique-name — `GetNameOwner` returns
/// the owner's `:1.x`, which is exactly what the message header carries.
/// `arlen.portal` registers us for the frontend alone, so nothing else
/// is a legitimate caller and no supported flow is refused here.
pub(crate) async fn sender_is_frontend(
    connection: &zbus::Connection,
    sender: Option<&str>,
) -> bool {
    let owner = match zbus::fdo::DBusProxy::new(connection).await {
        Ok(proxy) => match proxy.get_name_owner(FRONTEND_NAME.try_into().unwrap()).await {
            Ok(owner) => Some(owner.as_str().to_string()),
            Err(e) => {
                // Unowned means the frontend is not running, so
                // whoever called us is not it.
                warn!("cannot resolve the {FRONTEND_NAME} owner: {e}");
                None
            }
        },
        Err(e) => {
            warn!("cannot reach the bus daemon to attest the sender: {e}");
            None
        }
    };
    let attested = sender_matches_owner(sender, owner.as_deref());
    // Both names are bus-assigned unique names, not caller text, so
    // logging them is safe and is the only way to tell a mismatch
    // apart from a sender the header never carried.
    tracing::debug!(?sender, ?owner, attested, "sender attestation");
    attested
}

/// The verdict itself, split out from the bus round-trip so every
/// branch is checkable without a broker. Both unknowns — a message
/// carrying no sender, an owner we could not resolve — are answers of
/// "not attested", never "close enough".
pub(crate) fn sender_matches_owner(sender: Option<&str>, owner: Option<&str>) -> bool {
    match (sender, owner) {
        (Some(sender), Some(owner)) => sender == owner,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every interface file that guards on the frontend, and the source of each.
    /// Read as text rather than driven over a bus: the refusal needs a real
    /// sender and a real name owner, so there is no unit that can reach it. What
    /// this holds is the shape, which is what drifted.
    const GUARDED: &[(&str, &str)] = &[
        ("file_chooser.rs", include_str!("file_chooser.rs")),
        ("open_uri.rs", include_str!("open_uri.rs")),
        ("print.rs", include_str!("print.rs")),
        ("screencast.rs", include_str!("screencast.rs")),
        ("screenshot.rs", include_str!("screenshot.rs")),
    ];

    #[test]
    fn every_frontend_refusal_goes_through_the_one_helper() {
        // `Print` refused with `(2, HashMap::new())` - the right response code
        // and no reason at all - while the other four returned the sentence. The
        // cause was four byte-identical private copies of `error_results` and a
        // fifth interface that had none, so the helper was unreachable exactly
        // where it was needed. One helper now, and this is what keeps it one.
        for (name, source) in GUARDED {
            let guards = source.matches("sender_is_frontend(").count();
            assert!(guards > 0, "{name} is listed as guarded and guards nothing");
            let refusals = source.matches("refuse_not_the_frontend()").count();
            assert!(
                refusals > 0,
                "{name} guards on the frontend but does not refuse through the shared helper"
            );
        }
    }

    #[test]
    fn no_interface_keeps_its_own_copy_of_the_results_helper() {
        // The duplication was the mechanism, not an untidiness: a helper that
        // lives four times is one somebody reimplements as an empty map rather
        // than reaching for.
        for (name, source) in GUARDED {
            assert!(
                !source.contains("fn error_results(message: &str)"),
                "{name} has its own copy of error_results again; there is one in this module"
            );
        }
    }

    #[test]
    fn the_refusal_carries_a_reason_and_the_not_success_code() {
        // A refusal must be distinguishable from both halves of a normal answer:
        // not SUCCESS (the call went through) and not CANCELLED (the person said
        // no). The daemon said no, and the map says why.
        let (code, results) = refuse_not_the_frontend();
        assert_eq!(code, crate::request::response::OTHER);
        let reason = results.get("arlen-error").expect("a refusal states its reason");
        // `OwnedValue`'s Display quotes a string, so compare the contained text
        // rather than its rendering.
        let text: String = reason.clone().try_into().expect("the reason is a string");
        assert_eq!(text, NOT_THE_FRONTEND);
    }
}
