//! The data-only wallpaper manifest (wallpaper-plan.md Decided 8).
//!
//! A wallpaper is a TOML MANIFEST referencing plain asset files (image / video /
//! shader source), with NO executable code. KDE's wallpaper format is a
//! plasmoid-style code package (`metadata.json` + `contents/ui/main.qml` +
//! `contents/code/*.js`) that runs as the desktop background, the same
//! code-execution class as web wallpapers (and the same lock-screen hazard).
//! Arlen does not adopt that. This manifest parses into a CLOSED, fully-typed
//! model: every field is data (an enum, a file path, a number), so a manifest has
//! no field through which it could smuggle code, and the engine INTERPRETS the
//! manifest and renders the referenced assets sandboxed - it never executes
//! manifest content. That is the data-not-code property that keeps the renderer
//! sandboxable.
//!
//! The asset paths are DATA the engine loads, never interpolated into a shell or
//! code. This module validates them as inert strings (non-empty, no NUL or other
//! control characters); the engine is responsible for loading them under its own
//! capability confinement (a downloaded asset is `Origin::ExternalContent`).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// The largest crossfade a manifest may request. A longer value is clamped by
/// [`WallpaperManifest::transition`], never trusted unbounded.
pub const MAX_TRANSITION: Duration = Duration::from_secs(10);

/// The kind of wallpaper. A CLOSED set: a new kind is a new variant here, never a
/// free string, so the engine's dispatch stays exhaustive and a manifest can never
/// name an unknown (or code-bearing) renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperKind {
    /// A static image.
    Image,
    /// A looping video (mpv-class), rendered sandboxed.
    Video,
    /// A GPU shader (the low-power live alternative), rendered sandboxed.
    Shader,
    /// Time-of-day variants (selected by the WP-R4 schedule); each variant is an
    /// image / video / shader source.
    Dynamic,
}

/// How a source fills its output. Fill or Zoom only - never Stretch, which
/// distorts the image (the picker offers exactly these two).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    /// Cover the output, cropping overflow, preserving aspect.
    Fill,
    /// Fit the whole asset, letterboxing, preserving aspect.
    Zoom,
}

/// One rendered source: a plain asset file plus how it is shown. The `asset` is
/// DATA the engine loads, never code it runs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Source {
    /// The asset file path (image / video / shader source). A plain file the
    /// engine loads under its own confinement; never interpolated into code.
    pub asset: String,
    /// Fill or Zoom.
    pub scale: Scale,
    /// Loop a video source (ignored for a static image).
    #[serde(default)]
    pub loop_playback: bool,
}

/// A time-of-day phase a dynamic wallpaper switches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimePhase {
    /// The dawn transition window.
    Sunrise,
    /// Full daylight.
    Day,
    /// The dusk transition window.
    Sunset,
    /// Night.
    Night,
}

/// A dynamic-wallpaper variant: the source shown during one phase.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TimeVariant {
    /// The phase this variant is active in.
    pub phase: TimePhase,
    /// The source shown during the phase.
    pub source: Source,
}

/// A wallpaper manifest. Data-only (see the module doc): the engine interprets it
/// and renders the referenced assets sandboxed, never executing any of it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallpaperManifest {
    /// The wallpaper kind.
    pub kind: WallpaperKind,
    /// The source for a monitor with no per-monitor override, and the sole source
    /// of a static, non-per-monitor wallpaper. For a `Dynamic` kind it is the
    /// fallback when no time variant covers the current phase.
    pub default: Source,
    /// Per-monitor overrides, keyed by connector name (e.g. `"DP-1"`).
    #[serde(default)]
    pub per_monitor: BTreeMap<String, Source>,
    /// Time-of-day variants. Required for `Dynamic`, and rejected for any other
    /// kind (a static wallpaper has no phases).
    #[serde(default)]
    pub variants: Vec<TimeVariant>,
    /// The crossfade between sources, in milliseconds (0 = instant). Clamped to
    /// [`MAX_TRANSITION`] by [`WallpaperManifest::transition`].
    #[serde(default)]
    pub transition_ms: u64,
}

/// Why a manifest was rejected. Parsing and validation are fail-closed: a
/// malformed or code-smelling manifest is an error, never a permissive default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// The TOML did not parse, or carried an unknown field / wrong type.
    #[error("malformed manifest: {0}")]
    Parse(String),
    /// An asset path was empty.
    #[error("source asset path is empty")]
    EmptyAsset,
    /// An asset path carried a NUL or other control character (a path that reaches
    /// the engine's loader must be an inert string).
    #[error("source asset path carries a control character")]
    ControlInAsset,
    /// A `Dynamic` manifest carried no time variants (it would never switch).
    #[error("a dynamic wallpaper needs at least one time variant")]
    DynamicWithoutVariants,
    /// A non-`Dynamic` manifest carried time variants (a static wallpaper has no
    /// phases; the variants would be silently ignored, so reject them).
    #[error("only a dynamic wallpaper may carry time variants")]
    VariantsOnStatic,
}

