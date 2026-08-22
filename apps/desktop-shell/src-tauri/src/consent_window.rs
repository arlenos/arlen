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

        // No RGBA visual: measured `depth=Some(32)` here and on the waypointer,
        // which sets none of its own, so Tauri already provides one.
        //
        // `app_paintable` IS set, and separating it from that visual is the point.
        // The two went in together and came out together, so the depth
        // measurement only ever spoke to the visual. This is the other half: with
        // it false, GTK paints its own theme ground and tells the compositor the
        // surface is opaque, and an opaque surface is composited by replacement -
        // which would put pure black under a 50% dim exactly as measured, whatever
        // depth the visual has.
        gtk_window.set_app_paintable(true);

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

        // NOT armed from the raise. A surface that has not painted holds no
        // input and answers nothing - the rule is ordering, not speed, and it
        // does not depend on how large the gap happens to be.
        //
        // Why the gap is real, measured on 22 August rather than argued:
        //
        //   raise                    card on screen        after-paint says
        //   13.88s                   about +3s             4ms
        //   77.82s (second raise)    about +2.5 to +5s     1025ms
        //   13.80s (compositing on)  about +1.4s           9ms
        //
        // So GTK's frame clock reports a painted CONTAINER milliseconds after the
        // raise while the card inside it is seconds away, on both clocks, and the
        // delta is not even stable between two raises in one boot. Accelerated
        // compositing halves the gap and does not close it, so it is not purely
        // the VM's software rendering either. There is no signal here that means
        // pixels, which is why this arms from the PAGE instead: `consent_ready`
        // is invoked by the card's own component after its layout and two
        // animation frames, and that is the latest honest moment this process can
        // observe.
        //
        // The 21 August attempt at this had a 2s backstop that armed anyway, and
        // the backstop is what fired - `raised` at 131.2s, `armed` at 133.2s. So
        // the escape hatch was part of what failed. There is none now: a card that
        // never reports leaves the request unanswered, which is the fail-closed
        // direction and what the rule prescribes.
        //
        // AND THIS IS STILL NOT ENOUGH, measured on 22 August with the change in:
        // `arm` fires 37ms after the raise while the card reaches the screen a
        // second or more later, and a click driven at raise+1.2s was taken as the
        // answer with the frame at that instant showing no card. The rAF pair
        // completes as soon as GTK ticks a frame, and GTK ticks in 4ms.
        //
        // So the ordering rule is NOT satisfied by this shape alone. Three
        // candidate signals have now been measured and none of them means pixels
        // - the page's DOM report, both frame clocks, and this readiness report.
        // Closing it needs a structural answer rather than a better callback:
        // a surface that stays mapped so nothing has a first frame to wait for,
        // or one created per request so the load path is observable. That is a
        // change to a security surface and is recorded for the planner rather
        // than guessed at here.
        let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
        gtk_window.input_shape_combine_region(Some(&empty));
        // Re-asserted on every raise rather than trusted from creation: the
        // window is hidden and shown repeatedly, and each show maps the surface
        // afresh, which is precisely the moment interactivity is read.
        gtk_window.set_keyboard_mode(KeyboardMode::None);
        gtk_window.show_all();
        gtk_window.queue_draw();
        // The WEBVIEW is redrawn too, not only its GTK container. A second
        // request queued while the surface was down comes up mapped and blank
        // otherwise: measured on 21 August, `show: raised` at 131.2s and the
        // frame at 134.7s had no card on it while the surface was taking every
        // press on the desktop. An invisible modal is worse than a visible one,
        // so the redraw is asked for at the one place that knows the surface
        // just came back.
        webview.inner().queue_draw();

        // WHEN this surface actually presents, measured rather than assumed.
        //
        // A click driven 1.1s after the raise was taken as the answer while the
        // frame at that instant showed a plain desktop, so the ordering rule -
        // paint, then take input - needs a signal that means PIXELS. The page's
        // own report does not: it fires on a DOM update and was thrown away for
        // that reason (see above). These two lines say which of the remaining
        // candidates does. The toplevel and the webview each have their own frame
        // clock, and if the card is WebKit content presenting on its own surface
        // they will not agree; the gap between them is the thing to read.
        //
        // Once per raise: the handler disconnects itself on the first tick.
        for (what, widget) in [
            ("toplevel", gtk_window.clone().upcast::<gtk::Widget>()),
            ("webview", webview.inner().clone().upcast::<gtk::Widget>()),
        ] {
            let Some(clock) = WidgetExt::frame_clock(&widget) else {
                log::info!("consent_window::show: {what} has no frame clock");
                continue;
            };
            let started = std::time::Instant::now();
            let id: std::rc::Rc<std::cell::RefCell<Option<gtk::glib::SignalHandlerId>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            let take = id.clone();
            let handler = clock.connect_after_paint(move |c| {
                if let Some(h) = take.borrow_mut().take() {
                    log::info!(
                        "consent_window::show: {what} first after-paint {}ms after raise",
                        started.elapsed().as_millis()
                    );
                    gtk::glib::signal::signal_handler_disconnect(c, h);
                }
            });
            *id.borrow_mut() = Some(handler);
        }
    });
    log::info!("consent_window::show: raised");
}

