/// Permission token system: issue, validate, revoke.
///
/// Tokens are bound to a PID and validated against `/proc/{pid}/exe`.
/// PID reuse is caught by checking that the exe path still matches.
///
/// See `docs/architecture/permission-system.md` (Token section).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;

/// Errors from token validation.
#[derive(Debug, Error)]
pub enum TokenError {
    #[error("token not found")]
    NotFound,
    #[error("process {pid} no longer alive")]
    ProcessDead { pid: u32 },
    #[error("exe path mismatch: expected {expected}, found {actual}")]
    ExeMismatch { expected: String, actual: String },
    #[error("cannot read /proc/{pid}/exe: {source}")]
    ProcReadError { pid: u32, source: std::io::Error },
}

/// A permission token bound to a process.
#[derive(Debug, Clone)]
pub struct PermissionToken {
    /// Random 32-byte hex string.
    pub token: String,
    /// Application identifier.
    pub app_id: String,
    /// Process ID that owns this token.
    pub pid: u32,
    /// Expected executable path (resolved at issue time).
    pub exe_path: PathBuf,
    /// When the token was issued.
    pub issued_at: SystemTime,
}

/// Stores and validates active tokens.
pub struct TokenStore {
    tokens: HashMap<String, PermissionToken>,
}

impl TokenStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            tokens: HashMap::new(),
        }
    }

    /// Issue a new token for an app process.
    ///
    /// Resolves `/proc/{pid}/exe` to get the canonical exe path, generates
    /// a random token, and stores the mapping.
    pub fn issue(&mut self, app_id: &str, pid: u32) -> Result<String, TokenError> {
        let exe_path = resolve_exe(pid)?;
        let token_str = generate_token();

        self.tokens.insert(
            token_str.clone(),
            PermissionToken {
                token: token_str.clone(),
                app_id: app_id.into(),
                pid,
                exe_path,
                issued_at: SystemTime::now(),
            },
        );

        Ok(token_str)
    }

    /// Issue with an explicit exe path (for testing without /proc).
    pub fn issue_with_path(
        &mut self,
        app_id: &str,
        pid: u32,
        exe_path: PathBuf,
    ) -> String {
        let token_str = generate_token();
        self.tokens.insert(
            token_str.clone(),
            PermissionToken {
                token: token_str.clone(),
                app_id: app_id.into(),
                pid,
                exe_path,
                issued_at: SystemTime::now(),
            },
        );
        token_str
    }

    /// Validate a token: exists, process alive, exe matches.
    pub fn validate(&self, token: &str) -> Result<&PermissionToken, TokenError> {
        let entry = self.tokens.get(token).ok_or(TokenError::NotFound)?;

        // Check PID still alive.
        if !Path::new(&format!("/proc/{}", entry.pid)).exists() {
            return Err(TokenError::ProcessDead { pid: entry.pid });
        }

        // Check exe path still matches (catches PID reuse).
        let current_exe = resolve_exe(entry.pid)?;
        if current_exe != entry.exe_path {
            return Err(TokenError::ExeMismatch {
                expected: entry.exe_path.display().to_string(),
                actual: current_exe.display().to_string(),
            });
        }

        Ok(entry)
    }

    /// Validate without the /proc exe check (for testing or when
    /// the process is known to be the caller itself).
    pub fn validate_exists(&self, token: &str) -> Result<&PermissionToken, TokenError> {
        self.tokens.get(token).ok_or(TokenError::NotFound)
    }

    /// Revoke a token manually.
    pub fn revoke(&mut self, token: &str) -> bool {
        self.tokens.remove(token).is_some()
    }

    /// Remove tokens for dead processes.
    pub fn cleanup(&mut self) -> usize {
        let before = self.tokens.len();
        self.tokens.retain(|_, entry| {
            Path::new(&format!("/proc/{}", entry.pid)).exists()
        });
        before - self.tokens.len()
    }

    /// Number of active tokens.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// Read `/proc/{pid}/exe` symlink and canonicalize.
