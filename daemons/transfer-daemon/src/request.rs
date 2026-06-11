//! The transfer request model: intent and a reference, never the bytes (profile-system-plan.md PR-R4).
//!
//! A `TransferRequest` names a source profile, a destination profile, a flow
//! type and a payload HANDLE. The handle is a reference the broker resolves at
//! delivery, not the content itself, so the policy gate decides on
//! `(source, dest, type)` without ever touching the bytes. The bytes only enter
//! on the deferred broker/deliver path, after the gate has approved.
//!
//! Profile identity is a stable NAME (`personal`, `work`), the thing Settings
//! writes and the policy keys on. The name maps 1:1 to a Linux uid in the live
//! system, but the CORE stays uid-free: name->uid resolution is the broker
//! seam's job at the dual-uid boundary (deferred). A `ProfileId` is validated as
//! a safe identifier because it is interpolated into an audit subject and a
//! per-uid socket-path resolution, so it must never carry a path separator.

use serde::{Deserialize, Serialize};

/// The maximum length of a `File` payload's source path. A real path is far
/// shorter; this bounds a hostile request, it is not a semantic limit.
pub const MAX_SOURCE_PATH_LEN: usize = 4096;

/// A stable profile name. Validated as a safe identifier (`[a-z0-9._-]`, no
/// traversal) so it can never reach a socket path or an audit subject as a
/// separator. The newtype keeps a raw `String` from being passed where a
/// validated id is required.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ProfileId(String);

// A profile id is validated at EVERY untrusted boundary, including
// deserialization: a derived `Deserialize` would wrap any string (a TOML
// policy rule's `source`, a wire `TransferRequest`) into a `ProfileId`
// without the charset/traversal check, defeating the invariant the newtype
// exists for (it reaches an audit subject and a per-uid socket path). The
// manual impl routes through the validating constructor and fails closed.
impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        ProfileId::new(name).ok_or_else(|| serde::de::Error::custom("invalid profile id"))
    }
}

impl ProfileId {
    /// Build a validated profile id, or `None` if the name is not a safe path
    /// component. The charset is `[a-z0-9._-]` with no `..`, no leading or
    /// trailing dot, and no NUL (the charset already excludes every separator).
    /// This is the `arlen-permissions` `is_valid_app_id` discipline applied to a
    /// profile name, because the name reaches both a path and a log line.
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        if is_valid_profile_name(&name) {
            Some(Self(name))
        } else {
            None
        }
    }

    /// The validated name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `name` is a safe profile identifier: non-empty, `[a-z0-9._-]`, no
/// traversal, no leading/trailing dot, no NUL. Mirrors the private
/// `arlen_permissions::is_valid_app_id` rule (a profile id reaches a per-uid
/// socket path and an audit subject, the same hazard an app id has).
fn is_valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name != ".."
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains('\0')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// A cross-profile flow type. Closed so a new flow cannot be smuggled past the
/// policy matcher; a new flow is added by adding a variant, never by a free
/// string. The foundation's clipboard + drag-and-drop + shared folder reduce to
/// these: drag-and-drop is a `File` flow, the persistent shared folder is a
/// separate mode handled outside the per-transfer gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferType {
    /// A clipboard selection (the two-gesture copy/paste flow).
    Clipboard,
    /// A file (drag-and-drop, or a one-off file hand-off).
    File,
}

impl TransferType {
    /// The coarse wire label used as the audit subject suffix (`transfer.<ty>`).
    pub fn as_str(self) -> &'static str {
        match self {
            TransferType::Clipboard => "clipboard",
            TransferType::File => "file",
        }
    }
}

/// A reference to the payload, never the content. The broker resolves it at
/// delivery; the policy layer treats it as opaque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadRef {
    /// A single-use clipboard handle. The broker mints it on the copy gesture
    /// and invalidates it after exactly one transfer (the Qubes
    /// single-use-clear). The CORE only models that the request carries the
    /// handle id; the live minting and clearing are broker-side (deferred).
    Clipboard {
        /// The opaque handle id the broker minted for this selection.
        handle: String,
    },
    /// A file reference. `source_path` is a path WITHIN the source profile's
    /// namespace, opaque to the policy layer; the broker resolves it under the
    /// source uid at delivery.
    File {
        /// The path inside the source profile's namespace.
        source_path: String,
    },
}

/// One transfer request: intent and a reference, never the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRequest {
    /// The profile the bytes originate from.
    pub source: ProfileId,
    /// The profile the bytes are to land in.
    pub dest: ProfileId,
    /// The flow type.
    pub ty: TransferType,
    /// The payload handle (not the content).
    pub payload: PayloadRef,
}

