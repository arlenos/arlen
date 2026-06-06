/// Per-output top bars.
///
/// The default `main` window in `tauri.conf.json` covers the
/// primary monitor. This module discovers additional monitors via
/// GDK's `Display::n_monitors()` / `monitor(i)` and spawns one
/// extra WebviewWindow per secondary output, bound via
/// `gtk-layer-shell`'s `set_monitor`. Hot-plug is observed via the
/// `monitor-added` / `monitor-removed` signals on the Display.
///
/// We're on GTK 3 (gtk-rs 0.18) so the `gdk::Monitor::connector()`
/// API isn't available. We use the GDK monitor index as the bar's
/// identifier — stable for the lifetime of one boot. The
/// frontend's per-output workspace filtering goes through cosmic
/// protocols (`workspace_groups`), not GDK, so the index here is
/// only for bar-window-management.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const MAIN_LABEL: &str = "main";
const BAR_PREFIX: &str = "topbar-";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    /// Stable index assigned by GDK for this boot. The primary
    /// monitor isn't always index 0 — `is_primary()` decides.
    pub gdk_index: i32,
    /// Best-effort human label, falls back to "Display N".
    pub description: String,
    /// `true` for the GDK-primary monitor. Only the primary bar
    /// renders system indicators (Audio, Network, Tray, …).
    pub primary: bool,
    /// Connector name (`DP-1`, `HDMI-A-1`) resolved via the
    /// `wayland_client` thread's xdg-output cache. `None` while
    /// the cache hasn't yet seen this output (transient race
    /// during startup or hot-plug). Frontend's per-output filters
    /// fall back to the primary-only legacy path until this is
    /// populated.
    pub connector: Option<String>,
}

/// Snapshot of one wl_output's logical position + connector.
/// `wayland_client` writes the table on every output add/update;
/// `output_bars` reads it during `sync_bars` to resolve a
/// `gdk::Monitor` to its connector by matching geometry origin.
#[derive(Debug, Clone)]
pub struct OutputGeometry {
    pub x: i32,
    pub y: i32,
    pub connector: String,
}

/// Tauri-managed shared cache. Single mutex; both producer
/// (`wayland_client`) and consumer (`output_bars::sync_bars`) read
/// briefly, no contention in practice.
#[derive(Default)]
pub struct OutputConnectorTable {
    inner: Arc<Mutex<Vec<OutputGeometry>>>,
}

impl OutputConnectorTable {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update(&self, table: Vec<OutputGeometry>) {
        *self.inner.lock().unwrap() = table;
    }
    /// Resolve a `(x, y)` GDK monitor origin to a single connector
    /// name. Returns `None` when zero or MORE THAN ONE outputs
    /// share the same logical position — the latter happens with
    /// mirrored displays where multiple wl_outputs report the
    /// same `(0, 0)` and we cannot tell them apart from the
    /// origin alone. Refusing the match in that case keeps
    /// per-output filtering deterministic (frontend falls back to
    /// the legacy global view on the affected bars) instead of
    /// silently routing both bars to the same connector.
    pub fn lookup_at(&self, x: i32, y: i32) -> Option<String> {
        let g = self.inner.lock().unwrap();
        let matches: Vec<&OutputGeometry> =
            g.iter().filter(|g| g.x == x && g.y == y).collect();
        if matches.len() == 1 {
            Some(matches[0].connector.clone())
        } else {
            if matches.len() > 1 {
                log::debug!(
                    "output_bars: ambiguous origin ({x}, {y}) matches {} outputs ({}), refusing per-output assignment",
                    matches.len(),
                    matches.iter().map(|g| g.connector.as_str()).collect::<Vec<_>>().join(", "),
                );
            }
            None
        }
    }
}

#[derive(Default)]
pub struct OutputBarRegistry {
    inner: Arc<Mutex<HashMap<String, OutputInfo>>>,
}

impl OutputBarRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn set(&self, label: &str, info: OutputInfo) {
        let mut g = self.inner.lock().unwrap();
        g.insert(label.to_string(), info);
    }

    fn remove(&self, label: &str) {
        let mut g = self.inner.lock().unwrap();
        g.remove(label);
    }

    pub fn get(&self, label: &str) -> Option<OutputInfo> {
        let g = self.inner.lock().unwrap();
        g.get(label).cloned()
    }

    fn keys(&self) -> Vec<String> {
        let g = self.inner.lock().unwrap();
        g.keys().cloned().collect()
    }
}

/// Frontend-facing command. Each bar's `+page.svelte` invokes this
/// on mount with the implicit window context to learn its output.
#[tauri::command]
pub fn topbar_get_output(
    window: tauri::Window,
    registry: tauri::State<'_, Arc<OutputBarRegistry>>,
) -> Option<OutputInfo> {
    registry.get(window.label())
}

