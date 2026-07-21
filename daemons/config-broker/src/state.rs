//! The canonical AI master-switch state, owned by the broker.
//!
//! The state lives as `state.toml` in a 0700 directory the user's
//! normal uid cannot write (a daemon-uid- or root-owned dir under
//! `/var/lib` in deployment; an `$XDG_STATE_HOME` path in dev, with
//! `ARLEN_CONFIG_BROKER_DIR` as the override seam the systemd unit
//! points at the real protected dir). Writes are atomic + durable
//! (sibling temp, 0600, fsync, rename, dir-fsync). Reads fail closed:
//! a missing file yields the conservative floor, a corrupt file is an
//! error the caller must refuse on - it must never silently widen
//! authority.

use std::collections::BTreeSet;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The clamped ceiling for `access_level` (mirrors the five read
/// tiers 0..=4). A value above this is treated as malformed and
/// clamped to the floor, never the ceiling - fail-closed.
pub const MAX_ACCESS_LEVEL: u8 = 4;

/// The agent's baseline action mode. Only the two user-settable
/// values; autonomy-per-app rides `autonomous_apps`, and
/// `executor_live` is the orthogonal master gate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionMode {
    /// Propose only; nothing acts without explicit confirmation.
    #[default]
    Suggest,
    /// Act on the reversible/low-risk majority; gate the rest.
    Supervised,
}

impl ActionMode {
    /// The wire/TOML string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionMode::Suggest => "suggest",
            ActionMode::Supervised => "supervised",
        }
    }
}

/// The security-load-bearing AI master switches. Every field is a
/// thing a same-uid process could silently flip in today's ambient
/// `ai.toml`; the broker is their sole writer.
///
/// `Default` is the conservative fail-closed FLOOR (off / minimal /
/// suggest / no autonomy), the state a missing or unreadable store
/// resolves to. The generous shipped defaults (e.g. `access_level`
/// 3 - "see recent activity") are SEEDED into the store at first run
/// by the migration step, not baked into this floor: a security
/// store must never default-open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiMasterSwitches {
    /// The global AI master switch.
    pub enabled: bool,
    /// Read scope tier 0..=4 (0 = Minimal, reads nothing).
    pub access_level: u8,
    /// The executor "human gate" - when false, nothing the agent
    /// proposes ever writes.
    pub executor_live: bool,
    /// The baseline action mode.
    pub action_mode: ActionMode,
    /// The active provider id (empty = the daemon's configured
    /// default/ranking decides).
    pub provider: String,
    /// Per-app autonomy grants (the apps allowed to act without the
    /// per-action prompt).
    pub autonomous_apps: BTreeSet<String>,
}

impl Default for AiMasterSwitches {
    fn default() -> Self {
        Self {
            enabled: false,
            access_level: 0,
            executor_live: false,
            action_mode: ActionMode::Suggest,
            provider: String::new(),
            autonomous_apps: BTreeSet::new(),
        }
    }
}

impl AiMasterSwitches {
    /// The generous shipped default the broker SEEDS into a fresh
    /// store - distinct from [`Default`], the fail-closed FLOOR a
    /// missing/corrupt store resolves to at read time. Matches the
    /// shipped `ai.toml` (`DEFAULT_AI` in the settings app): the AI
    /// ships disabled but, once enabled, sees recent activity
    /// (`access_level` 3) so it is useful out of the box; the
    /// security-sensitive switches (`executor_live`, autonomy) stay
    /// at the floor. The two must stay in step until the cutover
    /// makes the broker the sole owner of these defaults.
    pub fn shipped_default() -> Self {
        Self {
            enabled: false,
            access_level: 3,
            executor_live: false,
            action_mode: ActionMode::Suggest,
            provider: "ollama-default".to_string(),
            autonomous_apps: BTreeSet::new(),
        }
    }