impl WallpaperManifest {
    /// Parse a manifest from TOML and validate it, fail-closed.
    pub fn parse(toml_text: &str) -> Result<Self, ManifestError> {
        let manifest: WallpaperManifest =
            toml::from_str(toml_text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the manifest: every asset path is an inert non-empty string, and
    /// the time variants match the kind. Structural type-safety (the closed
    /// enums, `deny_unknown_fields`) already guarantees no field carries code;
    /// this adds the value-level bounds.
    pub fn validate(&self) -> Result<(), ManifestError> {
        check_source(&self.default)?;
        for source in self.per_monitor.values() {
            check_source(source)?;
        }
        for variant in &self.variants {
            check_source(&variant.source)?;
        }
        match self.kind {
            WallpaperKind::Dynamic if self.variants.is_empty() => {
                Err(ManifestError::DynamicWithoutVariants)
            }
            WallpaperKind::Dynamic => Ok(()),
            _ if !self.variants.is_empty() => Err(ManifestError::VariantsOnStatic),
            _ => Ok(()),
        }
    }

    /// The crossfade duration, clamped to [`MAX_TRANSITION`] so a manifest cannot
    /// request an unbounded transition.
    pub fn transition(&self) -> Duration {
        Duration::from_millis(self.transition_ms).min(MAX_TRANSITION)
    }
}

/// Validate one source's asset path: non-empty and free of control characters.
fn check_source(source: &Source) -> Result<(), ManifestError> {
    if source.asset.is_empty() {
        return Err(ManifestError::EmptyAsset);
    }
    if source.asset.chars().any(|c| c.is_control()) {
        return Err(ManifestError::ControlInAsset);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_static_image_manifest() {
        let m = WallpaperManifest::parse(
            r#"
            kind = "image"
            [default]
            asset = "/usr/share/backgrounds/arlen.png"
            scale = "fill"
            "#,
        )
        .unwrap();
        assert_eq!(m.kind, WallpaperKind::Image);
        assert_eq!(m.default.asset, "/usr/share/backgrounds/arlen.png");
        assert_eq!(m.default.scale, Scale::Fill);
        assert!(!m.default.loop_playback);
        assert!(m.per_monitor.is_empty());
    }

    #[test]
    fn parses_per_monitor_and_dynamic_variants() {
        let m = WallpaperManifest::parse(
            r#"
            kind = "dynamic"
            transition_ms = 800
            [default]
            asset = "day.png"
            scale = "zoom"
            [per_monitor."DP-1"]
            asset = "ultrawide.png"
            scale = "fill"
            [[variants]]
            phase = "night"
            [variants.source]
            asset = "night.png"
            scale = "zoom"
            "#,
        )
        .unwrap();
        assert_eq!(m.kind, WallpaperKind::Dynamic);
        assert_eq!(m.per_monitor["DP-1"].asset, "ultrawide.png");
        assert_eq!(m.variants.len(), 1);
        assert_eq!(m.variants[0].phase, TimePhase::Night);
        assert_eq!(m.transition(), Duration::from_millis(800));
    }

    #[test]
    fn rejects_an_unknown_field_and_a_bad_kind() {
        // deny_unknown_fields keeps a manifest from carrying anything off-model.
        assert!(matches!(
            WallpaperManifest::parse("kind = \"image\"\nscript = \"evil.js\"\n[default]\nasset=\"a\"\nscale=\"fill\""),
            Err(ManifestError::Parse(_))
        ));
        // An unknown kind (e.g. a code-bearing "qml") is not in the closed enum.
        assert!(matches!(
            WallpaperManifest::parse("kind = \"qml\"\n[default]\nasset=\"a\"\nscale=\"fill\""),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn rejects_an_empty_or_control_bearing_asset() {
        assert_eq!(
            WallpaperManifest::parse("kind=\"image\"\n[default]\nasset=\"\"\nscale=\"fill\""),
            Err(ManifestError::EmptyAsset)
        );
        assert_eq!(
            WallpaperManifest::parse("kind=\"image\"\n[default]\nasset=\"a\\u0000b\"\nscale=\"fill\""),
            Err(ManifestError::ControlInAsset)
        );
    }

    #[test]
    fn enforces_the_variants_vs_kind_rule() {
        // Dynamic without variants would never switch.
        assert_eq!(
            WallpaperManifest::parse("kind=\"dynamic\"\n[default]\nasset=\"a\"\nscale=\"fill\""),
            Err(ManifestError::DynamicWithoutVariants)
        );
        // A static kind with variants: reject (they would be silently ignored).
        let static_with_variants = "kind=\"image\"\n[default]\nasset=\"a\"\nscale=\"fill\"\n[[variants]]\nphase=\"day\"\n[variants.source]\nasset=\"b\"\nscale=\"fill\"";
        assert_eq!(
            WallpaperManifest::parse(static_with_variants),
            Err(ManifestError::VariantsOnStatic)
        );
    }

    #[test]
    fn clamps_an_oversized_transition() {
        let m = WallpaperManifest::parse(
            "kind=\"image\"\ntransition_ms=99999999\n[default]\nasset=\"a\"\nscale=\"fill\"",
        )
        .unwrap();
        assert_eq!(m.transition(), MAX_TRANSITION);
    }
}
