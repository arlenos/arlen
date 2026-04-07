/// Module error tracking with auto-disable.
///
/// Counts runtime errors per module. After a threshold (10 errors within
/// a 60-second window), the module is auto-disabled and the user notified.
///
/// See `docs/architecture/module-system.md`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Errors within this window count toward auto-disable.
const ERROR_WINDOW: Duration = Duration::from_secs(60);

/// Auto-disable after this many errors within the window.
const ERROR_THRESHOLD: usize = 10;

/// Error tracking record for one module.
#[derive(Debug, Clone)]
struct ErrorRecord {
    /// Timestamps of recent errors (within the window).
    recent_errors: Vec<Instant>,
    /// Most recent error message.
    last_error: String,
    /// Whether the module was auto-disabled by the tracker.
    auto_disabled: bool,
}

impl ErrorRecord {
    fn new() -> Self {
        Self {
            recent_errors: Vec::new(),
            last_error: String::new(),
            auto_disabled: false,
        }
    }

    /// Prune errors older than the window.
    fn prune(&mut self) {
        let cutoff = Instant::now() - ERROR_WINDOW;
        self.recent_errors.retain(|t| *t > cutoff);
    }

    /// Current error count (within window).
    fn count(&mut self) -> usize {
        self.prune();
        self.recent_errors.len()
    }
}

/// Error status for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleErrorStatus {
    pub module_id: String,
    pub error_count: usize,
    pub last_error: String,
    pub auto_disabled: bool,
}

/// Tracks errors per module and auto-disables misbehaving modules.
pub struct ModuleErrorTracker {
    records: HashMap<String, ErrorRecord>,
}

impl ModuleErrorTracker {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Record an error for a module. Returns true if the module was just
    /// auto-disabled (threshold crossed).
    pub fn record_error(&mut self, module_id: &str, error: &str) -> bool {
        let record = self
            .records
            .entry(module_id.to_string())
            .or_insert_with(ErrorRecord::new);

        record.recent_errors.push(Instant::now());
        record.last_error = error.to_string();

        // Check threshold.
        if !record.auto_disabled && record.count() >= ERROR_THRESHOLD {
            record.auto_disabled = true;
            return true; // just crossed threshold
        }

        false
    }

    /// Get the current error count for a module (within the window).
    pub fn error_count(&mut self, module_id: &str) -> usize {
        match self.records.get_mut(module_id) {
            Some(r) => r.count(),
            None => 0,
        }
    }

    /// Whether a module was auto-disabled.
    pub fn is_auto_disabled(&self, module_id: &str) -> bool {
        self.records
            .get(module_id)
            .map(|r| r.auto_disabled)
            .unwrap_or(false)
    }

    /// Reset errors and auto-disabled state (for manual re-enable).
    pub fn reset(&mut self, module_id: &str) {
        self.records.remove(module_id);
    }

    /// Get error status for all tracked modules.
    pub fn all_statuses(&mut self) -> Vec<ModuleErrorStatus> {
        let mut statuses = Vec::new();
        for (id, record) in &mut self.records {
            record.prune();
            if record.recent_errors.is_empty() && !record.auto_disabled {
                continue;
            }
            statuses.push(ModuleErrorStatus {
                module_id: id.clone(),
                error_count: record.recent_errors.len(),
                last_error: record.last_error.clone(),
                auto_disabled: record.auto_disabled,
            });
        }
        statuses.sort_by(|a, b| a.module_id.cmp(&b.module_id));
        statuses
    }

