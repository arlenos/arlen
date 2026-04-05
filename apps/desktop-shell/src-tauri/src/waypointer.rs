/// Waypointer launcher overlay.
///
/// A second Tauri WebviewWindow configured as a fullscreen layer-shell
/// surface on the Overlay layer. Hidden by default; toggled open by the
/// Super key (via `waypointer_open` protocol event) or Tauri command.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

static WAYPOINTER_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Creates the Waypointer WebviewWindow (hidden) and configures it as
/// a layer-shell overlay surface.
pub fn create_window(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let _wp = WebviewWindowBuilder::new(app, "waypointer", WebviewUrl::App("/waypointer".into()))
        .title("Waypointer")
        .visible(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .build()?;

    log::info!("waypointer: window created (hidden)");
    Ok(())
}

/// Initialises the Waypointer window as a layer-shell surface.
///
/// Must be called from the GTK main thread (via `glib::idle_add_once`).
pub fn init_layer_shell(window: tauri::WebviewWindow) {
    if let Err(e) = window.with_webview(|webview| {
        use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
        use gtk_layer_shell::LayerShell;
        use gtk_layer_shell::{KeyboardMode, Layer};

        let Some(toplevel) = webview.inner().toplevel() else {
            log::warn!("waypointer: toplevel is None");
            return;
        };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else {
            log::warn!("waypointer: downcast to gtk::Window failed");
            return;
        };

        use gtk_layer_shell::Edge;

        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        // All four anchors so the compositor assigns full output size.
        gtk_window.set_anchor(Edge::Top, true);
        gtk_window.set_anchor(Edge::Bottom, true);
        gtk_window.set_anchor(Edge::Left, true);
        gtk_window.set_anchor(Edge::Right, true);
        // No exclusive zone: don't push other surfaces.
        gtk_window.set_exclusive_zone(-1);
        gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
        // Start with zero input region (hidden = no input).
        {
            use gtk::cairo::{RectangleInt, Region};
            let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
            gtk_window.input_shape_combine_region(Some(&empty));
        }

        log::info!(
            "waypointer::init_layer_shell: is_layer_window={} layer={:?} title={:?} alloc={}x{}",
            gtk_window.is_layer_window(),
            gtk_window.layer(),
            gtk_window.title().map(|t| t.to_string()),
            gtk_window.allocated_width(),
            gtk_window.allocated_height(),
        );
    }) {
        log::error!("waypointer: init_layer_shell failed: {e}");
    }
}

/// Shows the Waypointer overlay with fullscreen input region.
pub fn show(app: &AppHandle) {
    if WAYPOINTER_VISIBLE.load(Ordering::SeqCst) {
        return;
    }
    WAYPOINTER_VISIBLE.store(true, Ordering::SeqCst);

    if let Some(w) = app.get_webview_window("waypointer") {
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.with_webview(|webview| {
            use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
            use gtk::cairo::{RectangleInt, Region};
            use gtk_layer_shell::{KeyboardMode, LayerShell};

            let Some(toplevel) = webview.inner().toplevel() else { return };
            let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

            // Fullscreen input + exclusive keyboard so Escape works.
            let full = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767));
            gtk_window.input_shape_combine_region(Some(&full));
            gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
            gtk_window.show_all();
            gtk_window.queue_draw();

            log::info!(
                "waypointer::show: title={:?} alloc={}x{} visible={} mapped={} realized={}",
                gtk_window.title().map(|t| t.to_string()),
                gtk_window.allocated_width(),
                gtk_window.allocated_height(),
                gtk_window.is_visible(),
                gtk_window.is_mapped(),
                gtk_window.is_realized(),
            );

            if let Some(display) = gtk::gdk::Display::default() {
                display.flush();
            }
        });
        let _ = app.emit("lunaris://waypointer-show", ());

        // Focus the input immediately -- no delay. The DOM element persists
        // across show/hide cycles so it is always available.
        let _ = w.eval(
            "document.querySelector('[data-slot=\"command-input\"]')?.focus()"
        );

        // Clear the input value after a short delay (DOM needs one
        // frame after show_all to accept mutations). Focus is already
        // set above so keystrokes land in the input immediately.
        let w2 = w.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            let _ = w2.eval(
                "(() => { \
                   const i = document.querySelector('[data-slot=\"command-input\"]'); \
                   if (i) { i.value = ''; i.dispatchEvent(new Event('input', {bubbles:true})); } \
                 })()"
            );
        });

        log::info!("waypointer: shown");
    }
}

/// Hides the Waypointer overlay and zeros the input region.
pub fn hide(app: &AppHandle) {
    if !WAYPOINTER_VISIBLE.load(Ordering::SeqCst) {
        return;
    }
    WAYPOINTER_VISIBLE.store(false, Ordering::SeqCst);

    if let Some(w) = app.get_webview_window("waypointer") {
        // Clear the input before hiding so there is no flash of stale
        // content when the window is shown again.
        let _ = w.eval(
            r#"(() => {
                const i = document.querySelector("input");
                if (i) { i.value = ""; i.dispatchEvent(new Event("input", {bubbles:true})); }
                document.dispatchEvent(new CustomEvent("waypointer-reset"));
            })()"#,
        );

        let _ = w.with_webview(|webview| {
            use gtk::prelude::{Cast, WidgetExt};
            use gtk::cairo::{RectangleInt, Region};
            use gtk_layer_shell::{KeyboardMode, LayerShell};

            let Some(toplevel) = webview.inner().toplevel() else { return };
            let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

            // Zero input region + release keyboard.
            let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
            gtk_window.input_shape_combine_region(Some(&empty));
            gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
            gtk_window.queue_draw();
            if let Some(display) = gtk::gdk::Display::default() {
                display.flush();
            }
        });
        let _ = w.hide();
        let _ = app.emit("lunaris://waypointer-hide", ());
        log::info!("waypointer: hidden");
    }
}

/// Toggles the Waypointer overlay visibility.
pub fn toggle(app: &AppHandle) {
    if WAYPOINTER_VISIBLE.load(Ordering::SeqCst) {
        hide(app);
    } else {
        show(app);
    }
}

/// Returns whether the Waypointer overlay is currently visible.
pub fn is_visible() -> bool {
    WAYPOINTER_VISIBLE.load(Ordering::SeqCst)
}

/// Tauri command: toggle the Waypointer overlay.
#[tauri::command]
pub fn toggle_waypointer(app: AppHandle) {
    toggle(&app);
}
