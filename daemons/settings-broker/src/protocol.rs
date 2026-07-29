//! The broker's wire protocol: length-prefixed JSON (a 4-byte big-endian length
//! then the body), the same framing the sibling daemons use, capped before any
//! allocation so a bad length cannot make the broker reserve a large buffer.

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::broker::ChangeSignal;

/// The largest frame the broker will read or write. A settings write is a key
/// and a small value, so this is generous by design.
pub const MAX_FRAME: usize = 64 * 1024;

/// One key/value pair in a write request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyWrite {
    /// The dotted key.
    pub key: String,
    /// The proposed value.
    pub value: Value,
}

/// What a caller asks of the broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum Request {
    /// Write one app's settings. The whole batch is validated before any of it
    /// is applied, so a request either lands entirely or not at all.
    ///
    /// Note what is NOT here: the caller does not name the config file, the
    /// key's type, or the layer to write to. It names the app and the keys, and
    /// the broker resolves the rest from that app's declared schema. A request
    /// that could name its own target could write another app's settings, or
    /// write a key the schema says is not user-writable.
    Write {
        /// The app whose settings are being written.
        app_id: String,
        /// The keys and their new values.
        writes: Vec<KeyWrite>,
    },
}

/// What the broker answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum Response {
    /// The write landed. `changed` is exactly the keys whose value is now
    /// different, which is what subscribers act on; it is empty when the write
    /// was a no-op.
    Changed {
        /// The app whose settings changed.
        app_id: String,
        /// The keys that actually changed.
        changed: Vec<String>,
    },
    /// The schema refused a key. Names the key so the caller can say which,
    /// rather than reporting an anonymous failure.
    Refused {
        /// The offending key.
        key: String,
        /// Why it was refused, in plain language.
        reason: String,
    },
    /// The app is not known to the broker: no declared schema was found for it.
    UnknownApp {
        /// The app that was asked for.
        app_id: String,
    },
    /// Something else went wrong (io, a malformed config file).
    Error {
        /// What happened.
        message: String,
    },
}

impl From<ChangeSignal> for Response {
    fn from(signal: ChangeSignal) -> Self {
        Response::Changed {
            app_id: signal.app_id,
            changed: signal.changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_request_round_trips() {
        let request = Request::Write {
            app_id: "org.example.App".into(),
            writes: vec![KeyWrite {
                key: "theme".into(),
                value: Value::String("dark".into()),
            }],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
    }

    #[test]
    fn each_response_round_trips() {
        for response in [
            Response::Changed {
                app_id: "a".into(),
                changed: vec!["k".into()],
            },
            Response::Refused {
                key: "k".into(),
                reason: "nope".into(),
            },
            Response::UnknownApp { app_id: "a".into() },
            Response::Error {
                message: "boom".into(),
            },
        ] {
            let json = serde_json::to_string(&response).unwrap();
            assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
        }
    }

    /// A signal converts to the wire response without losing the no-op case.
    #[test]
    fn an_empty_signal_becomes_an_empty_changed_set() {
        let response: Response = ChangeSignal {
            app_id: "a".into(),
            changed: Vec::new(),
        }
        .into();
        assert_eq!(
            response,
            Response::Changed {
                app_id: "a".into(),
                changed: Vec::new()
            }
        );
    }
}
