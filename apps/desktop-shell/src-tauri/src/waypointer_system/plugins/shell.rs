/// Shell command plugin: executes shell commands (prefix ">").

use crate::waypointer_system::plugin::*;

pub struct ShellPlugin;

impl WaypointerPlugin for ShellPlugin {
    fn id(&self) -> &str { "core.shell" }
    fn name(&self) -> &str { "Shell Command" }
    fn description(&self) -> &str { "Execute shell commands. Shift+Enter runs inside the default terminal." }
    fn prefix(&self) -> Option<&str> { Some(">") }
    fn priority(&self) -> u32 { 0 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let cmd = query.trim();
        if cmd.is_empty() {
            return Vec::new();
        }
        vec![
            SearchResult {
                id: "shell-run".into(),
                title: format!("Run: {cmd}"),
                description: Some("Execute in shell".into()),
                icon: Some("terminal".into()),
                relevance: 1.0,
                action: Action::Execute { command: cmd.into() },
                plugin_id: String::new(),
            },
            SearchResult {
                id: "shell-terminal".into(),
                title: format!("Run in terminal: {cmd}"),
                description: Some("Open in terminal emulator".into()),
                icon: Some("terminal-square".into()),
                relevance: 0.8,
                action: Action::Custom {
                    handler: "shell_terminal".into(),
                    data: serde_json::json!({ "command": cmd }),
                },
                plugin_id: String::new(),
            },
        ]
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        match &result.action {
            Action::Execute { command } => {
                std::process::Command::new("sh")
                    .args(["-c", command])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| PluginError::ExecuteFailed(e.to_string()))?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
