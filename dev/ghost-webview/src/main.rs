// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Does the SHIPPED webview damage what a shrinking element vacates, on a real
//! layer surface?
//!
//! PR-20's ghost has now survived four measurements that each cleared a suspect:
//! `ghost-repro` showed the compositor repairs the region a shrinking layer
//! surface gives up, and `webkit-damage-probe.py` showed a shrinking element
//! inside an unchanging surface comes back clean, opaque and transparent. None of
//! them is the shipped configuration.
//!
//! Two gaps, and this closes both:
//!
//!   * **The engine.** Tauri on Linux links webkit2gtk-4.1 on GTK 3. The Python
//!     probe is WebKit 6.0 on GTK 4, which is the only pairing with a typelib on
//!     this host. Same project, different build, and a damage bug is exactly the
//!     sort of thing that differs between two builds.
//!   * **The surface role.** That probe used a toplevel. The waypointer is a
//!     fullscreen layer surface anchored to all four edges, which is what this
//!     one creates.
//!
//! No input, deliberately: `shoot-compositor.sh` records that injecting into a
//! nested surface under Xvfb is unsolved, so anything needing a keystroke cannot
//! be measured headlessly at all. The page shrinks itself on a timer, the same
//! trick `ghost-repro` uses.
//!
//! Run it under the nested compositor:
//!
//! ```text
//! cargo build --manifest-path dev/ghost-webview/Cargo.toml
//! dev/screenshot/shoot-compositor.sh /tmp/ghost-webview.png \
//!   dev/ghost-webview/target/debug/arlen-ghost-webview 1200
//! ```
//!
//! The argument is how long to hold the big block before shrinking it, in ms; it
//! must be shorter than the harness settle or the capture lands on the block and
//! the shrink is never photographed.
//!
//! Reading the result: the block is magenta, a colour in no theme, and it prints
//! its own geometry after the shrink so the capture is compared against numbers
//! rather than judged by eye. Magenta filling the block's OLD bounds is the ghost;
//! magenta at the new bounds and desktop everywhere else is a clean repaint.

use gtk::prelude::*;
use gtk_layer_shell::{Edge, Layer, LayerShell};
use webkit2gtk::{SettingsExt, WebView, WebViewExt};

/// Big then small, sharing a top-left corner so a stale region is attributable to
/// the old bounds rather than to "something magenta is on screen".
const PAGE: &str = r#"<!doctype html>
<html><body style="margin:0;background:transparent">
  <div id="b" style="position:absolute;left:120px;top:120px;
       width:520px;height:520px;background:#ff00ff"></div>
</body></html>"#;

/// The waypointer's own shape, which is the mode that was still missing.
///
/// Six configurations came back clean before this one, and the only difference
/// left between the probe and the thing that ghosts was the page. That page is
/// not a plain div: `WaypointerContent.svelte` puts the card inside a fixed
/// full-viewport backdrop, animates both on open (`wp-backdrop-fade` and
/// `wp-fade-in`, 150ms ease-out `both`), and the card's animation moves
/// `transform: scale() translateY()` as well as opacity. An animated transform
/// is what promotes an element to its own compositing layer, and a compositing
/// layer is repainted by a different path from a plain repaint - so this is the
/// one remaining place the damage could go missing.
///
/// Reproduced structurally rather than copied: a fixed inset-0 backdrop, a
/// centred card with the same two animations, and a list inside it that shrinks.
const PAGE_ANIMATED: &str = r#"<!doctype html>
<html><body style="margin:0;background:transparent">
  <div id="backdrop" style="position:fixed;inset:0;display:flex;
       justify-content:center;align-items:flex-start;padding-top:25vh;
       background:rgba(0,0,0,0.4);overflow:hidden;
       animation:fade 150ms ease-out both">
    <div id="card" style="position:relative;width:100%;max-width:600px;
         border-radius:12px;overflow:hidden;background:#ff00ff;
         animation:pop 150ms ease-out both">
      <div id="list" style="height:520px"></div>
    </div>
  </div>
  <style>
    @keyframes fade { from { opacity: 0 } to { opacity: 1 } }
    @keyframes pop {
      from { opacity: 0; transform: scale(0.98) translateY(-4px) }
      to   { opacity: 1; transform: scale(1) translateY(0) }
    }
  </style>
</body></html>"#;

/// The animated page shrinks its LIST, which is what filtering does - the card
/// follows, and the strip the card gives up is the region under test.
const SHRINK_LIST_AND_REPORT: &str = r#"
  document.getElementById('list').style.height = '140px';
  requestAnimationFrame(() => {
    const r = document.getElementById('card').getBoundingClientRect();
    const d = window.devicePixelRatio;
    console.log('after shrink: ' + JSON.stringify({
      dpr: d,
      css: [r.left, r.top, r.width, r.height],
      device: [r.left * d, r.top * d, r.width * d, r.height * d],
      viewport: [innerWidth * d, innerHeight * d],
    }));
  });
"#;

/// Shrinks the block, then reports where it ended up. The reporting half is not
/// decoration: the first run of the Python probe produced a rectangle that was
/// equally consistent with the shrunken block and with the big one clipped by the
/// window, and that is a judgement about a screenshot rather than a measurement.
const SHRINK_AND_REPORT: &str = r#"
  const b = document.getElementById('b');
  b.style.width = '140px';
  b.style.height = '140px';
  requestAnimationFrame(() => {
    const r = b.getBoundingClientRect();
    const d = window.devicePixelRatio;
    console.log('after shrink: ' + JSON.stringify({
      dpr: d,
      css: [r.left, r.top, r.width, r.height],
      device: [r.left * d, r.top * d, r.width * d, r.height * d],
      viewport: [innerWidth * d, innerHeight * d],
    }));
  });
"#;

fn main() {
    let hold_ms: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_200);
    let animated = std::env::args().nth(2).as_deref() == Some("animated");

    gtk::init().expect("gtk init");

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    // All four edges, so the compositor assigns the full output size and the
    // surface never resizes - which is the waypointer's shape and the reason the
    // vacated strip is INSIDE the surface, where damage is the client's to report.
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);

    // Transparent, like the waypointer: a surface composited by blending rather
    // than replacement is the case where "repainted" and "repainted with nothing"
    // stop being the same thing.
    window.set_app_paintable(true);
    if let Some(screen) = WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    let view = WebView::new();
    view.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    view.load_html(if animated { PAGE_ANIMATED } else { PAGE }, None);
    window.add(&view);
    window.show_all();

    // The page's console goes to the process's own output, which the harness
    // captures with SHOOT_CLIENT_LOG - otherwise the geometry the page reports
    // would go nowhere and the capture would be back to being judged by eye.
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_enable_write_console_messages_to_stdout(true);
    }

    let view_for_timer = view.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(hold_ms.into()), move || {
        // `evaluate_javascript` rather than the deprecated `run_javascript`: the
        // host ships WebKitGTK 2.52 and the old call has been deprecated since
        // 2.40, so using it would be writing a warning into a brand-new file.
        view_for_timer.evaluate_javascript(
            if animated { SHRINK_LIST_AND_REPORT } else { SHRINK_AND_REPORT },
            None,
            None,
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
        eprintln!("ghost-webview: shrank the block");
    });

    // Held open so the capture lands after the change and the surface is still
    // alive; dropping it would take the evidence with it.
    glib::timeout_add_seconds_local_once(120, gtk::main_quit);
    gtk::main();
}
