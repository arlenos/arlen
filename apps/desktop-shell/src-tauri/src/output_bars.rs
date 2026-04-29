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
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

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
pub fn install(app: AppHandle, registry: Arc<OutputBarRegistry>) {
    

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

    sync_bars(&app, &registry, &display);

    // Hot-plug. GDK 3 emits `monitor-added` / `monitor-removed` on
    // the Display. Both trigger a full re-sync (cheap — current
    // hardware tops out at 4-6 monitors).
    let app_added = app.clone();
    let registry_added = Arc::clone(&registry);
    let app_removed = app.clone();
    let registry_removed = Arc::clone(&registry);
    display.connect_monitor_added(move |display, _monitor| {
        log::info!(
            "output_bars: monitor added (now {})",
            display.n_monitors()
        );
        sync_bars(&app_added, &registry_added, display);
    });
    display.connect_monitor_removed(move |display, _monitor| {
        log::info!(
            "output_bars: monitor removed (now {})",
            display.n_monitors()
        );
        sync_bars(&app_removed, &registry_removed, display);
    });
}

fn sync_bars(
    app: &AppHandle,
    registry: &Arc<OutputBarRegistry>,
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

        let info = OutputInfo {
            gdk_index: i,
            description,
            primary: is_primary_monitor,
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
