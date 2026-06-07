//! Conversation session persistence for the harness (A8).
//!
//! Sessions are stored as one JSON file so the history rail survives a restart.
//! The backend is deliberately schema-agnostic: it persists whatever JSON array
//! the frontend sends (the `Session[]` shape lives in the frontend), so the
//! transcript shape can evolve without a Rust mirror. Both commands fail soft,
//! a missing or unreadable store just reads as no history, and a bad write is
//! reported but never corrupts the existing file (the write is atomic).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Refuse a pathological payload rather than write an unbounded file. A
/// conversation history of any sane size is far under this.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// `$XDG_DATA_HOME/arlen/harness/sessions.json`, or `None` if no data dir
/// resolves (then persistence is a no-op and the rail is in-memory only).
fn sessions_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("arlen").join("harness").join("sessions.json"))
}

/// Read the persisted sessions array from `path`, or an empty array if the file
/// is absent, unreadable, malformed, or not a JSON array. Pure over the path so
/// it is unit-tested without touching the real data dir.
fn load_from(path: &Path) -> Value {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Value::Array(Vec::new());
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_array() => v,
        _ => Value::Array(Vec::new()),
    }
}

/// Write the sessions array to `path` atomically (write a sibling temp file,
/// then rename over the target), so a crash mid-write never truncates the
/// existing history. Refuses a non-array or an oversized payload.
fn save_to(path: &Path, sessions: &Value) -> Result<(), String> {
    if !sessions.is_array() {
        return Err("sessions must be a JSON array".into());
    }
    let serialized = serde_json::to_string(sessions).map_err(|e| e.to_string())?;
    if serialized.len() > MAX_BYTES {
        return Err("sessions payload too large".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Same-directory temp so the rename is atomic on one filesystem.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serialized).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the persisted conversation sessions, newest first. Never errors, a
/// fresh or unreadable store reads as an empty array.
#[tauri::command]
pub async fn harness_sessions_load() -> Value {
    match sessions_path() {
        Some(path) => load_from(&path),
        None => Value::Array(Vec::new()),
    }
}

/// Persist the conversation sessions. Atomic and bounded; fails soft (the rail
/// stays in-memory) rather than corrupting the store.
#[tauri::command]
pub async fn harness_sessions_save(sessions: Value) -> Result<(), String> {
    match sessions_path() {
        Some(path) => save_to(&path, &sessions),
        None => Err("no data directory resolved".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_file_loads_empty_array() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sessions.json");
        assert_eq!(load_from(&path), json!([]));
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("sessions.json");
        let data = json!([{ "id": "a", "title": "Hi", "messages": [], "createdAt": 1 }]);
        save_to(&path, &data).unwrap();
        assert_eq!(load_from(&path), data);
    }

    #[test]
    fn a_non_array_payload_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sessions.json");
        assert!(save_to(&path, &json!({ "not": "an array" })).is_err());
    }

    #[test]
    fn a_corrupt_file_loads_empty_not_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sessions.json");
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert_eq!(load_from(&path), json!([]));
    }

    #[test]
    fn an_oversized_payload_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sessions.json");
        let big = "x".repeat(MAX_BYTES + 1);
        assert!(save_to(&path, &json!([{ "blob": big }])).is_err());
    }
}
