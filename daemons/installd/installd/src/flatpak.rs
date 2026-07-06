/// Flatpak integration for the install daemon.
///
/// Installs, uninstalls, and lists Flatpak applications via the `flatpak`
/// CLI. After installation, a default Arlen permission profile is created
/// so the app participates in the Knowledge Graph and Event Bus permission
/// system alongside native .lunpkg apps.

use std::process::Command;

use thiserror::Error;

/// Errors from Flatpak operations.
#[derive(Debug, Error)]
pub enum FlatpakError {
    #[error("flatpak command not found")]
    NotFound,
    #[error("flatpak install failed: {0}")]
    InstallFailed(String),
    #[error("flatpak uninstall failed: {0}")]
    UninstallFailed(String),
    #[error("flatpak info failed: {0}")]
    InfoFailed(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

/// Metadata about an installed Flatpak app.
#[derive(Debug, Clone)]
pub struct FlatpakInfo {
    pub app_id: String,
    pub name: String,
    pub version: String,
}

/// Install a Flatpak app for the current user.
///
/// Uses `flatpak install --user --noninteractive`. The `remote` defaults
/// to "flathub" if empty.
pub fn install_flatpak(app_id: &str, remote: &str) -> Result<(), FlatpakError> {
    check_flatpak_available()?;

    let remote = if remote.is_empty() { "flathub" } else { remote };

    let output = Command::new("flatpak")
        .args(["install", "--user", "--noninteractive", remote, app_id])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(FlatpakError::InstallFailed(stderr));
    }

    tracing::info!("flatpak: installed {app_id} from {remote}");
    Ok(())
}

/// Uninstall a Flatpak app for the current user.
pub fn uninstall_flatpak(app_id: &str) -> Result<(), FlatpakError> {
    check_flatpak_available()?;

    let output = Command::new("flatpak")
        .args(["uninstall", "--user", "--noninteractive", app_id])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(FlatpakError::UninstallFailed(stderr));
    }

    tracing::info!("flatpak: uninstalled {app_id}");
    Ok(())
}

/// Get metadata for an installed Flatpak app.
pub fn get_flatpak_info(app_id: &str) -> Result<FlatpakInfo, FlatpakError> {
    let output = Command::new("flatpak")
        .args(["info", "--user", app_id])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(FlatpakError::InfoFailed(stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut name = String::new();
    let mut version = String::new();

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("Name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("Version:") {
            version = val.trim().to_string();
        }
    }

    Ok(FlatpakInfo {
        app_id: app_id.to_string(),
        name,
        version,
    })
}

/// List all user-installed Flatpak applications.
///
/// Returns `Vec<(app_id, name, version, "flatpak")>`.
pub fn list_installed_flatpaks() -> Vec<(String, String, String, String)> {
    let output = match Command::new("flatpak")
        .args([
            "list",
            "--user",
            "--app",
            "--columns=application,name,version",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut apps = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        let app_id = parts.first().unwrap_or(&"").to_string();
        let name = parts.get(1).unwrap_or(&"").to_string();
        let version = parts.get(2).unwrap_or(&"").to_string();

        if !app_id.is_empty() {
            apps.push((app_id, name, version, "flatpak".into()));
        }
    }

    apps
}

/// Whether `app_id` is a syntactically valid flatpak application id: a non-empty,
/// dot-separated reverse-DNS name over `[A-Za-z0-9._-]` (flatpak permits uppercase,
/// e.g. `org.kde.Kdenlive`) with no `..` and no leading/trailing dot. The id is
/// interpolated into the generated profile TOML and joined into the profile path,
/// so this rejects the metacharacters (`"`, `]`, newline) that would inject grants
/// and the separators (`/`, `..`) that would escape `~/.config/permissions/`.
pub fn is_valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && !app_id.starts_with('.')
        && !app_id.ends_with('.')
        && !app_id.contains("..")
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Generate a default Arlen permission profile TOML for a Flatpak app.
///
/// Flatpak apps get a conservative default profile. The actual sandbox
/// enforcement comes from Flatpak itself; this profile controls
/// Knowledge Graph and Event Bus access. The caller MUST validate `app_id` with
/// [`is_valid_app_id`] first; the id is interpolated into the TOML unescaped.
pub fn default_permission_profile(app_id: &str) -> String {
    format!(
        r#"[info]
app_id = "{app_id}"
tier = "third-party"

[graph]
read = ["{app_id}.*"]
write = ["{app_id}.*"]

[event_bus]
subscribe = ["system.theme.*"]
publish = ["{app_id}.*"]

[filesystem]
documents = false
downloads = false

[network]
domains = []

[capabilities]
notifications = true
clipboard = false
autostart = false
background = false
"#
    )
}

/// The permission `[Context]` a Flatpak app declares (its finish-args as
/// installed), read from `flatpak info --show-permissions`. Arlen generates the
/// FLOOR profile from this - grant exactly the dimensions Flatpak already grants
/// the app, never more (the Flatpak manifest is the floor, app-enrollment §E5).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FlatpakContext {
    /// `filesystems=` entries, e.g. `home`, `xdg-download`, `host`.
    pub filesystems: Vec<String>,
    /// `shared=` entries, e.g. `network`, `ipc`.
    pub shared: Vec<String>,
    /// `sockets=` entries, e.g. `wayland`, `pulseaudio` (no profile dimension).
    pub sockets: Vec<String>,
    /// `devices=` entries, e.g. `dri`, `all` (no profile dimension).
    pub devices: Vec<String>,
}

/// Parse `flatpak info --show-permissions <app>` output - an INI-like `[Context]`
/// section whose values are `;`-separated lists - into a [`FlatpakContext`]. Only
/// the `[Context]` section is read; other sections are ignored.
pub fn parse_show_permissions(output: &str) -> FlatpakContext {
    let mut ctx = FlatpakContext::default();
    let mut in_context = false;
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_context = line.eq_ignore_ascii_case("[Context]");
            continue;
        }
        if !in_context {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let items: Vec<String> = val
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        match key.trim() {
            "filesystems" => ctx.filesystems = items,
            "shared" => ctx.shared = items,
            "sockets" => ctx.sockets = items,
            "devices" => ctx.devices = items,
            _ => {}
        }
    }
    ctx
}

