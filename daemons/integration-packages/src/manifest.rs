//! The Integration Package manifest (integration-packages-plan.md, "The
//! Integration Package manifest" + IP-R5).
//!
//! The outer manifest of a community integration package: which app it targets,
//! which packages it conflicts with, and which optional components it bundles (a
//! permission profile, a Settings adapter). This is the FORMAT + CONFLICT layer:
//! parse + validate the manifest, decide compatibility against an installed app
//! version, and apply the conflict rule - conflicts are a user choice at install
//! and are never silently merged; when two conflicting packages are both
//! installed, the first-installed one's permissions stay active until the user
//! resolves it. Loading the bundled permission profile and writing it is the
//! installd lifecycle (this layer only references it); the adapter itself is
//! parsed by [`crate::adapter`].

use serde::Deserialize;

/// A manifest parse or validation failure. Nothing is installed on error.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest TOML could not be parsed.
    #[error("invalid manifest TOML: {0}")]
    Toml(String),
    /// A required field was missing or a value was invalid.
    #[error("invalid manifest: {0}")]
    Invalid(String),
}

/// `[package]`: identity and declared conflicts.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PackageMeta {
    /// The package id (reverse-DNS), unique across installed packages.
    pub id: String,
    /// Human-readable package name.
    pub name: String,
    /// Package version string.
    pub version: String,
    /// Ids of packages this one conflicts with (cannot be co-active).
    #[serde(default)]
    pub conflicts: Vec<String>,
}

/// `[app]`: which application the package integrates with. At least one app
/// identifier must be present.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct AppRef {
    /// The Arlen Store id.
    #[serde(default)]
    pub store_id: Option<String>,
    /// The Flatpak application id.
    #[serde(default)]
    pub flatpak_id: Option<String>,
    /// The distro package name.
    #[serde(default)]
    pub package_name: Option<String>,
    /// A semver range the app version must satisfy, e.g. `">=120.0, <130.0"`.
    #[serde(default)]
    pub compatible_with: Option<String>,
}

/// `[includes]`: optional bundled components, referenced by path within the
/// package. Both are optional; loading them is the installer's job.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct Includes {
    /// Path to a bundled permission profile (relative to the package root).
    #[serde(default)]
    pub permissions: Option<String>,
    /// Path to the bundled Settings adapter manifest.
    #[serde(default)]
    pub adapter: Option<String>,
}

/// A parsed Integration Package manifest.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IntegrationPackage {
    /// Package identity and conflicts.
    pub package: PackageMeta,
    /// The target application.
    pub app: AppRef,
    /// Optional bundled components.
    #[serde(default)]
    pub includes: Includes,
}

impl IntegrationPackage {
    /// Parse and validate a manifest. Requires a non-empty id and name, at least
    /// one app identifier, and (if present) a `compatible_with` that parses as a
    /// semver requirement.
    pub fn parse(toml_text: &str) -> Result<Self, ManifestError> {
        let pkg: IntegrationPackage =
            toml::from_str(toml_text).map_err(|e| ManifestError::Toml(e.to_string()))?;
        if pkg.package.id.trim().is_empty() {
            return Err(ManifestError::Invalid("package.id must be set".to_string()));
        }
        if pkg.package.name.trim().is_empty() {
            return Err(ManifestError::Invalid("package.name must be set".to_string()));
        }
        if pkg.app.store_id.is_none() && pkg.app.flatpak_id.is_none() && pkg.app.package_name.is_none()
        {
            return Err(ManifestError::Invalid(
                "[app] needs at least one of store_id / flatpak_id / package_name".to_string(),
            ));
        }
        if let Some(req) = pkg.app.compatible_with.as_deref() {
            semver::VersionReq::parse(req)
                .map_err(|e| ManifestError::Invalid(format!("compatible_with is not a valid range: {e}")))?;
        }
        Ok(pkg)
    }

    /// Whether `app_version` satisfies the package's `compatible_with` range. A
    /// package with no declared range is compatible with any version; an
    /// unparseable installed version is treated as incompatible (fail closed).
    pub fn is_compatible(&self, app_version: &str) -> bool {
        match self.app.compatible_with.as_deref() {
            None => true,
            Some(req) => match (semver::VersionReq::parse(req), semver::Version::parse(app_version)) {
                (Ok(req), Ok(v)) => req.matches(&v),
                _ => false,
            },
        }
    }

    /// Whether this package and `other` conflict (either declares the other's id).
    pub fn conflicts_with(&self, other: &IntegrationPackage) -> bool {
        self.package.conflicts.contains(&other.package.id)
            || other.package.conflicts.contains(&self.package.id)
    }
}