/// Why a transfer request was rejected before it ever reached the policy gate.
/// Validation is fail-closed: a malformed request is an `Err`, never a
/// permissive default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// A `File` payload's source path exceeds [`MAX_SOURCE_PATH_LEN`].
    #[error("source path exceeds {MAX_SOURCE_PATH_LEN} bytes")]
    PathTooLong,
    /// A payload string carried a NUL byte (a path or handle reaching a syscall
    /// must not).
    #[error("payload carries a NUL byte")]
    PayloadNul,
    /// The payload handle / path was empty.
    #[error("payload reference is empty")]
    EmptyPayload,
}

impl TransferRequest {
    /// Validate the request shape, fail-closed. Checks the payload reference is
    /// non-empty, carries no NUL, and (for a `File`) is within the path cap.
    ///
    /// `source` and `dest` are already validated `ProfileId`s by construction
    /// (the newtype's only constructor enforces the charset), so id-validity is
    /// structural. Whether the `(source, dest, type)` flow is permitted is the
    /// policy gate's decision, NOT validation's: a well-formed request can still
    /// be denied. A same-profile transfer (`source == dest`) is not rejected
    /// here; it simply must match a policy rule like any other pair, and in
    /// practice no rule is written for it (so it falls through to default-deny).
    pub fn validate(&self) -> Result<(), RequestError> {
        let reference = match &self.payload {
            PayloadRef::Clipboard { handle } => handle,
            PayloadRef::File { source_path } => source_path,
        };
        if reference.is_empty() {
            return Err(RequestError::EmptyPayload);
        }
        if reference.contains('\0') {
            return Err(RequestError::PayloadNul);
        }
        if let PayloadRef::File { source_path } = &self.payload {
            if source_path.len() > MAX_SOURCE_PATH_LEN {
                return Err(RequestError::PathTooLong);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_id_accepts_a_safe_name_and_rejects_traversal() {
        assert!(ProfileId::new("personal").is_some());
        assert!(ProfileId::new("work-2").is_some());
        assert!(ProfileId::new("org.acme.exam").is_some());
        // Traversal and separators are refused (a profile id reaches a path).
        assert!(ProfileId::new("..").is_none());
        assert!(ProfileId::new("../etc").is_none());
        assert!(ProfileId::new("a/b").is_none());
        assert!(ProfileId::new(".hidden").is_none());
        assert!(ProfileId::new("trailing.").is_none());
        assert!(ProfileId::new("").is_none());
        assert!(ProfileId::new("with space").is_none());
        assert!(ProfileId::new("UPPER").is_none());
        assert!(ProfileId::new("nul\0byte").is_none());
    }

    fn pid(name: &str) -> ProfileId {
        ProfileId::new(name).expect("valid test profile id")
    }

    #[test]
    fn deserializing_a_profile_id_re_validates() {
        // A valid id round-trips.
        let ok: ProfileId = serde_json::from_str("\"work\"").unwrap();
        assert_eq!(ok.as_str(), "work");
        // A traversal / separator / case id is REFUSED on the wire, not wrapped.
        for bad in ["\"../../etc/passwd\"", "\"a/b/c\"", "\"UPPER\"", "\"..\"", "\"with space\""] {
            assert!(
                serde_json::from_str::<ProfileId>(bad).is_err(),
                "{bad} should be rejected by ProfileId::deserialize"
            );
        }
        // The same guard fires through a whole TransferRequest body and a TOML rule.
        let req = r#"{"source":"../etc","dest":"personal","ty":"file","payload":{"file":{"source_path":"/x"}}}"#;
        assert!(serde_json::from_str::<TransferRequest>(req).is_err());
    }

    #[test]
    fn validate_accepts_a_well_formed_file_request() {
        let req = TransferRequest {
            source: pid("work"),
            dest: pid("personal"),
            ty: TransferType::File,
            payload: PayloadRef::File {
                source_path: "/home/work/report.pdf".into(),
            },
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_rejects_an_empty_or_nul_or_oversized_payload() {
        let empty = TransferRequest {
            source: pid("work"),
            dest: pid("personal"),
            ty: TransferType::Clipboard,
            payload: PayloadRef::Clipboard { handle: "".into() },
        };
        assert_eq!(empty.validate(), Err(RequestError::EmptyPayload));

        let nul = TransferRequest {
            source: pid("work"),
            dest: pid("personal"),
            ty: TransferType::File,
            payload: PayloadRef::File {
                source_path: "/home/work/a\0b".into(),
            },
        };
        assert_eq!(nul.validate(), Err(RequestError::PayloadNul));

        let long = TransferRequest {
            source: pid("work"),
            dest: pid("personal"),
            ty: TransferType::File,
            payload: PayloadRef::File {
                source_path: "x".repeat(MAX_SOURCE_PATH_LEN + 1),
            },
        };
        assert_eq!(long.validate(), Err(RequestError::PathTooLong));
    }

    #[test]
    fn a_request_round_trips_through_json() {
        let req = TransferRequest {
            source: pid("work"),
            dest: pid("personal"),
            ty: TransferType::Clipboard,
            payload: PayloadRef::Clipboard {
                handle: "sel-7".into(),
            },
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: TransferRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, req);
    }
}