    /// Clamp any structurally-invalid field to its fail-closed value.
    /// An `access_level` above the ceiling is malformed input, so it
    /// drops to 0 (minimal) - never to the ceiling, which would let a
    /// corrupt file or a buggy caller silently grant the widest scope.
    /// Applied on both load and store, so no out-of-range value is
    /// ever persisted or returned.
    pub fn sanitised(mut self) -> Self {
        if self.access_level > MAX_ACCESS_LEVEL {
            self.access_level = 0;
        }
        self
    }
}

/// The security-relevant switch keys that differ between `old` and `new`, each as
/// a short `key=new_value` summary for the audit trail (empty = no change). Every
/// field of [`AiMasterSwitches`] is security-relevant - the master switch, the read
/// scope, the executor "human gate", the baseline action mode, the provider
/// endpoint, and the per-app autonomy grants - so a change to any one is recorded
/// so a silent flip of the AI's authority posture becomes visible in the ledger.
pub fn changed_security_keys(old: &AiMasterSwitches, new: &AiMasterSwitches) -> Vec<String> {
    let mut changed = Vec::new();
    if old.enabled != new.enabled {
        changed.push(format!("enabled={}", new.enabled));
    }
    if old.access_level != new.access_level {
        changed.push(format!("access_level={}", new.access_level));
    }
    if old.executor_live != new.executor_live {
        changed.push(format!("executor_live={}", new.executor_live));
    }
    if old.action_mode != new.action_mode {
        changed.push(format!("action_mode={}", new.action_mode.as_str()));
    }
    if old.provider != new.provider {
        changed.push(format!("provider={}", new.provider));
    }
    if old.autonomous_apps != new.autonomous_apps {
        changed.push(format!("autonomous_apps={}", new.autonomous_apps.len()));
    }
    changed
}

/// True iff the transition `old` -> `new` ADDS authority in any
/// dimension - the dangerous direction that warrants the tamper-evident
/// trail and is gated fail-closed (the escalation is refused if it
/// cannot be recorded). A change that only REMOVES authority - the AI
/// turned off, the executor gate closed, the read scope narrowed,
/// autonomy revoked, the action mode dropped back to suggest, the
/// provider cleared to the configured default - is NOT escalating: it
/// rides the unconditional off-switch path (always applied, best-effort
/// audit), so an attacker who takes down the audit daemon can never trap
/// the AI in the ON / wide-open state (the removability invariant). A
/// provider REPOINT to a concrete endpoint counts as escalating: it
/// redirects where the AI's prompts and data egress, the dangerous
/// direction; only clearing it to empty (revert to the daemon's default)
/// is safe.
pub fn escalates(old: &AiMasterSwitches, new: &AiMasterSwitches) -> bool {
    (!old.enabled && new.enabled)
        || (!old.executor_live && new.executor_live)
        || (new.access_level > old.access_level)
        || (old.action_mode == ActionMode::Suggest && new.action_mode == ActionMode::Supervised)
        || new
            .autonomous_apps
            .difference(&old.autonomous_apps)
            .next()
            .is_some()
        || (new.provider != old.provider && !new.provider.is_empty())
}