/// Every conflicting pair among `packages`, as `(id, id)` with the lexically
/// smaller id first (so a pair is reported once). Surfaced to the user at install;
/// conflicts are never silently merged.
pub fn detect_conflicts(packages: &[IntegrationPackage]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, a) in packages.iter().enumerate() {
        for b in &packages[i + 1..] {
            if a.conflicts_with(b) {
                let (x, y) = (&a.package.id, &b.package.id);
                if x <= y {
                    out.push((x.clone(), y.clone()));
                } else {
                    out.push((y.clone(), x.clone()));
                }
            }
        }
    }
    out
}

/// The ids whose permissions are SUPPRESSED by an unresolved conflict, given the
/// install order (earliest first): a package is inactive when an earlier-installed
/// package conflicts with it. This is the "first-installed-wins until the user
/// resolves it" rule - the earlier package stays active, the later one is held
/// inactive rather than silently merged.
pub fn inactive_due_to_conflict(installed_in_order: &[IntegrationPackage]) -> Vec<String> {
    let mut inactive = Vec::new();
    for (i, pkg) in installed_in_order.iter().enumerate() {
        if installed_in_order[..i].iter().any(|earlier| earlier.conflicts_with(pkg)) {
            inactive.push(pkg.package.id.clone());
        }
    }
    inactive
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(id: &str, conflicts: &[&str]) -> IntegrationPackage {
        let conflicts_toml = conflicts
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            "[package]\nid=\"{id}\"\nname=\"{id} pkg\"\nversion=\"1.0\"\nconflicts=[{conflicts_toml}]\n\
             [app]\nflatpak_id=\"org.x.{id}\""
        );
        IntegrationPackage::parse(&toml).unwrap()
    }

    #[test]
    fn parses_a_full_manifest() {
        let toml = r#"
            [package]
            id = "com.acme.firefox-tweaks"
            name = "Firefox Tweaks"
            version = "2.1"
            conflicts = ["com.other.firefox"]
            [app]
            flatpak_id = "org.mozilla.firefox"
            compatible_with = ">=120.0, <130.0"
            [includes]
            permissions = "profile.toml"
            adapter = "adapter.toml"
        "#;
        let p = IntegrationPackage::parse(toml).unwrap();
        assert_eq!(p.package.id, "com.acme.firefox-tweaks");
        assert_eq!(p.package.conflicts, vec!["com.other.firefox"]);
        assert_eq!(p.includes.permissions.as_deref(), Some("profile.toml"));
    }

    #[test]
    fn an_app_with_no_identifier_is_rejected() {
        let toml = "[package]\nid=\"x\"\nname=\"X\"\nversion=\"1\"\n[app]";
        assert!(matches!(IntegrationPackage::parse(toml), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn an_invalid_compatible_range_is_rejected() {
        let toml = "[package]\nid=\"x\"\nname=\"X\"\nversion=\"1\"\n[app]\nflatpak_id=\"a\"\ncompatible_with=\"not a range\"";
        assert!(matches!(IntegrationPackage::parse(toml), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn compatibility_matches_the_range() {
        let p = IntegrationPackage::parse(
            "[package]\nid=\"x\"\nname=\"X\"\nversion=\"1\"\n[app]\nflatpak_id=\"a\"\ncompatible_with=\">=120.0, <130.0\"",
        )
        .unwrap();
        assert!(p.is_compatible("125.0.0"));
        assert!(!p.is_compatible("119.0.0"));
        assert!(!p.is_compatible("130.0.0"));
        assert!(!p.is_compatible("garbage"), "an unparseable version fails closed");
    }

    #[test]
    fn no_range_is_compatible_with_anything() {
        let p = pkg("a", &[]);
        assert!(p.is_compatible("1.2.3"));
    }

    #[test]
    fn conflicts_are_symmetric_and_detected_once() {
        // a declares conflict with b; the relation holds both ways.
        let a = pkg("a", &["b"]);
        let b = pkg("b", &[]);
        assert!(a.conflicts_with(&b));
        assert!(b.conflicts_with(&a));
        let conflicts = detect_conflicts(&[a, b]);
        assert_eq!(conflicts, vec![("a".to_string(), "b".to_string())]);
    }

    #[test]
    fn first_installed_wins_until_resolved() {
        // Install order a (declares conflict with b), then b: b is held inactive.
        let a = pkg("a", &["b"]);
        let b = pkg("b", &[]);
        let inactive = inactive_due_to_conflict(&[a, b]);
        assert_eq!(inactive, vec!["b".to_string()], "the later-installed conflicting package is inactive");
    }

    #[test]
    fn non_conflicting_packages_are_all_active() {
        let a = pkg("a", &[]);
        let b = pkg("b", &[]);
        assert!(inactive_due_to_conflict(&[a, b]).is_empty());
        let c = pkg("c", &[]);
        assert!(detect_conflicts(&[pkg("a", &[]), pkg("b", &[]), c]).is_empty());
    }
}
