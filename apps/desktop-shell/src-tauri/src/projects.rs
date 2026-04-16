/// Project system: Focus Mode state, graph queries, and Tauri commands.
///
/// Projects are detected by the Knowledge Daemon. The shell queries them
/// via the graph daemon socket and manages Focus Mode state locally.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

use prost::Message;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;

// ── Types ───────────────────────────────────────────────────────────────

/// A project as seen by the frontend (camelCase JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub root_path: String,
    pub accent_color: Option<String>,
    pub icon: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub last_accessed: Option<i64>,
    pub inferred: bool,
    pub confidence: u8,
    pub promoted: bool,
}

/// Focus Mode state persisted to shell.toml and sent to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FocusState {
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub root_path: Option<String>,
    pub accent_color: Option<String>,
    pub activated_at: Option<i64>,
}

/// Managed state shared across Tauri commands.
pub struct ProjectsState {
    pub focus: RwLock<FocusState>,
}

impl ProjectsState {
    /// Create empty state (no active focus).
    pub fn new() -> Self {
        Self {
            focus: RwLock::new(FocusState::default()),
        }
    }
}

// ── Graph Query Client ──────────────────────────────────────────────────

const KNOWLEDGE_SOCKET: &str = "/run/lunaris/knowledge.sock";

/// Send a Cypher query to the Knowledge Daemon and return the raw result.
fn graph_query(cypher: &str) -> Result<String, String> {
    let socket = std::env::var("LUNARIS_DAEMON_SOCKET")
        .unwrap_or_else(|_| KNOWLEDGE_SOCKET.to_string());

    let mut stream =
        UnixStream::connect(&socket).map_err(|e| format!("graph connect: {e}"))?;

    let query_bytes = cypher.as_bytes();
    let len = (query_bytes.len() as u32).to_be_bytes();
    stream.write_all(&len).map_err(|e| format!("graph write len: {e}"))?;
    stream
        .write_all(query_bytes)
        .map_err(|e| format!("graph write query: {e}"))?;
    stream.flush().map_err(|e| format!("graph flush: {e}"))?;

    // Read response: 4-byte BE length + UTF-8 result.
    let mut resp_len = [0u8; 4];
    stream
        .read_exact(&mut resp_len)
        .map_err(|e| format!("graph read len: {e}"))?;
    let resp_size = u32::from_be_bytes(resp_len) as usize;
    if resp_size > 4 * 1024 * 1024 {
        return Err(format!("graph response too large: {resp_size}"));
    }

    let mut buf = vec![0u8; resp_size];
    stream
        .read_exact(&mut buf)
        .map_err(|e| format!("graph read body: {e}"))?;

    String::from_utf8(buf).map_err(|e| format!("graph utf8: {e}"))
}

/// Parse pipe-delimited rows from a Ladybug text result.
///
/// Format:
/// ```text
/// p.id|p.name|p.description|p.root_path|...\n
/// uuid|name|desc|/path|...\n
/// ```
fn parse_projects_result(raw: &str) -> Vec<Project> {
    if raw.trim().is_empty() || raw.starts_with("ERROR") {
        return Vec::new();
    }

    let mut lines = raw.lines();

    // First line is the header -- skip it.
    let Some(_header) = lines.next() else {
        return Vec::new();
    };

    let mut projects = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 12 {
            continue;
        }

        let opt = |i: usize| -> Option<String> {
            let s = cols[i].trim();
            if s.is_empty() { None } else { Some(s.to_string()) }
        };
        let i64_or = |i: usize, default: i64| -> i64 {
            cols[i].trim().parse().unwrap_or(default)
        };
        let bool_col = |i: usize| -> bool {
            matches!(cols[i].trim(), "True" | "true" | "1")
        };

        projects.push(Project {
            id: cols[0].trim().to_string(),
            name: cols[1].trim().to_string(),
            description: opt(2),
            root_path: cols[3].trim().to_string(),
            accent_color: opt(4),
            icon: opt(5),
            status: cols[6].trim().to_string(),
            created_at: i64_or(7, 0),
            last_accessed: {
                let v = i64_or(8, 0);
                if v == 0 { None } else { Some(v) }
            },
            inferred: bool_col(9),
            confidence: cols[10].trim().parse().unwrap_or(0),
            promoted: bool_col(11),
        });
    }

    log::info!("parsed {} projects from graph", projects.len());
    projects
}