/// Map a Flatpak `filesystems=` token to an Arlen filesystem dimension, or `None`
/// if it has no matching dimension (a raw host path, a subdir the profile does
/// not model). `host`/`host-os` grant broad access; the conservative floor maps
/// them to the user's home only, never more than the profile can express.
fn flatpak_fs_dimension(token: &str) -> Option<&'static str> {
    // Flatpak filesystem tokens may carry an access suffix (`home:ro`); the
    // dimension is keyed on the path token before it.
    let base = token.split(':').next().unwrap_or(token).trim_end_matches('/');
    match base {
        "home" | "host" | "host-os" => Some("home"),
        "xdg-documents" => Some("documents"),
        "xdg-download" | "xdg-downloads" => Some("downloads"),
        "xdg-pictures" => Some("pictures"),
        "xdg-music" => Some("music"),
        "xdg-videos" => Some("videos"),
        _ => None,
    }
}

/// Read an installed Flatpak app's declared permission `[Context]` via
/// `flatpak info --show-permissions`, for generating the floor profile. Errors if
/// the app is not installed or Flatpak is unavailable; the caller falls back to
/// the conservative [`default_permission_profile`].
pub fn get_flatpak_context(app_id: &str) -> Result<FlatpakContext, FlatpakError> {
    let output = Command::new("flatpak")
        .args(["info", "--user", "--show-permissions", app_id])
        .output()?;
    if !output.status.success() {
        return Err(FlatpakError::InfoFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(parse_show_permissions(&String::from_utf8_lossy(&output.stdout)))
}

/// Generate an Arlen permission-profile TOML FLOOR from a Flatpak `[Context]`:
/// grant exactly what Flatpak already grants, never more. Filesystem tokens map to
/// the matching XDG dimension (`home`/`host` conservatively to `home`);
/// `shared=network` grants network; sockets and devices have no profile dimension
/// (the app reaches display/audio/devices through Flatpak's own portals, not an
/// Arlen fs/net/graph grant), so they are not granted. Graph access stays the
/// conservative own-namespace default (Flatpak declares no graph reach). `app_id`
/// must be validated by [`is_valid_app_id`] before this is called - it is
/// interpolated into the TOML and the profile path.
pub fn floor_profile_from_context(ctx: &FlatpakContext, app_id: &str) -> String {
    let mut dims: Vec<&'static str> = ctx
        .filesystems
        .iter()
        .filter_map(|f| flatpak_fs_dimension(f))
        .collect();
    dims.sort_unstable();
    dims.dedup();
    let fs_lines: String = ["home", "documents", "downloads", "pictures", "music", "videos"]
        .iter()
        .filter(|d| dims.contains(d))
        .map(|d| format!("{d} = true\n"))
        .collect();
    let network = ctx.shared.iter().any(|s| s == "network");
    let network_section = if network {
        "[network]\nallow_all = true\n"
    } else {
        "[network]\ndomains = []\n"
    };
    format!(
        "[info]\napp_id = \"{app_id}\"\ntier = \"third-party\"\n\n\
         [graph]\nread = [\"{app_id}.*\"]\nwrite = [\"{app_id}.*\"]\n\n\
         [filesystem]\n{fs_lines}\n\
         {network_section}\n\
         [capabilities]\nnotifications = true\nclipboard = false\n"
    )
}

/// Check that flatpak CLI is available.
fn check_flatpak_available() -> Result<(), FlatpakError> {
    match Command::new("flatpak").arg("--version").output() {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(FlatpakError::NotFound),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_show_permissions_reads_the_context_section() {
        let out = "[Application]\nname=org.x.App\n\n\
                   [Context]\n\
                   shared=network;ipc;\n\
                   sockets=x11;wayland;pulseaudio;\n\
                   devices=dri;\n\
                   filesystems=home;xdg-download;\n";
        let ctx = parse_show_permissions(out);
        assert_eq!(ctx.shared, vec!["network", "ipc"]);
        assert_eq!(ctx.filesystems, vec!["home", "xdg-download"]);
        assert_eq!(ctx.sockets, vec!["x11", "wayland", "pulseaudio"]);
        assert_eq!(ctx.devices, vec!["dri"]);
    }

    #[test]
    fn floor_profile_grants_exactly_the_declared_reach_and_parses() {
        let ctx = FlatpakContext {
            filesystems: vec!["home".into(), "xdg-download".into(), "host".into()],
            shared: vec!["network".into()],
            sockets: vec!["wayland".into()],
            devices: vec!["dri".into()],
        };
        let toml = floor_profile_from_context(&ctx, "org.x.App");
        // The floor is a valid canonical profile.
        let profile: arlen_permissions::PermissionProfile = toml::from_str(&toml).unwrap();
        // home (home + host both map to it) and downloads granted; the others not.
        assert!(profile.filesystem.home);
        assert!(profile.filesystem.downloads);
        assert!(!profile.filesystem.documents);
        assert!(!profile.filesystem.pictures);
        // shared=network -> network; sockets/devices grant nothing.
        assert!(profile.network.allow_all);
    }

    #[test]
    fn floor_profile_without_network_grants_none() {
        let ctx = FlatpakContext {
            filesystems: vec!["xdg-music".into()],
            ..Default::default()
        };
        let profile: arlen_permissions::PermissionProfile =
            toml::from_str(&floor_profile_from_context(&ctx, "org.y.App")).unwrap();
        assert!(profile.filesystem.music);
        assert!(!profile.filesystem.home);
        assert!(!profile.network.allow_all);
    }

    #[test]
    fn test_default_permission_profile() {
        let profile = default_permission_profile("org.gnome.Calculator");
        assert!(profile.contains("app_id = \"org.gnome.Calculator\""));
        assert!(profile.contains("tier = \"third-party\""));
        assert!(profile.contains("org.gnome.Calculator.*"));

        // Validate it parses as TOML.
        let parsed: toml::Value = toml::from_str(&profile).unwrap();
        assert!(parsed.get("info").is_some());
        assert!(parsed.get("graph").is_some());
    }

    #[test]
    fn test_is_valid_app_id() {
        assert!(is_valid_app_id("org.gnome.Calculator"));
        assert!(is_valid_app_id("org.kde.Kdenlive"));
        assert!(is_valid_app_id("com.obsproject.Studio"));
        // Injection + traversal forms the format!/path build must never see.
        for bad in [
            "",
            "x\"]\nwrite = [\"system.*\"]\n#",
            "../../evil",
            "a/b",
            ".hidden",
            "trail.",
            "with space",
        ] {
            assert!(!is_valid_app_id(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn test_list_installed_flatpaks_no_flatpak() {
        // If flatpak is not installed or has no user apps, returns empty.
        // This test validates the graceful fallback.
        let apps = list_installed_flatpaks();
        // We can't assert the exact count (depends on system), but it
        // should not panic.
        assert!(apps.iter().all(|(_, _, _, src)| src == "flatpak"));
    }

    #[test]
    fn test_parse_flatpak_list_output() {
        // Simulate parsing the tab-separated output format.
        let line = "org.gnome.Calculator\tCalculator\t46.1";
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(parts[0], "org.gnome.Calculator");
        assert_eq!(parts[1], "Calculator");
        assert_eq!(parts[2], "46.1");
    }
}