/// The audit event for a change to the AI master switches: kind
/// [`AuditKind::CapabilityChange`], a content-free subject, and an outcome naming
/// the caller and which switches changed. The audit daemon sets the ACTOR from the
/// submitting peer (the broker), so the CALLER's app id is carried in the outcome
/// for the accountability trail.
pub fn switch_change_event(caller_app_id: &str, changed: &[String]) -> audit_proto::IngestRequest {
    audit_proto::IngestRequest {
        kind: audit_proto::AuditKind::CapabilityChange,
        structural: audit_proto::StructuralRecord {
            subject: "ai.master_switches".to_string(),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: format!("set by {caller_app_id}: {}", changed.join(", ")),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// A failure reading or writing the canonical state.
#[derive(Debug, Error)]
pub enum StateError {
    /// No state directory could be resolved (no override, no
    /// `XDG_STATE_HOME`, no `HOME`).
    #[error("no state directory (set ARLEN_CONFIG_BROKER_DIR, XDG_STATE_HOME or HOME)")]
    NoStateDir,
    /// A filesystem operation failed.
    #[error("state io: {0}")]
    Io(String),
    /// The state file exists but did not parse - the caller must
    /// refuse, not fall back to a guessed state.
    #[error("state file is corrupt: {0}")]
    Parse(String),
}

/// The state-file name inside the broker directory.
const STATE_FILE: &str = "state.toml";

/// Resolve the broker state directory: the `ARLEN_CONFIG_BROKER_DIR`
/// override (the seam the systemd unit points at the protected dir),
/// else `$XDG_STATE_HOME/arlen/config-broker`, else
/// `$HOME/.local/state/arlen/config-broker`.
pub fn state_dir() -> Result<PathBuf, StateError> {
    if let Some(dir) = std::env::var_os("ARLEN_CONFIG_BROKER_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(base) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(base).join("arlen").join("config-broker"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".local/state/arlen/config-broker"));
    }
    Err(StateError::NoStateDir)
}

/// The broker's durable store: a 0700 directory holding the 0600
/// `state.toml`.
#[derive(Debug, Clone)]
pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    /// Open the store at an explicit directory, creating it 0700 if
    /// absent.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StateError> {
        let dir = dir.into();
        ensure_private_dir(&dir)?;
        Ok(Self { dir })
    }

    /// Open the store at the resolved default directory.
    pub fn open_default() -> Result<Self, StateError> {
        Self::open(state_dir()?)
    }

    /// The state file path.
    fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }

    /// Load the canonical state. A missing file resolves to the
    /// conservative floor ([`AiMasterSwitches::default`]); a present
    /// file is parsed and sanitised (out-of-range fields fail closed);
    /// a present-but-unparseable file is an error the caller must
    /// refuse on.
    pub fn load(&self) -> Result<AiMasterSwitches, StateError> {
        let path = self.state_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AiMasterSwitches::default());
            }
            Err(e) => return Err(StateError::Io(e.to_string())),
        };
        let switches: AiMasterSwitches =
            toml::from_str(&text).map_err(|e| StateError::Parse(e.to_string()))?;
        Ok(switches.sanitised())
    }

    /// Persist the canonical state durably: write a 0600 sibling temp,
    /// fsync it, rename over `state.toml` (atomic), then fsync the
    /// directory so the rename survives a crash.
    pub fn store(&self, switches: &AiMasterSwitches) -> Result<(), StateError> {
        // Clamp before persisting so an out-of-range field never
        // reaches disk, regardless of caller.
        let switches = switches.clone().sanitised();
        let text = toml::to_string_pretty(&switches)
            .map_err(|e| StateError::Io(format!("serialize: {e}")))?;
        let path = self.state_path();
        let tmp = self.dir.join(format!(".{STATE_FILE}.tmp"));
        write_atomic_0600(&tmp, &path, text.as_bytes())?;
        Ok(())
    }

    /// Seed the canonical state with `seed` ONLY if no state file
    /// exists yet (a fresh broker). An existing store - even one a
    /// user narrowed to the floor - is left untouched, so a restart
    /// never clobbers a deliberate setting. Returns whether it
    /// seeded. The broker is the single writer at startup (before it
    /// accepts connections), so the check-then-write is race-free.
    pub fn seed_if_absent(&self, seed: &AiMasterSwitches) -> Result<bool, StateError> {
        if self.state_path().exists() {
            return Ok(false);
        }
        self.store(seed)?;
        Ok(true)
    }

    /// The store directory (for the socket/lock siblings a later
    /// slice adds).
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Create `dir` (and parents) mode 0700, idempotently.
fn ensure_private_dir(dir: &Path) -> Result<(), StateError> {
    if dir.is_dir() {
        // Tighten an existing dir to 0700 (a prior looser creation or
        // a deploy default must not leave it group/other-accessible).
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| StateError::Io(e.to_string()))?;
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| StateError::Io(e.to_string()))
}

