//! WP-R4 time-of-day selection: which source a wallpaper shows right now.
//!
//! A `Dynamic` wallpaper carries time-of-day [`TimeVariant`]s; this module is the
//! pure decision of which one is active at a given moment. [`phase_at`] maps the
//! clock (and optional sun times) to a [`TimePhase`]; [`active_source`] picks the
//! variant for that phase (falling back to the manifest default when no variant
//! covers it), and [`source_for_monitor`] layers a per-monitor override on top.
//! Everything here is pure and total: it always returns a source and never
//! panics, so the renderer can call it on every frame-tick cheaply. The crossfade
//! between a source change is the renderer's, driven by
//! [`WallpaperManifest::transition`](crate::manifest::WallpaperManifest::transition).

use crate::manifest::{Source, TimePhase, WallpaperKind, WallpaperManifest};

/// Fixed sunrise boundary (minute of day) when no real sun time is supplied.
const FIXED_SUNRISE: u32 = 6 * 60;
/// Fixed sunset boundary (minute of day) when no real sun time is supplied.
const FIXED_SUNSET: u32 = 18 * 60;
/// The dawn/dusk transition window in minutes: `[sun, sun + window)` is the
/// Sunrise / Sunset phase, the span between the windows is Day, the rest Night.
const PHASE_WINDOW: u32 = 60;
/// The last valid minute of a day.
const LAST_MINUTE: u32 = 24 * 60 - 1;

/// The clock context a selection is made against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeContext {
    /// Minutes since local midnight, clamped to `0..=1439`.
    pub minute_of_day: u32,
    /// Sunrise as a minute of day, for sunrise-sunset mode. `None` uses the fixed
    /// fallback boundary.
    pub sunrise: Option<u32>,
    /// Sunset as a minute of day. `None` uses the fixed fallback boundary.
    pub sunset: Option<u32>,
}

impl TimeContext {
    /// A context from a minute of day using the fixed day/night boundaries (no
    /// real sun times). The renderer supplies sun times via the struct fields when
    /// it has the location.
    pub fn at_minute(minute_of_day: u32) -> Self {
        Self {
            minute_of_day,
            sunrise: None,
            sunset: None,
        }
    }
}

/// Whether `m` is in the half-open window `[start, start + window)`.
fn in_window(m: u32, start: u32, window: u32) -> bool {
    m >= start && m < start.saturating_add(window)
}

/// The phase at `ctx`: a window around sunrise is Sunrise and around sunset is
/// Sunset, the span between them is Day, and everything else (including before
/// sunrise and after the sunset window) is Night. Degenerate inputs (sun times
/// out of order or a window that runs off the end of the day) still resolve to a
/// definite phase rather than panicking.
pub fn phase_at(ctx: &TimeContext) -> TimePhase {
    let m = ctx.minute_of_day.min(LAST_MINUTE);
    let sunrise = ctx.sunrise.unwrap_or(FIXED_SUNRISE).min(LAST_MINUTE);
    let sunset = ctx.sunset.unwrap_or(FIXED_SUNSET).min(LAST_MINUTE);

    // The dawn / dusk windows take precedence so a moment inside one is that
    // transition phase even when the day/night spans would also cover it.
    if in_window(m, sunrise, PHASE_WINDOW) {
        return TimePhase::Sunrise;
    }
    if in_window(m, sunset, PHASE_WINDOW) {
        return TimePhase::Sunset;
    }
    // Day runs from the end of the sunrise window up to sunset (empty if the sun
    // times are degenerate, which just leaves Sunrise/Sunset/Night).
    let day_start = sunrise.saturating_add(PHASE_WINDOW);
    if m >= day_start && m < sunset {
        return TimePhase::Day;
    }
    TimePhase::Night
}

/// The source a `manifest` shows at `ctx`. For a `Dynamic` wallpaper, the variant
/// whose phase is active now, falling back to the manifest default when no variant
/// covers that phase; for any other kind, the default source.
pub fn active_source<'a>(manifest: &'a WallpaperManifest, ctx: &TimeContext) -> &'a Source {
    if manifest.kind != WallpaperKind::Dynamic {
        return &manifest.default;
    }
    let phase = phase_at(ctx);
    manifest
        .variants
        .iter()
        .find(|v| v.phase == phase)
        .map(|v| &v.source)
        .unwrap_or(&manifest.default)
}