fn resolve_exe(pid: u32) -> Result<PathBuf, TokenError> {
    std::fs::read_link(format!("/proc/{pid}/exe")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            TokenError::ProcessDead { pid }
        } else {
            TokenError::ProcReadError { pid, source: e }
        }
    })
}

/// Generate a random 32-byte hex token.
fn generate_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Combine multiple entropy sources for a unique token.
    let mut hasher = DefaultHasher::new();
    SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    let h1 = hasher.finish();

    let mut hasher2 = DefaultHasher::new();
    h1.hash(&mut hasher2);
    // Add a counter-like component.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .hash(&mut hasher2);
    let h2 = hasher2.finish();

    format!("{h1:016x}{h2:016x}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 32); // 16 hex digits * 2
    }

    #[test]
    fn test_issue_and_validate_self() {
        // Issue a token for our own process.
        let mut store = TokenStore::new();
        let token = store.issue("com.test", std::process::id()).unwrap();
        assert_eq!(store.len(), 1);

        // Validate: our process is alive and exe matches.
        let entry = store.validate(&token).unwrap();
        assert_eq!(entry.app_id, "com.test");
        assert_eq!(entry.pid, std::process::id());
    }

    #[test]
    fn test_validate_not_found() {
        let store = TokenStore::new();
        assert!(matches!(
            store.validate_exists("nonexistent"),
            Err(TokenError::NotFound)
        ));
    }

    #[test]
    fn test_revoke() {
        let mut store = TokenStore::new();
        let token = store
            .issue_with_path("com.test", std::process::id(), PathBuf::from("/fake"))
            ;
        assert_eq!(store.len(), 1);

        assert!(store.revoke(&token));
        assert_eq!(store.len(), 0);
        assert!(!store.revoke(&token)); // already revoked
    }

    #[test]
    fn test_validate_dead_process() {
        let mut store = TokenStore::new();
        // PID 999999999 is almost certainly dead.
        let token = store.issue_with_path(
            "com.test",
            999_999_999,
            PathBuf::from("/usr/bin/nonexistent"),
        );

        assert!(matches!(
            store.validate(&token),
            Err(TokenError::ProcessDead { pid: 999_999_999 })
        ));
    }

    #[test]
    fn test_cleanup_removes_dead() {
        let mut store = TokenStore::new();

        // Token for our (alive) process.
        store.issue_with_path("alive", std::process::id(), PathBuf::from("/a"));
        // Token for a dead process.
        store.issue_with_path("dead", 999_999_999, PathBuf::from("/b"));

        assert_eq!(store.len(), 2);
        let removed = store.cleanup();
        assert_eq!(removed, 1);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_issue_resolves_exe() {
        let mut store = TokenStore::new();
        let token = store.issue("com.test", std::process::id()).unwrap();
        let entry = store.validate_exists(&token).unwrap();
        // exe_path should be non-empty and exist.
        assert!(!entry.exe_path.as_os_str().is_empty());
    }

    #[test]
    fn test_exe_mismatch_detected() {
        let mut store = TokenStore::new();
        // Issue with a fake exe path for our own PID.
        let token = store.issue_with_path(
            "com.test",
            std::process::id(),
            PathBuf::from("/usr/bin/definitely-not-our-binary"),
        );

        // Validation should catch the mismatch.
        assert!(matches!(
            store.validate(&token),
            Err(TokenError::ExeMismatch { .. })
        ));
    }

    #[test]
    fn test_multiple_tokens_same_app() {
        let mut store = TokenStore::new();
        let t1 = store.issue_with_path("com.test", 1001, PathBuf::from("/a"));
        let t2 = store.issue_with_path("com.test", 1002, PathBuf::from("/b"));
        assert_ne!(t1, t2);
        assert_eq!(store.len(), 2);
    }
}
