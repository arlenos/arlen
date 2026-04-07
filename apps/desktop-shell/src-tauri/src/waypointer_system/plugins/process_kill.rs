/// Process kill plugin: list and kill processes (prefix "kill").

use crate::waypointer_system::plugin::*;

pub struct ProcessKillPlugin;

impl WaypointerPlugin for ProcessKillPlugin {
    fn id(&self) -> &str { "core.process-kill" }
    fn name(&self) -> &str { "Kill Process" }
    fn prefix(&self) -> Option<&str> { Some("kill") }
    fn priority(&self) -> u32 { 0 }
    fn max_results(&self) -> usize { 20 }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        // Read /proc to list user processes matching the query.
        let uid = unsafe { libc::getuid() };
        let query_lower = query.trim().to_lowercase();
        let mut results = Vec::new();

        let proc_dir = match std::fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => return results,
        };

        for entry in proc_dir.flatten() {
            let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Check ownership.
            let status_path = format!("/proc/{pid}/status");
            let status = match std::fs::read_to_string(&status_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let proc_uid: u32 = status
                .lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            if proc_uid != uid {
                continue;
            }

            let name = status
                .lines()
                .find(|l| l.starts_with("Name:"))
                .map(|l| l.split_whitespace().last().unwrap_or("").to_string())
                .unwrap_or_default();

            if name.is_empty() || (!query_lower.is_empty() && !name.to_lowercase().contains(&query_lower)) {
                continue;
            }

            let mem_kb: u64 = status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            results.push(SearchResult {
                id: format!("kill-{pid}"),
                title: format!("{name} (PID {pid})"),
                description: Some(format!("{:.1} MB", mem_kb as f64 / 1024.0)),
                icon: Some("skull".into()),
                relevance: 0.5,
                action: Action::Custom {
                    handler: "kill_process".into(),
                    data: serde_json::json!({ "pid": pid }),
                },
                plugin_id: String::new(),
            });

            if results.len() >= 20 {
                break;
            }
        }

        // Sort by memory (highest first).
        results.sort_by(|a, b| {
            let mem_a = extract_mem(&a.description);
            let mem_b = extract_mem(&b.description);
            mem_b.partial_cmp(&mem_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    fn execute(&self, result: &SearchResult) -> Result<(), PluginError> {
        if let Action::Custom { ref data, .. } = result.action {
            if let Some(pid) = data.get("pid").and_then(|v| v.as_u64()) {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }
        Ok(())
    }
}

fn extract_mem(desc: &Option<String>) -> f64 {
    desc.as_ref()
        .and_then(|d| d.trim_end_matches(" MB").parse().ok())
        .unwrap_or(0.0)
}