/// Arms the surface: the card is on screen, so it may now take input.
///
/// Called by the consent page once its card has laid out and two animation
/// frames have gone by. This is the ordering rule's second half - `show` maps a
/// surface that swallows nothing, and only this makes it answerable.
///
/// Idempotent: a second call re-asserts the same region, which is what a re-raise
/// of an already-armed surface needs anyway.
pub fn arm(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else {
        log::warn!("consent_window::arm: no consent window");
        return;
    };
    let _ = w.with_webview(move |webview| {
        use gtk::cairo::{RectangleInt, Region};
        use gtk::prelude::{Cast, WidgetExt};
        use gtk_layer_shell::{KeyboardMode, LayerShell};

        let Some(toplevel) = webview.inner().toplevel() else {
            return;
        };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else {
            return;
        };
        let full = Region::create_rectangle(&RectangleInt::new(0, 0, 32767, 32767));
        gtk_window.input_shape_combine_region(Some(&full));
        gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
    });
    log::info!("consent_window::arm: the card is up, input taken from here");
}

/// Drops the consent surface once no request is pending.
pub fn hide(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let _ = w.with_webview(move |webview| {
        use gtk::cairo::{RectangleInt, Region};
        use gtk::prelude::{Cast, WidgetExt};
        let Some(toplevel) = webview.inner().toplevel() else { return };
        let Ok(gtk_window) = toplevel.downcast::<gtk::Window>() else { return };

        let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
        gtk_window.input_shape_combine_region(Some(&empty));
        // KEYBOARD INTERACTIVITY IS LOWERED HERE, and the comment this replaces
        // said the opposite: that unmapping releases the focus anyway, so leaving
        // it Exclusive made the next show correct by construction. The first half
        // of that is not true on this compositor, and the whole desktop paid for
        // it - after a request was answered the top bar took hover and refused
        // every click, on the image, for weeks.
        //
        // What the machine said, once the frontend log carried a window label:
        //   [FRONTEND] [consent] first pointerdown at 1257,17 on DIV.fixed.inset-0
        //   COVERS THE SCREEN slot=dialog-overlay
        // A press aimed at the top bar was arriving at the CONSENT surface. The
        // empty input region above sends motion to the bar underneath (so the bar
        // hovers, which is why this read as a dead frontend for so long), while
        // the button press follows the keyboard focus this window never gave up.
        //
        // Hidden at the GTK level, not only through Tauri. `w.hide()` below left
        // the layer surface mapped, which is the best explanation for the boots
        // where an answered card stayed painted on screen.
        //
        // The keyboard mode is deliberately NOT touched here, and that is a
        // correction of my own change from earlier today: I lowered it to None
        // on the way out, which is both dead (interactivity is read when a
        // surface MAPS - see the note on the deleted `set_main_keyboard_grab`)
        // and unsafe, because `show` calls Tauri's `show()` before the closure
        // re-asserts Exclusive, so a lowered mode could be the one read at the
        // next map and the card would come up unable to take Escape.
        gtk_window.hide();
    });
    let _ = w.hide();
    log::info!("consent_window::hide: dropped");
}
