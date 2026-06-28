//! Pinned-artifact persistence for the harness - the artifact context-menu
//! "Pin" action.
//!
//! Pinning keeps an artifact in a pinned-artifacts surface that survives a
//! restart (`ArtifactMeta.title` exists precisely so a pinned artifact can show
//! a label in that list). The store is a single JSON file, the same shape and
//! discipline as the conversation `sessions` store: the backend is schema-
//! agnostic (it persists whatever JSON array the frontend sends, so the pinned-
//! artifact shape - artifact + id + pinned-at - lives in the frontend and can
//! evolve without a Rust mirror), the write is atomic, and a missing or
//! unreadable store just reads as no pins.
//!
//! Deliberately NOT a knowledge-graph write: an artifact is ephemeral turn
//! output, not a graph observation node; there is no Artifact node type, and the
//! `PinnedMarker` node marks existing observation nodes for retention, not
//! arbitrary content. A KG-backed pin would need a new node type plus a write
//! grant for the harness (multi-component, deployment-gated). The local store is
//! the correct complete seam for the pinned-artifact list; the command surface
//! would stay stable if a KG backing is ever added underneath.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Refuse a pathological payload rather than write an unbounded file. A pinned
/// set of any sane size is far under this; an image artifact's base64 is the
/// largest single entry, so the ceiling is generous.
const MAX_BYTES: usize = 32 * 1024 * 1024;

/// `$XDG_DATA_HOME/arlen/harness/pins.json`, or `None` if no data dir resolves
/// (then pinning is a no-op and the surface is in-memory only).
fn pins_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("arlen").join("harness").join("pins.json"))
}

/// Read the persisted pins array from `path`, or an empty array if the file is
/// absent, unreadable, malformed, or not a JSON array. Pure over the path so it
/// is unit-tested without touching the real data dir.
fn load_from(path: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Value::Array(Vec::new());
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_array() => v,
        _ => Value::Array(Vec::new()),
    }
}

/// Write the pins array to `path` atomically (write a sibling temp file, then
/// rename over the target), so a crash mid-write never truncates the existing
/// pins. Refuses a non-array or an oversized payload.
fn save_to(path: &Path, pins: &Value) -> Result<(), String> {
    if !pins.is_array() {
        return Err("pins must be a JSON array".into());
    }
    let serialized = serde_json::to_string(pins).map_err(|e| e.to_string())?;
    if serialized.len() > MAX_BYTES {
        return Err("pins payload too large".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the persisted pinned artifacts. Never errors, a fresh or unreadable
/// store reads as an empty array.
#[tauri::command]
pub async fn artifact_pins_load() -> Value {
    match pins_path() {
        Some(path) => load_from(&path),
        None => Value::Array(Vec::new()),
    }
}

/// Persist the pinned artifacts. Atomic and bounded; fails soft (the surface
/// stays in-memory) rather than corrupting the store. The frontend sends the
/// full pinned set on each pin/unpin, the same way the sessions rail does.
#[tauri::command]
pub async fn artifact_pins_save(pins: Value) -> Result<(), String> {
    match pins_path() {
        Some(path) => save_to(&path, &pins),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_store_reads_as_an_empty_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pins.json");
        assert_eq!(load_from(&path), json!([]));
    }

    #[test]
    fn pins_round_trip_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pins.json");
        let pins = json!([
            {"id": "p1", "pinnedAt": 1, "artifact": {"kind": "markdown"}},
            {"id": "p2", "pinnedAt": 2, "artifact": {"kind": "code"}}
        ]);
        save_to(&path, &pins).unwrap();
        assert_eq!(load_from(&path), pins);
    }

    #[test]
    fn a_non_array_payload_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pins.json");
        assert!(save_to(&path, &json!({"not": "an array"})).is_err());
        // The store is untouched, so a subsequent load still reads empty.
        assert_eq!(load_from(&path), json!([]));
    }

    #[test]
    fn a_malformed_store_reads_as_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pins.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert_eq!(load_from(&path), json!([]));
    }
}