/// Discover monitors via GDK and spin up bars. Called from the GTK
/// main thread (via `glib::idle_add_once`) after the `main`
/// window's layer-shell init.
pub fn install(
    app: AppHandle,
    registry: Arc<OutputBarRegistry>,
    table: Arc<OutputConnectorTable>,
) {
    let display = match gtk::gdk::Display::default() {
        Some(d) => d,
        None => {
            log::warn!("output_bars: no GDK display, skipping per-output bars");
            return;
        }
    };

    log::info!(
        "output_bars: GDK reports {} monitor(s) at startup",
        display.n_monitors(),
    );

    sync_bars(&app, &registry, &table, &display);

    // Hot-plug. GDK 3 emits `monitor-added` / `monitor-removed` on
    // the Display. Both trigger a full re-sync (cheap — current
    // hardware tops out at 4-6 monitors).
    let app_added = app.clone();
    let registry_added = Arc::clone(&registry);
    let table_added = Arc::clone(&table);
    let app_removed = app.clone();
    let registry_removed = Arc::clone(&registry);
    let table_removed = Arc::clone(&table);
    display.connect_monitor_added(move |display, _monitor| {
        log::info!(
            "output_bars: monitor added (now {})",
            display.n_monitors()
        );
        sync_bars(&app_added, &registry_added, &table_added, display);
    });
    display.connect_monitor_removed(move |display, _monitor| {
        log::info!(
            "output_bars: monitor removed (now {})",
            display.n_monitors()
        );
        sync_bars(&app_removed, &registry_removed, &table_removed, display);
    });
}

/// Re-run a sync from outside the GDK signal handlers. Called by
/// `wayland_client` (a separate thread) after the xdg-output
/// table fills in, so the registry's `connector` field gets
/// backfilled without waiting for a hot-plug.
///
/// Uses `glib::idle_add_once` (Send-safe variant) so the actual
/// GDK calls land on the GTK main thread regardless of which
/// thread invoked the refresh.
pub fn refresh(
    app: AppHandle,
    registry: Arc<OutputBarRegistry>,
    table: Arc<OutputConnectorTable>,
) {
    glib::idle_add_once(move || {
        if let Some(display) = gtk::gdk::Display::default() {
            sync_bars(&app, &registry, &table, &display);
        }
    });
}

fn sync_bars(
    app: &AppHandle,
    registry: &Arc<OutputBarRegistry>,
    table: &Arc<OutputConnectorTable>,
    display: &gtk::gdk::Display,
) {
    use gtk::gdk::prelude::MonitorExt;

    let n = display.n_monitors();
    let mut seen_labels: Vec<String> = Vec::new();

    // Pre-pick the primary monitor index. GDK reports
    // `is_primary()` true on real DRM, but Winit / X11-nested /
    // some single-output setups flag none — fall back to monitor 0
    // so we always have exactly one bar bound to `main`. Doing this
    // BEFORE the loop avoids a race where the per-monitor branch
    // creates a `topbar-0` for monitor 0, then a separate
    // post-loop fallback creates `main` for the same monitor 0,
    // stacking two bars on the same output.
    let primary_idx = (0..n)
        .find(|i| {
            display
                .monitor(*i)
                .map(|m| m.is_primary())
                .unwrap_or(false)
        })
        .unwrap_or(0);

    for i in 0..n {
        let Some(monitor) = display.monitor(i) else { continue };
        let is_primary_monitor = i == primary_idx;
        let description = describe_monitor(&monitor, i);
        // Resolve connector via the wl_output table populated by
        // `wayland_client`. Match by GDK monitor's geometry origin
        // — `xdg_output::logical_position` and GDK's geometry use
        // the same compositor-coordinate space, so equal `(x, y)`
        // identifies the same physical output.
        let geom = monitor.geometry();
        let connector = table.lookup_at(geom.x(), geom.y());

        let info = OutputInfo {
            gdk_index: i,
            description,
            primary: is_primary_monitor,
            connector,
        };

        if is_primary_monitor {
            registry.set(MAIN_LABEL, info.clone());
            if let Some(w) = app.get_webview_window(MAIN_LABEL) {
                bind_window_to_monitor(&w, &monitor);
            }
            seen_labels.push(MAIN_LABEL.to_string());
        } else {
            let label = format!("{BAR_PREFIX}{i}");
            seen_labels.push(label.clone());
            // Write the registry entry BEFORE building the webview
            // so a fast-mounting frontend's `topbar_get_output`
            // call can never see `None` for an existing window.
            registry.set(&label, info);

            let exists = app.get_webview_window(&label).is_some();
            if !exists {
                if let Err(err) = create_secondary_bar(app, &label, &monitor) {
                    log::error!(
                        "output_bars: failed to create secondary bar idx={i}: {err}",
                    );
                    continue;
                }
            } else {
                // Existing windows must be rebound on every sync.
                // GDK 3 renumbers monitors when one disconnects, so
                // the same `topbar-N` label can now refer to a
                // different physical monitor — without re-issuing
                // `set_monitor`, the bar would still be attached to
                // its old (gone) output. Rebinding is a cheap
                // idempotent ioctl when the monitor hasn't changed.
                if let Some(w) = app.get_webview_window(&label) {
                    bind_window_to_monitor(&w, &monitor);
                }
            }
        }
    }

    let stale: Vec<String> = registry
        .keys()
        .into_iter()
        .filter(|k| !seen_labels.contains(k))
        .collect();
    for label in stale {
        if label == MAIN_LABEL {
            registry.remove(&label);
            continue;
        }
        log::info!("output_bars: tearing down stale bar '{label}'");
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.close();
        }
        registry.remove(&label);
    }

    // Notify every TopBar instance that the registry changed, so
    // bars that mounted with `connector: null` (xdg-output name
    // hadn't arrived yet) re-fetch and pick up the resolved
    // connector. Without this the secondary bar would remain
    // permanently in the legacy primary-only fallback.
    let _ = app.emit("lunaris://topbar-output-changed", ());
}

