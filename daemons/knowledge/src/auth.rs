/// Authentication handler for the Graph Daemon.
///
/// Issues capability tokens to connecting applications based on their
/// permission profiles, and verifies tokens on every subsequent request.
///
/// See `docs/architecture/CAPABILITY-TOKENS.md` Sections 7-8.

use crate::identity::{app_id_from_cgroup, app_id_from_pid, process_alive, IdentityError};
use arlen_permissions::{load_profile, load_profile_from, PermissionError, PermissionProfile};

use crate::permission::{profile_mtime, GraphScopeExt};
use crate::token::{CapabilityToken, TokenSigner};
use crate::token_cache::TokenCache;

use thiserror::Error;

/// Errors from authentication operations.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("graph access not granted for {0}")]
    GraphAccessNotGranted(String),
    #[error("token signature invalid")]
    TokenInvalid,
    #[error("token expired")]
    TokenExpired,
    #[error("process {0} no longer alive")]
    ProcessDead(u32),
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("permission: {0}")]
    Permission(#[from] PermissionError),
}

/// Manages token issuing, caching, and verification.
pub struct Authenticator {
    signer: TokenSigner,
    cache: TokenCache,
}

impl Authenticator {
    /// Create a new authenticator with a fresh HMAC key.
    pub fn new() -> Self {
        Self {
            signer: TokenSigner::new(),
            cache: TokenCache::new(),
        }
    }

    /// Issue a token for a connecting process.
    ///
    /// 1. Resolves app_id from PID via `/proc/{pid}/exe`
    /// 2. Loads permission profile
    /// 3. Checks `[graph]` access
    /// 4. Builds and signs token from profile scopes
    /// 5. Caches token with profile mtime
    pub fn issue_token_for_pid(&mut self, pid: u32) -> Result<CapabilityToken, AuthError> {
        // A hardened, non-dumpable peer's /proc/exe is EACCES even to root, so the
        // exe-path resolve fails; fall back to the peer's cgroup unit (not
        // ptrace-gated), which identifies the canonical AI daemons. Without this a
        // hardened ai-agent could READ (the connection resolver already falls back)
        // but its WRITE token issuance failed here, so executor_live never wrote.
        let app_id = match app_id_from_pid(pid) {
            Ok(id) => id,
            Err(e) => app_id_from_cgroup(pid).ok_or(e)?,
        };
        self.issue_token_for_app(&app_id, pid)
    }

    /// Issue a token for a known app_id and PID (skips identity resolution).
    /// Useful for testing and for cases where app_id is already known.
    pub fn issue_token_for_app(
        &mut self,
        app_id: &str,
        pid: u32,
    ) -> Result<CapabilityToken, AuthError> {
        let profile = load_profile(app_id)?;
        self.issue_token_from_profile(app_id, pid, &profile)
    }

    /// Issue a token from an already-loaded profile.
    pub fn issue_token_from_profile(
        &mut self,
        app_id: &str,
        pid: u32,
        profile: &PermissionProfile,
    ) -> Result<CapabilityToken, AuthError> {
        if !profile.has_graph_access() {
            return Err(AuthError::GraphAccessNotGranted(app_id.to_string()));
        }

        let mut token = CapabilityToken::new(
            app_id.to_string(),
            pid,
            profile.to_read_scopes(),
            profile.to_write_scopes(),
            profile.to_relation_scopes(),
            profile.to_instance_scope(),
        )
        .with_delegated_namespaces(profile.delegated_namespaces());

        self.signer.sign(&mut token);

        let mtime = profile_mtime(app_id).ok();
        self.cache
            .insert(app_id.to_string(), token.clone(), mtime);

        Ok(token)
    }

    /// Verify a token presented with a request.
    ///
    /// Checks: HMAC signature, expiration, process liveness.
    pub fn verify_token(&self, token: &CapabilityToken) -> Result<(), AuthError> {
        if !self.signer.verify(token) {
            return Err(AuthError::TokenInvalid);
        }
        if token.is_expired() {
            return Err(AuthError::TokenExpired);
        }
        if !process_alive(token.pid) {
            return Err(AuthError::ProcessDead(token.pid));
        }
        Ok(())
    }

