//! D-Bus interface implementations.
//!
//! One module per portal interface. Each module defines a struct, the
//! `#[zbus::interface]` impl, and any helpers private to that interface.

pub mod file_chooser;
pub mod open_uri;
pub mod options;
pub mod print;
pub mod screenshot;

use tracing::warn;

/// The public frontend whose `app_id` verdict every impl backend consumes.
const FRONTEND_NAME: &str = "org.freedesktop.portal.Desktop";

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