    /// Get list of auto-disabled module IDs (for persistence).
    pub fn auto_disabled_ids(&self) -> Vec<String> {
        self.records
            .iter()
            .filter(|(_, r)| r.auto_disabled)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

use std::sync::Mutex;

/// Shared error tracker state.
pub type ErrorTrackerState = Mutex<ModuleErrorTracker>;

/// Record a module error from the frontend (ModuleHost onError).
#[tauri::command]
pub fn record_module_error(
    module_id: String,
    error: String,
    tracker: tauri::State<'_, ErrorTrackerState>,
    loader: tauri::State<'_, crate::modules::ModuleLoaderState>,
    app: tauri::AppHandle,
) -> bool {
    let mut tracker = tracker.lock().unwrap();
    let just_disabled = tracker.record_error(&module_id, &error);

    if just_disabled {
        log::warn!("module {module_id} auto-disabled after {} errors", ERROR_THRESHOLD);
        // Auto-disable in the loader.
        let mut loader = loader.lock().unwrap();
        loader.set_enabled(&module_id, false);

        // Notify frontend.
        use tauri::Emitter;
        let _ = app.emit(
            "lunaris://module-auto-disabled",
            serde_json::json!({
                "module_id": module_id,
                "error_count": ERROR_THRESHOLD,
                "last_error": error,
            }),
        );
    }

    just_disabled
}

/// Get error statuses for all modules.
#[tauri::command]
pub fn get_module_errors(
    tracker: tauri::State<'_, ErrorTrackerState>,
) -> Vec<ModuleErrorStatus> {
    let mut tracker = tracker.lock().unwrap();
    tracker.all_statuses()
}

/// Reset errors for a module (user re-enabling).
#[tauri::command]
pub fn reset_module_errors(
    module_id: String,
    tracker: tauri::State<'_, ErrorTrackerState>,
) {
    let mut tracker = tracker.lock().unwrap();
    tracker.reset(&module_id);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_below_threshold() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..9 {
            let disabled = tracker.record_error("com.test", &format!("error {i}"));
            assert!(!disabled);
        }
        assert_eq!(tracker.error_count("com.test"), 9);
        assert!(!tracker.is_auto_disabled("com.test"));
    }

    #[test]
    fn test_auto_disable_at_threshold() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..9 {
            tracker.record_error("com.test", &format!("error {i}"));
        }
        // 10th error should trigger auto-disable.
        let disabled = tracker.record_error("com.test", "error 9");
        assert!(disabled);
        assert!(tracker.is_auto_disabled("com.test"));
    }

    #[test]
    fn test_auto_disable_only_triggers_once() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..10 {
            tracker.record_error("com.test", &format!("error {i}"));
        }
        // 11th should not trigger again.
        let disabled = tracker.record_error("com.test", "error 10");
        assert!(!disabled);
    }

    #[test]
    fn test_reset() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..10 {
            tracker.record_error("com.test", &format!("error {i}"));
        }
        assert!(tracker.is_auto_disabled("com.test"));

        tracker.reset("com.test");
        assert!(!tracker.is_auto_disabled("com.test"));
        assert_eq!(tracker.error_count("com.test"), 0);
    }

    #[test]
    fn test_separate_modules() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..10 {
            tracker.record_error("com.buggy", &format!("error {i}"));
        }
        assert!(tracker.is_auto_disabled("com.buggy"));
        assert!(!tracker.is_auto_disabled("com.good"));
        assert_eq!(tracker.error_count("com.good"), 0);
    }

    #[test]
    fn test_auto_disabled_ids() {
        let mut tracker = ModuleErrorTracker::new();
        for i in 0..10 {
            tracker.record_error("com.a", &format!("e{i}"));
        }
        tracker.record_error("com.b", "e0");

        let disabled = tracker.auto_disabled_ids();
        assert!(disabled.contains(&"com.a".to_string()));
        assert!(!disabled.contains(&"com.b".to_string()));
    }

    #[test]
    fn test_all_statuses() {
        let mut tracker = ModuleErrorTracker::new();
        tracker.record_error("com.a", "err");
        tracker.record_error("com.b", "err");

        let statuses = tracker.all_statuses();
        assert_eq!(statuses.len(), 2);
    }

    #[test]
    fn test_no_errors_returns_empty() {
        let mut tracker = ModuleErrorTracker::new();
        assert!(tracker.all_statuses().is_empty());
        assert_eq!(tracker.error_count("com.test"), 0);
    }
}
