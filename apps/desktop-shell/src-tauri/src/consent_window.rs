// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The consent card's own layer-shell window.
//!
//! It exists because of a measured defect, not a preference. The card used to
//! render inside the top bar, and the bar is mapped with no keyboard
//! interactivity; `set_main_keyboard_grab` flipped it to `Exclusive` when a
//! request arrived. That flip does not work. Measured 9 August against the real
//! compositor with `dev/ghost-webview`'s `kbflip` and `kbexcl` modes: a layer
//! surface mapped `Exclusive` receives an injected key, and the same surface
//! mapped `None` and flipped to `Exclusive` afterwards receives nothing. On the
//! booted image that showed up as a consent card that could be answered with the
//! mouse and could not be dismissed with Escape - `deny()` was never called at
//! all, because the keydown never reached the page.
//!
//! Clicks kept working throughout, and the reason is the same one that makes this
//! window the fix: pointer routing follows the input region, which is positional
//! and does update at runtime, while keyboard routing follows focus, which is
//! settled when the surface maps.
//!
//! So the card gets a window that is created `Exclusive` and shown by mapping it,
//! which is exactly how the waypointer works and why Escape has always worked
//! there. `system-dialog-plan.md` asks v1 for a shell-owned layer surface with
//! guaranteed exclusive input; the bar could not give that and this can.
//!
//! Not solved here, and named so nobody reads more into it: this is not
//! clickjack-proof. The plan's privileged compositor protocol is what makes the
//! surface unspoofable and unobscurable, and that is still design-research in the
//! fork.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// The window label, used by every lookup here.
const LABEL: &str = "consent";

/// Creates the consent window, hidden, on its own route.
///
/// Hidden at build time rather than shown-and-emptied: a mapped surface that
/// merely has nothing in it still holds whatever focus its interactivity asks
/// for, and a consent surface that quietly owns the keyboard while no request is
/// pending would be worse than the bug it replaces.
pub fn create_window(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("/consent".into()))
        .title("Consent")
        .visible(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .build()?;

    log::info!("consent_window: created (hidden)");
    Ok(())
}

/// Configures the window as a fullscreen overlay layer surface.
///
/// Must run on the GTK main thread, like the waypointer's equivalent.
pub fn init_layer_shell(window: WebviewWindow) {
    if let Err(e) = window.with_webview(|webview| {
        use gtk::prelude::{Cast, WidgetExt};
        use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

        let Some(toplevel) = webview.inner().toplevel() else {
            log::warn!("consent_window: toplevel is None");
            return;
        };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else {
            log::warn!("consent_window: downcast to gtk::Window failed");
            return;
        };

        // No RGBA visual is set here on purpose. An earlier version did, on the
        // theory that the surface was compositing opaque; the boot after it
        // measured `depth=Some(32)` on this window AND on the waypointer, which
        // sets no visual of its own, so Tauri already provides one and the calls
        // changed nothing. Removed rather than left in with a reason the
        // measurement had disproven.

        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            gtk_window.set_anchor(edge, true);
        }
        // No exclusive zone: a consent card must not reflow the desktop under it.
        gtk_window.set_exclusive_zone(-1);
        // Set BEFORE the surface is ever mapped. This single line is the fix; a
        // later flip is what did not work.
        gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);

        // Empty input region while hidden, so a stray click cannot land on a
        // surface the user cannot see.
        {
            use gtk::cairo::{RectangleInt, Region};
            let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
            gtk_window.input_shape_combine_region(Some(&empty));
        }

        // Visual depth and compositing, because the page has been exonerated by
        // measurement: it reports html and body transparent and the overlay at
        // exactly rgba(0,0,0,0.5), yet 88.6% of the frame is PURE black, and a 50%
        // dim over a wallpaper cannot be pure black. So the surface under the page
        // is what is black. A 32-bit visual means the surface can carry alpha; 24
        // means it cannot, whatever the page does. The waypointer is transparent
        // in the same frame, so this line exists to be compared against its own.
        log::info!(
            "consent_window::init_layer_shell: is_layer_window={} layer={:?} depth={:?}",
            gtk_window.is_layer_window(),
            gtk_window.layer(),
            WidgetExt::visual(&gtk_window).map(|v| v.depth()),
        );
    }) {
        log::error!("consent_window: init_layer_shell failed: {e}");
    }
}

/// Raises the consent surface for a pending request.
pub fn show(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else {
        log::warn!("consent_window::show: no consent window");
        return;
    };
    let _ = w.show();
    let _ = w.set_focus();
    let _ = w.with_webview(move |webview| {
        use gtk::cairo::{RectangleInt, Region};
        use gtk::prelude::{Cast, WidgetExt};
        use gtk_layer_shell::{KeyboardMode, LayerShell};

        let Some(toplevel) = webview.inner().toplevel() else { return };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

        let full = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767));
        gtk_window.input_shape_combine_region(Some(&full));
        // Re-asserted on every raise rather than trusted from creation: the
        // window is hidden and shown repeatedly, and each show maps the surface
        // afresh, which is precisely the moment interactivity is read.
        gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
        gtk_window.show_all();
        gtk_window.queue_draw();
    });
    log::info!("consent_window::show: raised");
}

/// Drops the consent surface once no request is pending.
pub fn hide(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let _ = w.with_webview(move |webview| {
        use gtk::cairo::{RectangleInt, Region};
        use gtk::prelude::{Cast, WidgetExt};
        use gtk_layer_shell::LayerShell;

        let Some(toplevel) = webview.inner().toplevel() else { return };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

        let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
        gtk_window.input_shape_combine_region(Some(&empty));
        // Keyboard interactivity is deliberately NOT lowered here. Unmapping the
        // surface releases the focus anyway, and leaving it Exclusive is what
        // makes the next show correct by construction.
        let _ = gtk_window.is_layer_window();
    });
    let _ = w.hide();
    log::info!("consent_window::hide: dropped");
}