// ── Tauri Commands ──────────────────────────────────────────────────────

/// List all active projects from the Knowledge Graph.
#[tauri::command]
pub async fn list_projects() -> Result<Vec<Project>, String> {
    let result = match graph_query(
        "MATCH (p:Project) WHERE p.status = 'active' \
         RETURN p.id, p.name, p.description, p.root_path, \
         p.accent_color, p.icon, p.status, p.created_at, \
         p.last_accessed, p.inferred, p.confidence, p.promoted \
         ORDER BY p.last_accessed DESC",
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("list_projects: graph query failed (is knowledge daemon running?): {e}");
            return Ok(Vec::new());
        }
    };
    Ok(parse_projects_result(&result))
}

/// Get a single project by ID.
#[tauri::command]
pub async fn get_project(project_id: String) -> Result<Option<Project>, String> {
    let id_esc = project_id.replace('\'', "\\'");
    let result = match graph_query(&format!(
        "MATCH (p:Project {{id: '{id_esc}'}}) \
         RETURN p.id, p.name, p.description, p.root_path, \
         p.accent_color, p.icon, p.status, p.created_at, \
         p.last_accessed, p.inferred, p.confidence, p.promoted"
    )) {
        Ok(r) => r,
        Err(e) => {
            log::debug!("get_project: graph query failed: {e}");
            return Ok(None);
        }
    };
    Ok(parse_projects_result(&result).into_iter().next())
}

/// Enter Focus Mode for a project.
#[tauri::command]
pub async fn activate_focus(
    project_id: String,
    project_name: String,
    root_path: String,
    accent_color: Option<String>,
    state: State<'_, Arc<ProjectsState>>,
    app: AppHandle,
) -> Result<(), String> {
    let fs = FocusState {
        project_id: Some(project_id.clone()),
        project_name: Some(project_name.clone()),
        root_path: Some(root_path.clone()),
        accent_color: accent_color.clone(),
        activated_at: Some(chrono::Utc::now().timestamp_millis()),
    };

    *state.focus.write().await = fs.clone();
    save_focus_to_shell_toml(&fs);

    app.emit("focus:activated", &fs)
        .map_err(|e| e.to_string())?;

    // Emit focus.activated to Event Bus for Notification Daemon.
    let suppress = load_suppress_list(&root_path);
    emit_focus_activated(&project_id, &project_name, &root_path, &accent_color, &suppress);

    log::info!("focus activated: {project_name} ({project_id})");
    Ok(())
}

/// Leave Focus Mode.
#[tauri::command]
pub async fn deactivate_focus(
    state: State<'_, Arc<ProjectsState>>,
    app: AppHandle,
) -> Result<(), String> {
    let (project_id, duration_secs);
    {
        let focus = state.focus.read().await;
        project_id = focus.project_id.clone().unwrap_or_default();
        duration_secs = focus
            .activated_at
            .map(|t| ((chrono::Utc::now().timestamp_millis() - t) / 1000) as u64)
            .unwrap_or(0);
        if let Some(ref name) = focus.project_name {
            log::info!("focus deactivated: {name} ({duration_secs}s)");
        }
    }

    *state.focus.write().await = FocusState::default();
    clear_focus_in_shell_toml();

    app.emit("focus:deactivated", ())
        .map_err(|e| e.to_string())?;

    emit_focus_deactivated(&project_id, duration_secs);
    Ok(())
}

