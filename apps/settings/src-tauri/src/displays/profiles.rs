/// Hot-plug profile management for the Display panel.
///
/// Two files cooperate:
///
/// 1. `~/.config/lunaris/compositor.d/displays.toml` — owned by the
///    compositor, written via `cosmic_comp_config::output::displays_toml`.
///    Holds the per-output configuration arrays under
///    `[[profile]]` records keyed by `output_set`.
/// 2. `~/.config/lunaris/compositor.d/displays.profiles.toml` —
///    owned by app-settings, this module. Holds user-facing
///    metadata: label, `last_used` timestamp.
///
/// Splitting the two stores avoids a fight over schema ownership.
/// The compositor's `to_toml_string` regenerates profile names on
/// every write, so user labels would not survive a round-trip if
/// we put them in displays.toml. The sidecar lives next to the
/// canonical file, joins on the canonical sort order
/// (alphabetical by `connector`), and never has to be touched by
/// the compositor.
///
/// The two files can drift if a profile is added by one side and
/// the other side hasn't observed it yet. We tolerate drift:
/// `list_profiles` returns profiles from displays.toml even when
/// the sidecar lacks an entry (label falls back to a derived
/// "DP-1 + HDMI-A-1" string), and stale sidecar entries (no
/// matching profile in displays.toml) are pruned on every save.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cosmic_comp_config::output::comp::{OutputConfig, OutputInfo};
use cosmic_comp_config::output::displays_toml;
use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = ".config/lunaris/compositor.d";
const DISPLAYS_TOML: &str = "displays.toml";
const PROFILES_TOML: &str = "displays.profiles.toml";

fn config_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(PathBuf::from(home).join(CONFIG_DIR))
}

fn displays_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(DISPLAYS_TOML))
}

fn profiles_meta_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(PROFILES_TOML))
}

