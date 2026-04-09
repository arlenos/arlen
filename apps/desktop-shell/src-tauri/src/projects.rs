/// Project system: Focus Mode state and Tauri commands.
///
/// Projects are detected by the Knowledge Daemon. The shell reads them
/// via the graph query socket and manages Focus Mode state locally.

use std::sync::Arc;

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

// ── Tauri Commands ──────────────────────────────────────────────────────

/// List all active projects from the Knowledge Graph.
#[tauri::command]
pub async fn list_projects() -> Result<Vec<Project>, String> {
    // TODO: query /run/lunaris/knowledge.sock
    // MATCH (p:Project {status: 'active'}) RETURN p
    Ok(vec![])
}

/// Get a single project by ID.
#[tauri::command]
pub async fn get_project(project_id: String) -> Result<Option<Project>, String> {
    // TODO: query graph
    let _ = project_id;
    Ok(None)
}

/// Enter Focus Mode for a project.
#[tauri::command]
pub async fn activate_focus(
    project_id: String,
    state: State<'_, Arc<ProjectsState>>,
    app: AppHandle,
) -> Result<(), String> {
    let project = get_project(project_id.clone())
        .await?
        .ok_or_else(|| format!("project not found: {project_id}"))?;

    let fs = FocusState {
        project_id: Some(project.id.clone()),
        project_name: Some(project.name.clone()),
        root_path: Some(project.root_path.clone()),
        accent_color: project.accent_color.clone(),
        activated_at: Some(chrono::Utc::now().timestamp_millis()),
    };

    *state.focus.write().await = fs.clone();
    save_focus_to_shell_toml(&fs);

    app.emit("focus:activated", &fs).map_err(|e| e.to_string())?;
    log::info!("focus activated: {} ({})", project.name, project.id);
    Ok(())
}

/// Leave Focus Mode.
#[tauri::command]
pub async fn deactivate_focus(
    state: State<'_, Arc<ProjectsState>>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let focus = state.focus.read().await;
        if let Some(ref name) = focus.project_name {
            let secs = focus
                .activated_at
                .map(|t| (chrono::Utc::now().timestamp_millis() - t) / 1000)
                .unwrap_or(0);
            log::info!("focus deactivated: {name} ({secs}s)");
        }
    }

    *state.focus.write().await = FocusState::default();
    clear_focus_in_shell_toml();

    app.emit("focus:deactivated", ()).map_err(|e| e.to_string())?;
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
    // Try loading persisted state.
    Ok(load_focus_from_shell_toml())
}

// ── Persistence (shell.toml [focus] section) ────────────────────────────

fn shell_toml_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|p| p.join("lunaris/shell.toml"))
}

fn save_focus_to_shell_toml(fs: &FocusState) {
    let Some(path) = shell_toml_path() else { return };
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
    let Some(path) = shell_toml_path() else { return };
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
        assert!(json.contains("abc"));
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

        // Write existing content.
        std::fs::write(
            &path,
            "[night_light]\nenabled = false\ntemperature = 3400\n",
        )
        .unwrap();

        let fs = FocusState {
            project_id: Some("id-1".into()),
            project_name: Some("Test".into()),
            root_path: Some("/test".into()),
            accent_color: None,
            activated_at: None,
        };

        // Manually call helpers with the temp path.
        let mut doc = read_shell_toml(&path);
        let mut focus = toml::map::Map::new();
        focus.insert("project_id".into(), toml::Value::String("id-1".into()));
        focus.insert("project_name".into(), toml::Value::String("Test".into()));
        if let toml::Value::Table(ref mut t) = doc {
            t.insert("focus".into(), toml::Value::Table(focus));
        }
        let content = toml::to_string_pretty(&doc).unwrap();
        std::fs::write(&path, &content).unwrap();

        // Read back.
        let doc2 = read_shell_toml(&path);
        let ft = doc2.get("focus").unwrap().as_table().unwrap();
        assert_eq!(
            ft.get("project_id").unwrap().as_str(),
            Some("id-1")
        );
        // Night light should still be there.
        assert!(doc2.get("night_light").is_some());
    }
}
