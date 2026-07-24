//! `arlen-wallpaperd`: renders the configured wallpaper on the Background layer
//! of every output via `wlr-layer-shell`. The pixel work (decode + Fill/Zoom
//! compose) is the pure `decode` module; this binary is the Wayland client
//! ([`arlen_wallpaper::render`]) plus manifest loading.

use std::time::{SystemTime, UNIX_EPOCH};

use arlen_wallpaper::config;
use arlen_wallpaper::manifest::WallpaperManifest;
use arlen_wallpaper::render::Wallpaper;
use arlen_wallpaper::schedule::TimeContext;
use wayland_client::Connection;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(manifest) = load_manifest() else {
        tracing::info!("no wallpaper manifest configured; nothing to render");
        return;
    };
    let time = TimeContext::at_minute(minute_of_day());

    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("no wayland display: {e}");
            return;
        }
    };
    let (mut wallpaper, mut queue) = match Wallpaper::new(&conn, manifest, time) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("wallpaper init failed: {e}");
            return;
        }
    };
    tracing::info!("arlen-wallpaperd rendering");
    loop {
        if let Err(e) = queue.blocking_dispatch(&mut wallpaper) {
            tracing::error!("wayland dispatch ended: {e}");
            break;
        }
    }
}

/// Load the wallpaper manifest: the `ARLEN_WALLPAPER_MANIFEST` override (for
/// testing) else the user path (`~/.config/arlen/wallpaper.toml`). `None` when
/// unset or unreadable - the daemon then renders nothing.
fn load_manifest() -> Option<WallpaperManifest> {
    let path = std::env::var_os("ARLEN_WALLPAPER_MANIFEST")
        .map(std::path::PathBuf::from)
        .or_else(config::user_manifest_path)?;
    match config::load_manifest(&path) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(path = %path.display(), "could not load wallpaper manifest: {e}");
            None
        }
    }
}

/// Minutes since midnight for the time-of-day source selection. Uses UTC as a
/// stable approximation (a static wallpaper - the common case - has no
/// time-of-day variants, so this only affects a `[[variants]]` manifest; real
/// local-time/sun-time selection is the schedule refinement).
fn minute_of_day() -> u32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ((secs % 86_400) / 60) as u32
}