/// Write `bytes` to `target` 0600 via a sibling temp + fsync + rename
/// + dir-fsync (atomic, crash-durable).
fn write_atomic_0600(tmp: &Path, target: &Path, bytes: &[u8]) -> Result<(), StateError> {
    use std::io::Write;
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(tmp)
            .map_err(|e| StateError::Io(e.to_string()))?;
        f.write_all(bytes).map_err(|e| StateError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| StateError::Io(e.to_string()))?;
    }
    std::fs::rename(tmp, target).map_err(|e| StateError::Io(e.to_string()))?;
    // Fsync the directory so the rename itself is durable.
    if let Some(parent) = target.parent() {
        if let Ok(d) = std::fs::File::open(parent) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &Path) -> StateStore {
        StateStore::open(dir).expect("open")
    }

    #[test]
    fn changed_security_keys_reports_each_flipped_field() {
        let base = AiMasterSwitches::default();
        // No change -> nothing to audit.
        assert!(changed_security_keys(&base, &base).is_empty());
        // Flipping the executor gate + widening the read scope is recorded, with
        // the new values (the point: a silent authority flip becomes visible).
        let mut new = base.clone();
        new.executor_live = true;
        new.access_level = 4;
        let changed = changed_security_keys(&base, &new);
        assert_eq!(changed.len(), 2);
        assert!(changed.iter().any(|c| c == "executor_live=true"));
        assert!(changed.iter().any(|c| c == "access_level=4"));
        // A repointed provider and a new autonomy grant are each recorded too.
        let mut new2 = base.clone();
        new2.provider = "http://evil.example".to_string();
        new2.autonomous_apps.insert("com.foo".to_string());
        let changed2 = changed_security_keys(&base, &new2);
        assert!(changed2.iter().any(|c| c.starts_with("provider=")));
        assert!(changed2.iter().any(|c| c.starts_with("autonomous_apps=")));
    }

    #[test]
    fn escalates_flags_only_authority_adding_changes() {
        let floor = AiMasterSwitches::default();
        // no change -> not escalating
        assert!(!escalates(&floor, &floor));
        // turning the AI on / opening the executor gate -> escalating
        assert!(escalates(&floor, &AiMasterSwitches { enabled: true, ..floor.clone() }));
        assert!(escalates(&floor, &AiMasterSwitches { executor_live: true, ..floor.clone() }));
        // widening the read scope escalates; narrowing it does not
        let lvl3 = AiMasterSwitches { access_level: 3, ..floor.clone() };
        assert!(escalates(&floor, &lvl3));
        assert!(!escalates(&lvl3, &floor));
        // suggest -> supervised escalates; the reverse does not
        let sup = AiMasterSwitches { action_mode: ActionMode::Supervised, ..floor.clone() };
        assert!(escalates(&floor, &sup));
        assert!(!escalates(&sup, &floor));
        // granting autonomy escalates; revoking it does not
        let mut grant = floor.clone();
        grant.autonomous_apps.insert("com.foo".to_string());
        assert!(escalates(&floor, &grant));
        assert!(!escalates(&grant, &floor));
        // a provider repoint to a concrete endpoint escalates; clearing it does not
        let p1 = AiMasterSwitches { provider: "ollama-default".to_string(), ..floor.clone() };
        let p2 = AiMasterSwitches { provider: "http://evil".to_string(), ..floor.clone() };
        assert!(escalates(&p1, &p2));
        assert!(!escalates(&p2, &AiMasterSwitches { provider: String::new(), ..floor.clone() }));
        // a whole-state drop from fully-open to the floor is the pure
        // off-switch: never escalating, even bundled.
        let open = AiMasterSwitches {
            enabled: true,
            access_level: 4,
            executor_live: true,
            action_mode: ActionMode::Supervised,
            provider: "x".to_string(),
            autonomous_apps: BTreeSet::from(["a".to_string()]),
        };
        assert!(!escalates(&open, &floor));
    }

    #[test]
    fn switch_change_event_records_the_caller_and_the_change() {
        let ev =
            switch_change_event("com.example.settings", &["executor_live=true".to_string()]);
        assert!(matches!(ev.kind, audit_proto::AuditKind::CapabilityChange));
        assert_eq!(ev.structural.subject, "ai.master_switches");
        // The caller id + the changed switch are in the outcome (the actor is the
        // submitting broker, so the caller is carried here for the trail).
        assert!(ev.structural.outcome.contains("com.example.settings"));
        assert!(ev.structural.outcome.contains("executor_live=true"));
    }

    #[test]
    fn missing_file_loads_the_conservative_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let got = s.load().unwrap();
        assert_eq!(got, AiMasterSwitches::default());
        assert!(!got.enabled);
        assert_eq!(got.access_level, 0);
        assert!(!got.executor_live);
        assert_eq!(got.action_mode, ActionMode::Suggest);
        assert!(got.autonomous_apps.is_empty());
    }

    #[test]
    fn round_trips_a_full_state() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let mut want = AiMasterSwitches {
            enabled: true,
            access_level: 3,
            executor_live: true,
            action_mode: ActionMode::Supervised,
            provider: "ollama-default".to_string(),
            autonomous_apps: BTreeSet::new(),
        };
        want.autonomous_apps.insert("org.arlen.files".to_string());
        s.store(&want).unwrap();
        assert_eq!(s.load().unwrap(), want);
    }

    #[test]
    fn a_corrupt_file_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        std::fs::write(tmp.path().join(STATE_FILE), "this = is = not = toml").unwrap();
        match s.load() {
            Err(StateError::Parse(_)) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_out_of_range_access_level_clamps_to_the_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        std::fs::write(tmp.path().join(STATE_FILE), "access_level = 9\nenabled = true\n").unwrap();
        let got = s.load().unwrap();
        assert_eq!(got.access_level, 0, "9 > MAX clamps to the safe floor, not the ceiling");
        assert!(got.enabled, "the valid field is preserved");
    }

    #[test]
    fn a_partial_file_fills_missing_fields_from_the_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        std::fs::write(tmp.path().join(STATE_FILE), "enabled = true\naccess_level = 2\n").unwrap();
        let got = s.load().unwrap();
        assert!(got.enabled);
        assert_eq!(got.access_level, 2);
        // unmentioned security fields stay at the floor
        assert!(!got.executor_live);
        assert_eq!(got.action_mode, ActionMode::Suggest);
    }

    #[test]
    fn the_dir_is_0700_and_the_file_is_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        s.store(&AiMasterSwitches::default()).unwrap();
        let dir_mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(tmp.path().join(STATE_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "broker dir must be owner-only");
        assert_eq!(file_mode, 0o600, "state file must be owner-only");
    }

    #[test]
    fn shipped_default_is_useful_but_not_open() {
        let d = AiMasterSwitches::shipped_default();
        // useful: recent-activity read scope once enabled
        assert_eq!(d.access_level, 3);
        assert_eq!(d.provider, "ollama-default");
        // but ships off + never auto-acting
        assert!(!d.enabled);
        assert!(!d.executor_live);
        assert_eq!(d.action_mode, ActionMode::Suggest);
        assert!(d.autonomous_apps.is_empty());
    }

    #[test]
    fn seed_writes_a_fresh_store_then_never_clobbers() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        // fresh: seeds
        assert!(s.seed_if_absent(&AiMasterSwitches::shipped_default()).unwrap());
        assert_eq!(s.load().unwrap(), AiMasterSwitches::shipped_default());
        // a user narrows to the floor
        s.store(&AiMasterSwitches::default()).unwrap();
        // a later seed (e.g. a restart) does NOT overwrite the narrowing
        assert!(!s.seed_if_absent(&AiMasterSwitches::shipped_default()).unwrap());
        assert_eq!(s.load().unwrap(), AiMasterSwitches::default());
    }

    #[test]
    fn action_mode_round_trips_through_toml() {
        assert_eq!(ActionMode::Suggest.as_str(), "suggest");
        assert_eq!(ActionMode::Supervised.as_str(), "supervised");
        let t = toml::to_string(&AiMasterSwitches {
            action_mode: ActionMode::Supervised,
            ..Default::default()
        })
        .unwrap();
        assert!(t.contains("action_mode = \"supervised\""));
    }
}
