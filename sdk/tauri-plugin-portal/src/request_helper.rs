//! Subscribe-then-call pattern for `org.freedesktop.portal.Request`.
//!
//! The portal spec returns a Request object path; the actual
//! response arrives as a `Response` signal at that path. If we
//! subscribe to the signal AFTER the method call, a fast backend
//! can deliver the response before our handler is wired up — the
//! D-Bus daemon does not buffer signals for late subscribers.
//!
//! Every method call therefore:
//! 1. Generates a random `handle_token`.
//! 2. Computes the deterministic Request path from sender + token.
//! 3. Subscribes to Response at that path.
//! 4. Invokes the method (passing the token in `options`).
//! 5. Awaits the signal stream's first item.

use std::collections::HashMap;

use futures_util::StreamExt;
use rand::Rng;
use tokio::time::{timeout, Duration};
use zbus::{
    message::Type as MessageType,
    zvariant::{ObjectPath, OwnedValue, Value},
    Connection, MatchRule, MessageStream,
};

use crate::types::PickerError;

/// How long we wait for the Response signal after issuing the
/// method call. The portal backend itself enforces a 5 min cap on
/// individual picks (F2.5 E13); 6 min here gives the backend room
/// to surface its own timeout response before we declare the
/// request lost.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(360);

/// Generate a token that is safe for D-Bus path segments. Lower-
/// case hex, never starts with a digit (portal frontends have
/// historically barfed on numeric leading characters in some
/// implementations).
pub fn fresh_token() -> String {
    let mut rng = rand::thread_rng();
    let n: u128 = rng.gen();
    format!("t{:032x}", n)
}

/// Subscribe to a single `Response` signal at `request_path` and
/// invoke `method`. Returns `(response_code, results)` from the
/// signal payload.
///
/// The closure is called *after* the subscription is in place to
/// keep the subscribe-then-call invariant. The closure returns the
/// `OwnedObjectPath` actually allocated by the daemon, which we
/// compare against `request_path` so a mis-aligned token aborts
/// rather than waits forever.
pub async fn submit_with_response<F, Fut>(
    connection: &Connection,
    request_path: ObjectPath<'static>,
    method: F,
) -> Result<(u32, HashMap<String, OwnedValue>), PickerError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = zbus::Result<zbus::zvariant::OwnedObjectPath>>,
{
    // Build a match rule for the Response signal at our exact
    // path. zbus's typed `Proxy::receive_*_signal` would also
    // work, but going through MessageStream + MatchRule lets us
    // own the subscription window precisely.
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface("org.freedesktop.portal.Request")
        .map_err(|e| PickerError::Other {
            message: format!("match rule interface: {e}"),
        })?
        .member("Response")
        .map_err(|e| PickerError::Other {
            message: format!("match rule member: {e}"),
        })?
        .path(request_path.clone())
        .map_err(|e| PickerError::Other {
            message: format!("match rule path: {e}"),
        })?
        .build();

    let mut stream = MessageStream::for_match_rule(rule, connection, None)
        .await
        .map_err(PickerError::from_zbus)?;

    // With subscription in place, fire the method.
    let allocated_path = method().await.map_err(PickerError::from_zbus)?;
    if allocated_path.as_str() != request_path.as_str() {
        // Fail fast (Codex review): the predicted path is the only
        // one we subscribed to. Waiting for the timeout would leave
        // the user staring at an unresponsive picker for six
        // minutes. The portal will eventually time out the orphan
        // Request internally; the caller can retry.
        return Err(PickerError::Other {
            message: format!(
                "portal allocated request path {} does not match token-derived path {}; \
                 frontend may not honour handle_token",
                allocated_path.as_str(),
                request_path.as_str()
            ),
        });
    }

    // Wait for the Response signal.
    let message = match timeout(RESPONSE_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(msg))) => msg,
        Ok(Some(Err(e))) => {
            return Err(PickerError::ConnectionLost {
                message: e.to_string(),
            });
        }
        Ok(None) => {
            return Err(PickerError::ConnectionLost {
                message: "signal stream ended".into(),
            });
        }
        Err(_) => {
            return Err(PickerError::Timeout {
                message: format!(
                    "no Response signal within {} seconds",
                    RESPONSE_TIMEOUT.as_secs()
                ),
            });
        }
    };

    // Decode (u, a{sv}) body.
    let body = message.body();
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        body.deserialize().map_err(PickerError::from_zbus)?;
    Ok((code, results))
}

/// Insert a `handle_token` into the options dict so the portal
/// frontend builds the predictable Request path.
pub fn set_handle_token<'a>(
    options: &mut HashMap<&'a str, OwnedValue>,
    token: &'a str,
) -> Result<(), PickerError> {
    let value = Value::new(token.to_string())
        .try_to_owned()
        .map_err(|e| PickerError::Other {
            message: format!("encode handle_token: {e}"),
        })?;
    options.insert("handle_token", value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_proxy::build_request_path;

    /// Tokens are unique across calls and parseable as a path
    /// segment.
    #[test]
    fn fresh_tokens_are_unique() {
        let a = fresh_token();
        let b = fresh_token();
        assert_ne!(a, b);
        assert!(a.starts_with('t'));
        assert_eq!(a.len(), 33);
        // No path-breaking characters.
        for ch in a.chars() {
            assert!(ch.is_ascii_alphanumeric());
        }
    }

    /// Building a request path from sender + token round-trips
    /// through the proxy helper.
    #[test]
    fn token_to_path_round_trips() {
        let token = fresh_token();
        let path = build_request_path(":1.42", &token).unwrap();
        assert!(path.as_str().ends_with(&token));
    }
}
