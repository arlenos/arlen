/// Projects plugin: searches the Knowledge Graph for active projects
/// and activates Focus Mode via a custom action.
///
/// Query prefix: `p:`. Without query, returns the most-recently-accessed
/// projects. With a filter after `p:`, matches against name or root path
/// (case-insensitive substring).
///
/// The `execute()` path only returns a signal for the frontend; the
/// actual focus activation happens via the existing `activate_focus`
/// Tauri command so `shell.toml` persistence and Event Bus emission
/// stay in one place (projects.rs).

use serde_json::json;

use crate::projects::{self, Project};
use crate::waypointer_system::plugin::*;

pub struct ProjectsPlugin;

impl WaypointerPlugin for ProjectsPlugin {
    fn id(&self) -> &str { "core.projects" }
    fn name(&self) -> &str { "Projects" }
    fn description(&self) -> &str { "Search detected projects from the Knowledge Graph and activate Focus Mode." }
    fn prefix(&self) -> Option<&str> { Some("p:") }
    fn priority(&self) -> u32 { 5 }
    fn max_results(&self) -> usize { 10 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let filter = query.trim().to_lowercase();
        let list = list_active_projects();

        let matched: Vec<&Project> = if filter.is_empty() {
            list.iter().collect()
        } else {
            list.iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&filter)
                        || p.root_path.to_lowercase().contains(&filter)
                })
                .collect()
        };

        matched
            .into_iter()
            .take(self.max_results())
            .map(|p| SearchResult {
                id: format!("project-{}", p.id),
                title: p.name.clone(),
                description: Some(p.root_path.clone()),
                icon: Some("folder-kanban".into()),
                // Recency bumps relevance so fresh projects surface first.
                relevance: project_relevance(p),
                action: Action::Custom {
                    handler: "focus".into(),
                    data: json!({
                        "projectId": p.id,
                        "projectName": p.name,
                        "rootPath": p.root_path,
                        "accentColor": p.accent_color,
                    }),
                },
                plugin_id: String::new(),
            })
            .collect()
    }

    fn execute(&self, _result: &SearchResult) -> Result<(), PluginError> {
        // The Custom "focus" action is dispatched by the frontend, which
        // then calls activate_focus to persist state and emit to the
        // Event Bus (keeping all focus lifecycle in projects.rs).
        Ok(())
    }
}

/// Queries the graph daemon for active projects. Returns an empty list
/// if the daemon is unreachable (expected during local dev without
/// the knowledge daemon running).
fn list_active_projects() -> Vec<Project> {
    let cypher = "MATCH (p:Project) WHERE p.status = 'active' \
                  RETURN p.id, p.name, p.description, p.root_path, \
                  p.accent_color, p.icon, p.status, p.created_at, \
                  p.last_accessed, p.inferred, p.confidence, p.promoted \
                  ORDER BY p.last_accessed DESC";
    match projects::graph_query(cypher) {
        Ok(raw) => projects::parse_projects_result(&raw),
        Err(e) => {
            log::debug!("projects plugin: graph query failed: {e}");
            Vec::new()
        }
    }
}

/// Relevance boost by recency: projects touched in the last day score near
/// 1.0, falling off over roughly a month.
fn project_relevance(p: &Project) -> f32 {
    const DAY_MS: i64 = 86_400_000;
    const HORIZON_DAYS: f32 = 30.0;
    let now = chrono::Utc::now().timestamp_millis();
    match p.last_accessed {
        Some(t) => {
            let age_days = ((now - t).max(0) / DAY_MS) as f32;
            (1.0 - (age_days / HORIZON_DAYS)).clamp(0.5, 1.0)
        }
        None => 0.5,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_project(id: &str, name: &str, last_accessed: Option<i64>) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            description: None,
            root_path: format!("/home/u/{name}"),
            accent_color: None,
            icon: None,
            status: "active".into(),
            created_at: 0,
            last_accessed,
            inferred: false,
            confidence: 90,
            promoted: true,
        }
    }

    #[test]
    fn relevance_recent_is_high() {
        let now = chrono::Utc::now().timestamp_millis();
        let p = mk_project("1", "fresh", Some(now));
        assert!(project_relevance(&p) > 0.95);
    }

    #[test]
    fn relevance_old_is_low() {
        let month_ago = chrono::Utc::now().timestamp_millis() - 30 * 86_400_000;
        let p = mk_project("2", "old", Some(month_ago));
        assert!((project_relevance(&p) - 0.5).abs() < 0.05);
    }

    #[test]
    fn relevance_without_access_is_midpoint() {
        let p = mk_project("3", "never", None);
        assert_eq!(project_relevance(&p), 0.5);
    }

    #[test]
    fn plugin_metadata() {
        let p = ProjectsPlugin;
        assert_eq!(p.id(), "core.projects");
        assert_eq!(p.prefix(), Some("p:"));
        assert_eq!(p.priority(), 5);
    }
}