/// Return the current focus state (may be restored from shell.toml).
#[tauri::command]
pub async fn get_focus_state(
    state: State<'_, Arc<ProjectsState>>,
) -> Result<Option<FocusState>, String> {
    let focus = state.focus.read().await;
    if focus.project_id.is_some() {
        return Ok(Some(focus.clone()));
    }
    Ok(load_focus_from_shell_toml())
}

// ── Event Bus Emission ──────────────────────────────────────────────────

mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/lunaris.eventbus.rs"));
}

const PRODUCER_SOCKET: &str = "/run/lunaris/event-bus-producer.sock";

fn emit_to_event_bus(event_type: &str, payload: Vec<u8>) {
    let socket = std::env::var("LUNARIS_PRODUCER_SOCKET")
        .unwrap_or_else(|_| PRODUCER_SOCKET.to_string());

    let event = proto::Event {
        id: uuid::Uuid::now_v7().to_string(),
        r#type: event_type.to_string(),
        timestamp: chrono::Utc::now().timestamp_micros(),
        source: "desktop-shell".to_string(),
        pid: std::process::id(),
        session_id: String::new(),
        payload,
        uid: unsafe { libc::getuid() },
        project_id: String::new(),
    };

    let encoded = event.encode_to_vec();
    let len = (encoded.len() as u32).to_be_bytes();

    let send = || -> Result<(), std::io::Error> {
        let mut stream = std::os::unix::net::UnixStream::connect(&socket)?;
        stream.write_all(&len)?;
        stream.write_all(&encoded)?;
        stream.flush()?;
        Ok(())
    };

    if let Err(e) = send() {
        log::debug!("event bus emit {event_type}: {e}");
    }
}

fn emit_focus_activated(
    project_id: &str,
    project_name: &str,
    root_path: &str,
    accent_color: &Option<String>,
    suppress_apps: &[String],
) {
    let payload = proto::FocusActivatedPayload {
        project_id: project_id.to_string(),
        project_name: project_name.to_string(),
        root_path: root_path.to_string(),
        accent_color: accent_color.clone().unwrap_or_default(),
        suppress_notifications_from: suppress_apps.to_vec(),
    };
    emit_to_event_bus("focus.activated", payload.encode_to_vec());
}

fn emit_focus_deactivated(project_id: &str, duration_seconds: u64) {
    let payload = proto::FocusDeactivatedPayload {
        project_id: project_id.to_string(),
        duration_seconds,
    };
    emit_to_event_bus("focus.deactivated", payload.encode_to_vec());
}

// ── Suppress List ───────────────────────────────────────────────────────

/// Load suppress_notifications_from from the project's .project file.
fn load_suppress_list(root_path: &str) -> Vec<String> {
    let project_file = PathBuf::from(root_path).join(".project");
    if !project_file.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&project_file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let config: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    config
        .get("focus")
        .and_then(|f| f.get("suppress_notifications_from"))
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ── Persistence (shell.toml [focus] section) ────────────────────────────

fn shell_toml_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("lunaris/shell.toml"))
}

fn save_focus_to_shell_toml(fs: &FocusState) {
    let Some(path) = shell_toml_path() else {
        return;
    };
    let mut doc = read_shell_toml(&path);

    let mut focus = toml::map::Map::new();
    if let Some(ref id) = fs.project_id {
        focus.insert("project_id".into(), toml::Value::String(id.clone()));
    }
    if let Some(ref name) = fs.project_name {
        focus.insert("project_name".into(), toml::Value::String(name.clone()));
    }
    if let Some(ref rp) = fs.root_path {
        focus.insert("root_path".into(), toml::Value::String(rp.clone()));
    }
    if let Some(ref ac) = fs.accent_color {
        focus.insert("accent_color".into(), toml::Value::String(ac.clone()));
    }

    if let toml::Value::Table(ref mut t) = doc {
        t.insert("focus".into(), toml::Value::Table(focus));
    }

    if let Ok(content) = toml::to_string_pretty(&doc) {
        let _ = std::fs::write(&path, content);
    }
}

