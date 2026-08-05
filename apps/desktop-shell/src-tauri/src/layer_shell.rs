// SPDX-License-Identifier: GPL-3.0-only

/// Initialises the Tauri window as a full-screen wlr-layer-shell surface on the
/// Overlay layer with a 36px exclusive zone at the top.
///
/// All four anchors are active (top + left + right + bottom) so the surface
/// covers the entire output. The compositor controls both dimensions.
/// Only the top 36px receives pointer input; everything below is transparent
/// and click-through (`input_shape_combine_region`).
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

        // `Layer::Top` (not `Overlay`). The Waypointer lives on
        // `Layer::Overlay` — keeping both on the same layer leaves
        // their stack order implementation-defined in wlr-layer-
        // shell, and in practice the main shell often painted on
        // top, visually clipping the fullscreen launcher behind the
        // 36px bar. Layer::Top still sits above regular xdg-toplevel
        // windows (so the bar, context menus, window headers, toasts
        // and other shell overlays still float over apps) but lets
        // the Waypointer's Overlay layer reliably cover everything
        // when it's open, without having to negotiate a compositor-
        // level grab.
        gtk_window.set_layer(Layer::Top);
        gtk_window.set_anchor(Edge::Top, true);
        gtk_window.set_anchor(Edge::Left, true);
        gtk_window.set_anchor(Edge::Right, true);
        gtk_window.set_anchor(Edge::Bottom, true);
        gtk_window.set_exclusive_zone(36);
    })?;

    // present() flushes all pending GTK/GDK Wayland requests synchronously so
    // the compositor receives the layer_surface role before the surface is mapped.
    // window.show() goes through Tauri/wry and does not guarantee flush order.
    window.with_webview(|webview| {
        use gtk::prelude::{Cast, GtkWindowExt, WidgetExt};
        if let Some(toplevel) = webview.inner().toplevel() {
            if let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() {
                // Compositor controls both dimensions via all-four-anchor configure.
                gtk_window.set_size_request(-1, -1);
                // show_all() recursively shows all child widgets (including the WebView)
                // and triggers GTK to commit actual buffer content to the surface.
                gtk_window.show_all();
                // Restrict pointer input to the top 36px bar. Everything below is
                // transparent and click-through.
                {
                    use gtk::cairo::{RectangleInt, Region};
                    let bar = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 36));
                    gtk_window.input_shape_combine_region(Some(&bar));
                }
                // Clear the window to nothing before its children draw.
                //
                // A transparent window is `app_paintable`, which in GTK3's drawing
                // model means GTK stops painting the background and the application
                // owns it. Nothing owned it. The webview paints where it has
                // content, so an overlay APPEARS correctly and then, when it goes
                // away, nobody paints over the area it occupied and its last frame
                // stays on the desktop. Measured on the image: the consent card
                // answered by a click and the Quick Settings panel dismissed by
                // Escape both freeze part-way through their fade, while a tooltip
                // shown at the same moment paints crisply. Every shell overlay had
                // this, not one of them.
                //
                // `Operator::Source` REPLACES rather than blends, so the surface is
                // reset to fully transparent instead of having transparent painted
                // over stale pixels, which composites to no change at all. Then back
                // to the default operator so the children blend normally.
                {
                    use gtk::cairo::Operator;
                    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
                    // Say so once. A clear that never runs and a clear that runs
                    // and does not help look identical from outside, and only one
                    // of them says the hypothesis is wrong.
                    static ANNOUNCED: AtomicBool = AtomicBool::new(false);
                    static BIG_DRAWS: AtomicUsize = AtomicUsize::new(0);
                    gtk_window.connect_draw(|_, cr| {
                        if !ANNOUNCED.swap(true, Ordering::Relaxed) {
                            log::info!("layer_shell: window clear handler is running");
                        }
                        // The context is clipped to whatever was invalidated, so
                        // the clear can only reach that. Report the substantial
                        // redraws, capped, to see whether one covers the area an
                        // overlay vacated at the moment it closes - which is the
                        // difference between "nothing was invalidated" and
                        // "something was, and it did not include that area".
                        if let Ok((x1, y1, x2, y2)) = cr.clip_extents() {
                            let (w, h) = (x2 - x1, y2 - y1);
                            if w > 100.0 && h > 100.0 && BIG_DRAWS.fetch_add(1, Ordering::Relaxed) < 40
                            {
                                log::info!(
                                    "layer_shell: draw clip {:.0}x{:.0} at {:.0},{:.0}",
                                    w, h, x1, y1,
                                );
                            }
                        }
                        cr.set_operator(Operator::Source);
                        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
                        let _ = cr.paint();
                        cr.set_operator(Operator::Over);
                        glib::Propagation::Proceed
                    });
                }
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