    /// Invalidate a cached token for an app (on `permission.changed` event).
    pub fn invalidate(&mut self, app_id: &str) {
        self.cache.invalidate(app_id);
    }

    /// Invalidate all cached tokens (key rotation, daemon restart).
    pub fn invalidate_all(&mut self) {
        self.cache.invalidate_all();
    }

    /// Get a reference to the signer (for testing).
    #[cfg(test)]
    pub fn signer(&self) -> &TokenSigner {
        &self.signer
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{EntityScope, InstanceScope, RelationScope};
    use std::io::Write;
    use tempfile::TempDir;

    fn load_profile(content: &str) -> PermissionProfile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        // The canonical profile requires an `[info]` section; the inline test
        // bodies below only carry `[graph]`, so prepend a minimal one.
        let content = format!("[info]\napp_id = \"com.test.app\"\n{content}");
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        load_profile_from(f.path(), "com.test.app").unwrap()
    }

    #[test]
    fn test_issue_token_success() {
        let profile = load_profile(
            r#"
[graph]
read = ["system.File", "com.test.*"]
write = ["com.test.*"]
relations = [
    { from = "com.test.Note", to = "system.File", type = "REFERENCES" },
]
instance_scope = "own"
"#,
        );

        let mut auth = Authenticator::new();
        let token = auth
            .issue_token_from_profile("com.test", std::process::id(), &profile)
            .unwrap();

        assert_eq!(token.app_id, "com.test");
        assert!(token.can_read("system.File"));
        assert!(token.can_read("com.test.Note"));
        assert!(token.can_write("com.test.Note"));
        assert!(!token.can_write("system.File"));
        assert!(token.can_create_relation("com.test.Note", "system.File", "REFERENCES"));
        assert_eq!(token.instance_scope, InstanceScope::Own);
        assert!(auth.signer().verify(&token));
    }

    /// The shipped `ai-agent` profile is the executor go-live grant: it must
    /// authorise exactly the one relation the auto-tag workflow writes
    /// (File -[FILE_PART_OF]-> Project) and nothing more. This loads the real
    /// deployed artifact and asserts its normalized scopes whole, not by a few
    /// `can_*` spot checks: a spot check would miss an added relation, an extra
    /// read, or a stray write scope, and since the grant carries the token-wide
    /// `InstanceScope::All` an unnoticed extra relation would become a
    /// privileged all-instances graph write. (The canonical agent binary path
    /// resolving to the app id `ai-agent` is covered separately by the identity
    /// resolver's `test_app_id_from_path_ai_agent_canonical_libexec`, so the two
    /// tests together cover binary -> app id -> exact grant.)
    #[test]
    fn shipped_ai_agent_profile_grants_exactly_the_file_part_of_link() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ai-engine-daemon/dist/permissions/ai-agent.toml");
        let profile =
            load_profile_from(&path, "ai-agent").expect("the shipped ai-agent profile must parse");
        assert!(
            profile.has_graph_access(),
            "the agent needs graph access to write the link"
        );
        let graph = &profile.graph;

        // Exactly one relation scope, the FILE_PART_OF link, and no other.
        let relations = profile.to_relation_scopes();
        assert_eq!(relations.len(), 1, "exactly one relation may be granted");
        assert_eq!(relations[0].from, "system.File");
        assert_eq!(relations[0].to, "system.Project");
        assert_eq!(relations[0].relation_type, "FILE_PART_OF");

        // No node-create (write) scope at all: the agent writes a relation, not
        // a node.
        assert!(
            profile.to_write_scopes().is_empty(),
            "the agent must hold no node write scope"
        );