/// The source for the monitor named `connector` at `ctx`. A per-monitor override
/// is a STATIC assignment for that output and wins outright; a monitor with no
/// override shows the shared, time-active source ([`active_source`]). Per-monitor
/// time-of-day variation is not expressible (the manifest's variants are global),
/// which is the deliberate simple policy: per-monitor is a fixed override, the
/// dynamic schedule drives the shared default.
pub fn source_for_monitor<'a>(
    manifest: &'a WallpaperManifest,
    connector: &str,
    ctx: &TimeContext,
) -> &'a Source {
    manifest
        .per_monitor
        .get(connector)
        .unwrap_or_else(|| active_source(manifest, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::WallpaperManifest;

    fn dynamic_manifest() -> WallpaperManifest {
        WallpaperManifest::parse(
            r#"
            kind = "dynamic"
            [default]
            asset = "fallback.png"
            scale = "fill"
            [per_monitor."DP-1"]
            asset = "monitor-fixed.png"
            scale = "fill"
            [[variants]]
            phase = "day"
            [variants.source]
            asset = "day.png"
            scale = "fill"
            [[variants]]
            phase = "night"
            [variants.source]
            asset = "night.png"
            scale = "fill"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn phases_map_around_the_fixed_boundaries() {
        // Fixed sunrise 06:00, sunset 18:00, window 60m.
        assert_eq!(phase_at(&TimeContext::at_minute(3 * 60)), TimePhase::Night); // 03:00
        assert_eq!(phase_at(&TimeContext::at_minute(6 * 60 + 10)), TimePhase::Sunrise); // 06:10
        assert_eq!(phase_at(&TimeContext::at_minute(12 * 60)), TimePhase::Day); // 12:00
        assert_eq!(phase_at(&TimeContext::at_minute(18 * 60 + 10)), TimePhase::Sunset); // 18:10
        assert_eq!(phase_at(&TimeContext::at_minute(22 * 60)), TimePhase::Night); // 22:00
        // Just before sunrise is still Night.
        assert_eq!(phase_at(&TimeContext::at_minute(5 * 60 + 59)), TimePhase::Night);
    }

    #[test]
    fn supplied_sun_times_override_the_fixed_boundaries() {
        // A late polar sunrise: 09:00 sunrise means 07:00 is still Night.
        let ctx = TimeContext {
            minute_of_day: 7 * 60,
            sunrise: Some(9 * 60),
            sunset: Some(17 * 60),
        };
        assert_eq!(phase_at(&ctx), TimePhase::Night);
        let dawn = TimeContext {
            minute_of_day: 9 * 60 + 5,
            ..ctx
        };
        assert_eq!(phase_at(&dawn), TimePhase::Sunrise);
    }

    #[test]
    fn active_source_picks_the_phase_variant_and_falls_back() {
        let m = dynamic_manifest();
        // Day -> the day variant.
        assert_eq!(active_source(&m, &TimeContext::at_minute(12 * 60)).asset, "day.png");
        // Night -> the night variant.
        assert_eq!(active_source(&m, &TimeContext::at_minute(23 * 60)).asset, "night.png");
        // Sunrise has no variant -> the default fallback.
        assert_eq!(
            active_source(&m, &TimeContext::at_minute(6 * 60 + 10)).asset,
            "fallback.png"
        );
    }

    #[test]
    fn a_static_manifest_always_shows_its_default() {
        let m = WallpaperManifest::parse(
            "kind=\"image\"\n[default]\nasset=\"static.png\"\nscale=\"zoom\"",
        )
        .unwrap();
        // The clock is irrelevant for a non-dynamic kind.
        assert_eq!(active_source(&m, &TimeContext::at_minute(2 * 60)).asset, "static.png");
        assert_eq!(active_source(&m, &TimeContext::at_minute(14 * 60)).asset, "static.png");
    }

    #[test]
    fn a_per_monitor_override_is_a_static_assignment() {
        let m = dynamic_manifest();
        // DP-1 has a fixed override regardless of the time of day.
        assert_eq!(
            source_for_monitor(&m, "DP-1", &TimeContext::at_minute(12 * 60)).asset,
            "monitor-fixed.png"
        );
        assert_eq!(
            source_for_monitor(&m, "DP-1", &TimeContext::at_minute(23 * 60)).asset,
            "monitor-fixed.png"
        );
        // A monitor with no override follows the time-active shared source.
        assert_eq!(
            source_for_monitor(&m, "HDMI-A-1", &TimeContext::at_minute(12 * 60)).asset,
            "day.png"
        );
    }

    #[test]
    fn selection_is_total_on_extreme_inputs() {
        let m = dynamic_manifest();
        // An out-of-range minute and out-of-order sun times must not panic and
        // must still yield a source.
        let weird = TimeContext {
            minute_of_day: u32::MAX,
            sunrise: Some(u32::MAX),
            sunset: Some(0),
        };
        let _ = phase_at(&weird);
        let _ = active_source(&m, &weird);
        let _ = source_for_monitor(&m, "DP-1", &weird);
    }
}
