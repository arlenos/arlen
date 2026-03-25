// SPDX-License-Identifier: GPL-3.0-only

/// Initialises the Tauri window as a wlr-layer-shell surface anchored to the
/// top edge of the active output with a 28px exclusive zone.
///
/// Must be called in Tauri's `setup` callback after the window is realised but
/// before it is shown (`"visible": false` in tauri.conf.json guarantees this).
pub fn init(window: tauri::WebviewWindow) -> Result<(), tauri::Error> {
    log::info!("layer_shell::init called");

    window.with_webview(|webview| {
        use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
        use gtk_layer_shell::{Edge, Layer, LayerShell};

        // webview.inner() returns webkit2gtk::WebView (type inferred, not named).
        // webkit2gtk::WebView implements gtk::IsA<gtk::Widget> so WidgetExt applies.
        let Some(toplevel) = webview.inner().toplevel() else {
            log::info!("layer_shell: toplevel is None");
            return;
        };
        log::info!("layer_shell: toplevel found");

        log::info!(
            "layer_shell: toplevel GTK type = {}",
            glib::prelude::ObjectExt::type_(&toplevel).name()
        );
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else {
            log::info!("layer_shell: downcast to gtk::Window failed");
            return;
        };
        log::info!(
            "layer_shell: gtk_window type = {}",
            glib::prelude::ObjectExt::type_(&gtk_window).name()
        );

        let display = gtk::gdk::Display::default();
        log::info!("layer_shell: GDK display = {:?}", display.map(|d| d.name()));

        log::info!("layer_shell: window is mapped = {}", gtk_window.is_mapped());
        log::info!("layer_shell: window is realized = {}", gtk_window.is_realized());
        gtk_window.connect_realize(|w| {
            log::info!("layer_shell: realize signal fired");
            use gtk::prelude::WidgetExt;
            w.queue_draw();
        });
        gtk_window.init_layer_shell();
        log::info!("layer_shell: init_layer_shell called");
        log::info!("layer_shell: gtk_layer_shell version = {}", gtk_layer_shell::major_version());
        log::info!("layer_shell: is_layer_window = {}", gtk_window.is_layer_window());
        log::info!("layer_shell: layer = {:?}", gtk_window.layer());

        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_anchor(Edge::Top, true);
        gtk_window.set_anchor(Edge::Left, true);
        gtk_window.set_anchor(Edge::Right, true);
        gtk_window.set_exclusive_zone(36);
        // Width is stretched automatically by left+right anchors; 1 is the minimum
        // value GTK accepts (gtk_window_resize asserts width > 0).
        gtk_window.set_default_size(1, 36);
    })?;

    // present() flushes all pending GTK/GDK Wayland requests synchronously so
    // the compositor receives the layer_surface role before the surface is mapped.
    // window.show() goes through Tauri/wry and does not guarantee flush order.
    window.with_webview(|webview| {
        use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
        if let Some(toplevel) = webview.inner().toplevel() {
            if let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() {
                // Remove width constraint so the compositor can set the full output
                // width via the layer-shell configure event. Height is fixed at 28px.
                gtk_window.set_size_request(-1, 36);
                // show_all() recursively shows all child widgets (including the WebView)
                // and triggers GTK to commit actual buffer content to the surface.
                gtk_window.show_all();
                log::info!("layer_shell: window shown via gtk show_all");
                gtk_window.queue_draw();
                if let Some(display) = gtk::gdk::Display::default() {
                    display.flush();
                    log::info!("layer_shell: GDK display flushed");
                }
                // After ack_configure the compositor expects a wl_surface.commit with
                // actual content. GTK does not redraw automatically here, so queue a
                // draw on the next idle tick (after the configure event is processed).
                let win_clone = gtk_window.clone();
                glib::idle_add_local_once(move || {
                    use gtk::prelude::WidgetExt;
                    win_clone.queue_draw();
                    log::info!("layer_shell: queue_draw issued");
                });
            }
        }
    })?;
    Ok(())
}