fn create_secondary_bar(
    app: &AppHandle,
    label: &str,
    monitor: &gtk::gdk::Monitor,
) -> Result<(), Box<dyn std::error::Error>> {
    let window =
        WebviewWindowBuilder::new(app, label, WebviewUrl::App("/".into()))
            .title("Lunaris Top Bar")
            .visible(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .build()?;

    log::info!("output_bars: created window '{label}'");

    // Capture only the monitor index — `gdk::Monitor` itself is
    // `!Send` because it wraps a raw GTK pointer, and
    // `with_webview` posts to the webview thread which requires
    // `Send`. We refetch the monitor by index inside the closure
    // (we're on the GTK main thread there).
    let idx = monitor_index(monitor).unwrap_or(0);
    let window_clone = window.clone();
    glib::idle_add_local_once(move || {
        if let Err(e) = init_secondary_layer_shell(window_clone, idx) {
            log::error!("output_bars: layer-shell init failed: {e}");
        }
    });
    Ok(())
}

fn init_secondary_layer_shell(
    window: WebviewWindow,
    monitor_idx: i32,
) -> Result<(), tauri::Error> {
    window.with_webview(move |webview| {
        
        use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
        use gtk_layer_shell::{Edge, Layer, LayerShell};

        let Some(toplevel) = webview.inner().toplevel() else {
            log::warn!("output_bars: secondary toplevel is None");
            return;
        };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else {
            log::warn!("output_bars: secondary downcast failed");
            return;
        };

        let Some(display) = gtk::gdk::Display::default() else {
            log::warn!("output_bars: no GDK display in init");
            return;
        };
        let Some(monitor) = display.monitor(monitor_idx) else {
            log::warn!("output_bars: no monitor at idx={monitor_idx} in init");
            return;
        };

        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Top);
        gtk_window.set_anchor(Edge::Top, true);
        gtk_window.set_anchor(Edge::Left, true);
        gtk_window.set_anchor(Edge::Right, true);
        gtk_window.set_anchor(Edge::Bottom, true);
        gtk_window.set_exclusive_zone(36);
        gtk_window.set_monitor(&monitor);
        gtk_window.set_size_request(-1, -1);
        gtk_window.show_all();

        {
            use gtk::cairo::{RectangleInt, Region};
            let bar = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 36));
            gtk_window.input_shape_combine_region(Some(&bar));
        }
        gtk_window.queue_draw();
        display.flush();
        log::info!(
            "output_bars: secondary layer-shell ready on monitor idx={monitor_idx}",
        );
    })?;
    Ok(())
}

fn bind_window_to_monitor(window: &WebviewWindow, monitor: &gtk::gdk::Monitor) {
    let idx = monitor_index(monitor).unwrap_or(0);
    let _ = window.with_webview(move |webview| {
        
        use gtk::prelude::{Cast, WidgetExt};
        use gtk_layer_shell::LayerShell;
        if let Some(toplevel) = webview.inner().toplevel() {
            if let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() {
                if let Some(display) = gtk::gdk::Display::default() {
                    if let Some(m) = display.monitor(idx) {
                        gtk_window.set_monitor(&m);
                    }
                }
            }
        }
    });
}

/// Find the index of `monitor` in the default Display's monitor
/// list. Used to pass identity across thread boundaries without
/// capturing the `!Send` monitor handle itself.
fn monitor_index(monitor: &gtk::gdk::Monitor) -> Option<i32> {
    
    let display = gtk::gdk::Display::default()?;
    for i in 0..display.n_monitors() {
        if let Some(m) = display.monitor(i) {
            if m == *monitor {
                return Some(i);
            }
        }
    }
    None
}

fn describe_monitor(monitor: &gtk::gdk::Monitor, idx: i32) -> String {
    use gtk::prelude::MonitorExt;
    let make = monitor.manufacturer().map(|s| s.to_string()).unwrap_or_default();
    let model = monitor.model().map(|s| s.to_string()).unwrap_or_default();
    if !make.is_empty() && !model.is_empty() {
        format!("{make} {model}")
    } else if !make.is_empty() {
        make
    } else if !model.is_empty() {
        model
    } else {
        format!("Display {}", idx + 1)
    }
}