        // Reads exactly File.{id,path} and Project.{id,root_path}, field-level
        // (fields: Some), with no exclusions and nothing else readable.
        let reads = profile.to_read_scopes();
        assert_eq!(reads.len(), 2, "exactly the two node types it proves over");
        let mut by_type: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for scope in &reads {
            assert!(
                scope.exclude_fields.is_empty(),
                "no exclude_fields expected on the grant"
            );
            let mut fields = scope
                .fields
                .clone()
                .expect("a field-level grant, not a whole-node read");
            fields.sort();
            by_type.insert(scope.entity_type.clone(), fields);
        }
        assert_eq!(
            by_type.get("system.File"),
            Some(&vec!["id".to_string(), "path".to_string()])
        );
        assert_eq!(
            by_type.get("system.Project"),
            Some(&vec!["id".to_string(), "root_path".to_string()])
        );

        // No sensitive-field reads in the grant.
        assert!(
            graph.read_sensitive.is_empty(),
            "the grant must read no sensitive fields"
        );

        // Linking two unowned system nodes is unanchored, so it needs the
        // privileged all-instances scope; the daemon would refuse it otherwise.
        assert_eq!(
            profile.to_instance_scope(),
            InstanceScope::All,
            "the File->Project link is unanchored, so it needs InstanceScope::All"
        );

        // End-to-end coherence: the same path the daemon uses mints a token
        // that can create the link and nothing adjacent.
        let mut auth = Authenticator::new();
        let token = auth
            .issue_token_from_profile("ai-agent", std::process::id(), &profile)
            .unwrap();
        assert!(token.can_create_relation("system.File", "system.Project", "FILE_PART_OF"));
        assert!(!token.can_create_relation("system.File", "system.App", "ACCESSED_BY"));
    }

    #[test]
    fn test_no_graph_permission() {
        let profile = load_profile(
            r#"
[filesystem]
allow = ["~/Documents"]
"#,
        );

        let mut auth = Authenticator::new();
        let result = auth.issue_token_from_profile("com.nograph", 1234, &profile);
        assert!(result.is_err());
        match result.unwrap_err() {
            AuthError::GraphAccessNotGranted(id) => assert_eq!(id, "com.nograph"),
            other => panic!("expected GraphAccessNotGranted, got: {other}"),
        }
    }

    #[test]
    fn test_missing_profile_no_access() {
        let profile = load_profile("");
        let mut auth = Authenticator::new();
        let result = auth.issue_token_from_profile("com.nonexistent", 1234, &profile);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_token_valid() {
        let profile = load_profile("[graph]\nread = [\"system.File\"]\n");

        let mut auth = Authenticator::new();
        let token = auth
            .issue_token_from_profile("com.verify", std::process::id(), &profile)
            .unwrap();

        assert!(auth.verify_token(&token).is_ok());
    }

    #[test]
    fn test_verify_token_tampered() {
        let profile = load_profile("[graph]\nread = [\"system.File\"]\n");

        let mut auth = Authenticator::new();
        let mut token = auth
            .issue_token_from_profile("com.tamper", std::process::id(), &profile)
            .unwrap();

        token.app_id = "com.evil".to_string();
        assert!(matches!(
            auth.verify_token(&token),
            Err(AuthError::TokenInvalid)
        ));
    }

    #[test]
    fn test_verify_token_dead_process() {
        let profile = load_profile("[graph]\nread = [\"system.File\"]\n");

        let mut auth = Authenticator::new();
        let mut token = auth
            .issue_token_from_profile("com.dead", std::process::id(), &profile)
            .unwrap();

        token.pid = 999_999_999;
        auth.signer.sign(&mut token);

        assert!(matches!(
            auth.verify_token(&token),
            Err(AuthError::ProcessDead(999_999_999))
        ));
    }

    #[test]
    fn test_invalidate_cache() {
        let profile = load_profile("[graph]\nread = [\"system.File\"]\n");

        let mut auth = Authenticator::new();
        let _ = auth
            .issue_token_from_profile("com.cache", std::process::id(), &profile)
            .unwrap();

        assert!(auth.cache.get("com.cache").is_some());
        auth.invalidate("com.cache");
        assert!(auth.cache.get("com.cache").is_none());
    }
}