fn clear_focus_in_shell_toml() {
    let Some(path) = shell_toml_path() else {
        return;
    };
    let mut doc = read_shell_toml(&path);
    if let toml::Value::Table(ref mut t) = doc {
        t.remove("focus");
    }
    if let Ok(content) = toml::to_string_pretty(&doc) {
        let _ = std::fs::write(&path, content);
    }
}

fn load_focus_from_shell_toml() -> Option<FocusState> {
    let path = shell_toml_path()?;
    let doc = read_shell_toml(&path);
    let focus = doc.get("focus")?.as_table()?;

    let project_id = focus
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if project_id.is_none() {
        return None;
    }

    Some(FocusState {
        project_id,
        project_name: focus
            .get("project_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        root_path: focus
            .get("root_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        accent_color: focus
            .get("accent_color")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        activated_at: None,
    })
}

fn read_shell_toml(path: &std::path::Path) -> toml::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| toml::from_str(&c).ok())
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_state_serialization() {
        let fs = FocusState {
            project_id: Some("abc".into()),
            project_name: Some("Test".into()),
            root_path: Some("/a".into()),
            accent_color: Some("#6366f1".into()),
            activated_at: Some(123),
        };
        let json = serde_json::to_string(&fs).unwrap();
        assert!(json.contains("projectId"));
        let parsed: FocusState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.project_id, fs.project_id);
    }

    #[test]
    fn project_serialization() {
        let p = Project {
            id: "uuid".into(),
            name: "Proj".into(),
            description: None,
            root_path: "/a".into(),
            accent_color: None,
            icon: None,
            status: "active".into(),
            created_at: 0,
            last_accessed: None,
            inferred: true,
            confidence: 90,
            promoted: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("rootPath"));
    }

    #[test]
    fn shell_toml_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("shell.toml");
        std::fs::write(
            &path,
            "[night_light]\nenabled = false\ntemperature = 3400\n",
        )
        .unwrap();

        let mut doc = read_shell_toml(&path);
        let mut focus = toml::map::Map::new();
        focus.insert("project_id".into(), toml::Value::String("id-1".into()));
        focus.insert(
            "project_name".into(),
            toml::Value::String("Test".into()),
        );
        if let toml::Value::Table(ref mut t) = doc {
            t.insert("focus".into(), toml::Value::Table(focus));
        }
        let content = toml::to_string_pretty(&doc).unwrap();
        std::fs::write(&path, &content).unwrap();

        let doc2 = read_shell_toml(&path);
        let ft = doc2.get("focus").unwrap().as_table().unwrap();
        assert_eq!(ft.get("project_id").unwrap().as_str(), Some("id-1"));
        assert!(doc2.get("night_light").is_some());
    }

    #[test]
    fn load_suppress_list_from_project_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".project"),
            "[project]\nname = \"t\"\n\n[focus]\nsuppress_notifications_from = [\"slack\", \"discord\"]\n",
        )
        .unwrap();
        let list = load_suppress_list(tmp.path().to_str().unwrap());
        assert_eq!(list, vec!["slack", "discord"]);
    }

    #[test]
    fn load_suppress_list_missing_file() {
        let list = load_suppress_list("/nonexistent/path");
        assert!(list.is_empty());
    }

    #[test]
    fn load_suppress_list_no_focus_section() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".project"),
            "[project]\nname = \"t\"\n",
        )
        .unwrap();
        let list = load_suppress_list(tmp.path().to_str().unwrap());
        assert!(list.is_empty());
    }

    #[test]
    fn emit_to_missing_socket_does_not_panic() {
        std::env::set_var("LUNARIS_PRODUCER_SOCKET", "/tmp/nonexistent-test-socket");
        emit_focus_activated("id", "name", "/path", &None, &[]);
        emit_focus_deactivated("id", 0);
        std::env::remove_var("LUNARIS_PRODUCER_SOCKET");
    }
}
