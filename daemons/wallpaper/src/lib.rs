//! Arlen wallpaper: the shared data-only manifest model (wallpaper-plan.md WP-R1).
//!
//! The background-layer client (the renderer) and the Settings picker are two
//! consumers of ONE wallpaper model: a data-only TOML [`manifest`] referencing
//! plain asset files. Unlike KDE's plasmoid wallpaper format (a `main.qml` +
//! `*.js` package that RUNS code as the desktop background, the same
//! code-execution and lock-screen hazard as web wallpapers), an Arlen wallpaper
//! carries no executable code: every manifest field is data (an enum, a path, a
//! number), the engine interprets the manifest and renders the referenced assets
//! sandboxed, and it never executes manifest content. That data-not-code property
//! is what keeps the renderer sandboxable, the same floor the theme system and the
//! Settings Adapter rest on.
//!
//! This crate is the manifest model + its validation; the time-of-day selection
//! (WP-R4) and the renderer client build on it.

pub mod manifest;

pub use manifest::{
    ManifestError, Scale, Source, TimePhase, TimeVariant, WallpaperKind, WallpaperManifest,
};