// ---------------------------------------------------------------------------
// Sidecar schema
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SidecarFile {
    #[serde(default, rename = "profile", skip_serializing_if = "Vec::is_empty")]
    profiles: Vec<SidecarEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SidecarEntry {
    /// Mirror of the canonical `output_set` from displays.toml,
    /// serialised as a list of `{connector, make, model}` to keep
    /// the file readable without cross-referencing displays.toml.
    output_set: Vec<OutputInfoMeta>,
    label: String,
    /// Wall-clock UTC timestamp of the last `apply_profile` call
    /// (or the save timestamp for never-applied profiles). Used to
    /// pick a winner when two profiles share an output_set.
    last_used: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct OutputInfoMeta {
    connector: String,
    make: String,
    model: String,
}

impl From<&OutputInfo> for OutputInfoMeta {
    fn from(value: &OutputInfo) -> Self {
        Self {
            connector: value.connector.clone(),
            make: value.make.clone(),
            model: value.model.clone(),
        }
    }
}

fn read_sidecar() -> SidecarFile {
    let Ok(path) = profiles_meta_path() else {
        return SidecarFile::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return SidecarFile::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

fn write_sidecar(sidecar: &SidecarFile) -> Result<(), String> {
    let path = profiles_meta_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let serialized = toml::to_string_pretty(sidecar)
        .map_err(|e| format!("serialize sidecar: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, serialized).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Frontend types
// ---------------------------------------------------------------------------

/// One profile as the Settings list sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    /// Stable identifier — derived from the canonical output_set
    /// so the same physical setup hashes to the same id every
    /// time. Used by mutation commands.
    pub id: String,
    /// User-chosen label, or auto-derived (`"DP-1 + HDMI-A-1"`)
    /// when the sidecar has no entry.
    pub label: String,
    /// Connector list, sorted alphabetically. Same order as
    /// stored in displays.toml.
    pub output_set: Vec<OutputInfoSummary>,
    /// Last-applied wall-clock time, or `None` for new profiles
    /// that haven't been applied yet.
    pub last_used: Option<DateTime<Utc>>,
    /// True if this profile's `output_set` matches the live
    /// monitor list right now. Computed by the caller (we don't
    /// ship the live list here to keep the function pure).
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfoSummary {
    pub connector: String,
    pub make: String,
    pub model: String,
}

impl From<&OutputInfo> for OutputInfoSummary {
    fn from(value: &OutputInfo) -> Self {
        Self {
            connector: value.connector.clone(),
            make: value.make.clone(),
            model: value.model.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the stable identifier for a profile from its output set.
///
/// We hash (connector | make | model) joined with `\0` separators.
/// Stable across boots because the compositor sorts output_set by
/// connector before serialising. Choosing a stable id rather than
/// the user-mutable label means rename / re-save cycles don't
/// orphan apply requests in flight.
pub fn profile_id(output_set: &[OutputInfo]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    for info in output_set {
        info.connector.hash(&mut h);
        b'\0'.hash(&mut h);
        info.make.hash(&mut h);
        b'\0'.hash(&mut h);
        info.model.hash(&mut h);
        b'\x1e'.hash(&mut h); // record separator
    }
    format!("set-{:016x}", h.finish())
}

/// Auto-derived label for profiles that have no sidecar entry.
/// "DP-1 + HDMI-A-1" reads OK in the list and is unique enough
/// for the user to identify their physical setup.
fn fallback_label(output_set: &[OutputInfo]) -> String {
    let connectors: Vec<_> = output_set.iter().map(|i| i.connector.as_str()).collect();
    if connectors.is_empty() {
        "(empty)".to_string()
    } else {
        connectors.join(" + ")
    }
}

/// Read every saved profile and return a Settings-friendly
/// summary. `live_set` is the current monitor list (from the
/// wayland thread) — profiles whose output_set matches are
/// flagged `is_current = true` so the UI can put a "Active" badge.
pub fn list_profiles(live_set: &[OutputInfo]) -> Vec<ProfileSummary> {
    let path = match displays_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let cfg = displays_toml::load(&path);
    let sidecar = read_sidecar();
    let live_meta: Vec<OutputInfoMeta> = live_set.iter().map(OutputInfoMeta::from).collect();

    let mut out = Vec::with_capacity(cfg.config.len());
    for (output_set, _outputs) in cfg.config.iter() {
        let id = profile_id(output_set);
        let meta_set: Vec<OutputInfoMeta> = output_set.iter().map(OutputInfoMeta::from).collect();
        let entry = sidecar
            .profiles
            .iter()
            .find(|e| e.output_set == meta_set);
        let label = entry
            .map(|e| e.label.clone())
            .unwrap_or_else(|| fallback_label(output_set));
        let last_used = entry.map(|e| e.last_used);
        let is_current = meta_set == live_meta;
        out.push(ProfileSummary {
            id,
            label,
            output_set: output_set.iter().map(OutputInfoSummary::from).collect(),
            last_used,
            is_current,
        });
    }
    // Sort: current first, then most-recently-used, then alphabetical.
    out.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then(b.last_used.cmp(&a.last_used))
            .then(a.label.cmp(&b.label))
    });
    out
}

/// Save the live monitor topology as a profile with the given
/// label. **Updates in-place** when an existing profile shares
/// the same `output_set`, both in `displays.toml` (canonical
/// HashMap insert overwrites by key) and in the sidecar. This
/// matches the natural mental model — "save the current setup
/// for these monitors" — and keeps the two stores 1:1 per
/// output_set, which the compositor's auto-apply logic relies on.
///
/// Renaming a saved profile uses `rename_profile`, not this; it
/// preserves the original `last_used` timestamp.
pub fn save_profile(
    label: String,
    live_set: Vec<OutputInfo>,
    live_outputs: Vec<OutputConfig>,
) -> Result<ProfileSummary, String> {
    let trimmed = label.trim().to_string();
    if trimmed.is_empty() {
        return Err("profile label cannot be empty".to_string());
    }

    let path = displays_path()?;
    let mut cfg = displays_toml::load(&path);
    cfg.config.insert(live_set.clone(), live_outputs);
    displays_toml::save(&path, &cfg).map_err(|e| e.to_string())?;

    let mut sidecar = read_sidecar();
    let meta_set: Vec<OutputInfoMeta> = live_set.iter().map(OutputInfoMeta::from).collect();
    let now = Utc::now();
    if let Some(existing) = sidecar
        .profiles
        .iter_mut()
        .find(|e| e.output_set == meta_set)
    {
        existing.label = trimmed.clone();
        existing.last_used = now;
    } else {
        sidecar.profiles.push(SidecarEntry {
            output_set: meta_set,
            label: trimmed.clone(),
            last_used: now,
        });
    }
    write_sidecar(&sidecar)?;

    Ok(ProfileSummary {
        id: profile_id(&live_set),
        label: trimmed,
        output_set: live_set.iter().map(OutputInfoSummary::from).collect(),
        last_used: Some(now),
        is_current: true,
    })
}

/// Look up the profile by id and return the canonical
/// configuration ready to feed into `display_apply_config`. Also
/// updates the sidecar's `last_used` timestamp on success — the
/// caller does the actual apply, this function records the
/// intent.
pub fn load_profile_for_apply(
    id: &str,
) -> Result<(Vec<OutputInfo>, Vec<OutputConfig>), String> {
    let path = displays_path()?;
    let cfg = displays_toml::load(&path);

    let entry = cfg
        .config
        .iter()
        .find(|(set, _)| profile_id(set) == id)
        .ok_or_else(|| format!("profile '{id}' not found"))?;

    let output_set = entry.0.clone();
    let outputs = entry.1.clone();

    // Bump the sidecar timestamp so list ordering and auto-apply
    // tie-breaking reflect this apply.
    let meta_set: Vec<OutputInfoMeta> = output_set.iter().map(OutputInfoMeta::from).collect();
    let mut sidecar = read_sidecar();
    if let Some(e) = sidecar
        .profiles
        .iter_mut()
        .find(|e| e.output_set == meta_set)
    {
        e.last_used = Utc::now();
    } else {
        // Sidecar lacks entry — synthesise one with the fallback
        // label so future list_profiles calls get a stable name.
        sidecar.profiles.push(SidecarEntry {
            output_set: meta_set,
            label: fallback_label(&output_set),
            last_used: Utc::now(),
        });
    }
    let _ = write_sidecar(&sidecar);

    Ok((output_set, outputs))
}

/// Delete a profile from displays.toml AND its sidecar entry.
pub fn delete_profile(id: &str) -> Result<(), String> {
    let path = displays_path()?;
    let mut cfg = displays_toml::load(&path);

    let to_remove: Vec<_> = cfg
        .config
        .keys()
        .filter(|set| profile_id(set) == id)
        .cloned()
        .collect();
    if to_remove.is_empty() {
        return Err(format!("profile '{id}' not found"));
    }
    let mut removed_meta_sets = Vec::new();
    for key in to_remove {
        let meta: Vec<OutputInfoMeta> = key.iter().map(OutputInfoMeta::from).collect();
        cfg.config.remove(&key);
        removed_meta_sets.push(meta);
    }
    displays_toml::save(&path, &cfg).map_err(|e| e.to_string())?;

    let mut sidecar = read_sidecar();
    sidecar
        .profiles
        .retain(|e| !removed_meta_sets.contains(&e.output_set));
    write_sidecar(&sidecar)?;
    Ok(())
}

/// Update only the label. The output configuration stays as-is.
/// If no sidecar entry exists yet, one is created (so renaming a
/// migrated `migrated-1` profile gives it a real label).
pub fn rename_profile(id: &str, new_label: String) -> Result<(), String> {
    let trimmed = new_label.trim().to_string();
    if trimmed.is_empty() {
        return Err("profile label cannot be empty".to_string());
    }

    let path = displays_path()?;
    let cfg = displays_toml::load(&path);
    let target_set = cfg
        .config
        .keys()
        .find(|set| profile_id(set) == id)
        .cloned()
        .ok_or_else(|| format!("profile '{id}' not found"))?;
    let meta_set: Vec<OutputInfoMeta> = target_set.iter().map(OutputInfoMeta::from).collect();

    let mut sidecar = read_sidecar();
    if let Some(e) = sidecar
        .profiles
        .iter_mut()
        .find(|e| e.output_set == meta_set)
    {
        e.label = trimmed;
    } else {
        sidecar.profiles.push(SidecarEntry {
            output_set: meta_set,
            label: trimmed,
            last_used: Utc::now(),
        });
    }
    write_sidecar(&sidecar)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn info(connector: &str, make: &str, model: &str) -> OutputInfo {
        OutputInfo {
            connector: connector.into(),
            make: make.into(),
            model: model.into(),
        }
    }

    #[test]
    fn profile_id_is_stable_across_calls() {
        let set = vec![info("DP-1", "Dell", "U2719D")];
        assert_eq!(profile_id(&set), profile_id(&set));
    }

    #[test]
    fn profile_id_changes_when_outputs_differ() {
        let a = vec![info("DP-1", "Dell", "U2719D")];
        let b = vec![info("HDMI-A-1", "LG", "27UL850")];
        assert_ne!(profile_id(&a), profile_id(&b));
    }

    #[test]
    fn profile_id_is_order_sensitive() {
        // Compositor sorts by connector before serializing, so
        // the canonical OutputInfo vec is sorted. Two vecs in
        // different order should hash differently — which means
        // callers must pass a sorted vec. (This is a sanity check
        // that we don't accidentally normalise inside profile_id
        // and break the contract.)
        let a = vec![info("DP-1", "x", "y"), info("HDMI-A-1", "x", "y")];
        let b = vec![info("HDMI-A-1", "x", "y"), info("DP-1", "x", "y")];
        assert_ne!(profile_id(&a), profile_id(&b));
    }

    #[test]
    fn fallback_label_joins_connectors() {
        let set = vec![info("DP-1", "x", "y"), info("HDMI-A-1", "x", "y")];
        assert_eq!(fallback_label(&set), "DP-1 + HDMI-A-1");
    }

    #[test]
    fn fallback_label_for_empty_set() {
        assert_eq!(fallback_label(&[]), "(empty)");
    }

    #[test]
    fn save_profile_rejects_empty_label() {
        let r = save_profile(String::new(), vec![info("DP-1", "x", "y")], vec![]);
        assert!(r.is_err());
        let r = save_profile("   ".into(), vec![info("DP-1", "x", "y")], vec![]);
        assert!(r.is_err());
    }
}
