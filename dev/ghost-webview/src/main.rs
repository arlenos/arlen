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
//! The ghost modes need no input: the page shrinks itself on a timer, the same
//! trick `ghost-repro` uses. The `kbflip` and `kbexcl` modes below DO take a
//! keystroke, and that is now possible - `wtype` against the nested compositor's
//! own socket works, which retires the note that headless input was unsolved. It
//! was unsolved for ydotool, which injects at the evdev layer and lands in the
//! host session instead.
//!
//! Run it under the nested compositor:
//!
//! ```text
//! cargo build --manifest-path dev/ghost-webview/Cargo.toml
//! SHOOT_SKIP_XVFB=1 SHOOT_DISPLAY=:0 RUST_LOG=info \
//!   dev/screenshot/shoot-compositor.sh /tmp/ghost-webview.png \
//!   "$PWD/target/debug/arlen-ghost-webview" 1200
//! ```
//!
//! Three things in that line are not decoration, each one measured after a run
//! failed on it. The binary is in the REPO's `target/`, not the crate's, because
//! `.cargo/config.toml` sets a shared `target-dir` that reaches even a crate with
//! its own workspace; the path this comment used to give has never existed. It
//! must be absolute, since the harness runs the client from another directory.
//! And cosmic-comp cannot run on Xvfb at all - its X11 backend wants DRI3 and its
//! winit fallback wants `EGL_EXT_device_drm` - so it needs a DRM-capable X server,
//! which on a Wayland host means XWayland at `:0`. `RUST_LOG` must leave `info`
//! on, because the harness learns the compositor's socket name by reading it out
//! of the log, and filtering that line makes a healthy run report as a dead one.
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

/// Does a layer surface that flips to exclusive keyboard interactivity AFTER it is
/// mapped actually receive keyboard focus?
///
/// This is the shell's consent card in miniature. The bar maps with
/// `KeyboardMode::None` and `set_main_keyboard_grab` flips it to `Exclusive` when a
/// request arrives (`shell_overlay_client.rs:1110`); measured on the booted image,
/// Escape then never reaches the page and the request is never denied. The
/// waypointer, which is mapped exclusive from the start, takes Escape fine.
///
/// **Answered, 9 August: it does not.** Run against the same compositor with the
/// same injector, `kbexcl` (mapped exclusive) logs `KEY RECEIVED keyval=Key(120)`
/// and `kbflip` (flipped after mapping) logs nothing. The compositor's own focus
/// log says nothing either way, which is why the key had to be the measurement:
/// asking whether a key ARRIVES beats asking what the compositor says about it.
///
///   SHOOT_SKIP_XVFB=1 SHOOT_DISPLAY=:0 SHOOT_CLIENT2='sleep 5; wtype x' \
///     RUST_LOG=info dev/screenshot/shoot-compositor.sh /tmp/kb.png \
///     "$PWD/target/debug/arlen-ghost-webview" 2000 kbflip
///
/// `exclusive_from_start` is the CONTROL, and without it this probe proves
/// nothing. "No key arrived" is equally consistent with "the flip does not carry
/// focus" and with "the injector never reached this compositor", and those are
/// opposite conclusions. The control maps exclusive at once - the waypointer's
/// shape, the case known to work on the booted image - so a key must arrive. If
/// it does not, the probe is broken and says so instead of indicting the code.
fn run_keyboard_flip(hold_ms: u32, exclusive_from_start: bool) {
    use gtk_layer_shell::KeyboardMode;

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);
    // Mapped WITHOUT keyboard interactivity, which is the whole point: a surface
    // born exclusive is the case that already works.
    window.set_keyboard_mode(if exclusive_from_start {
        KeyboardMode::Exclusive
    } else {
        KeyboardMode::None
    });

    // Real content, because an empty GTK window is not the thing under test. The
    // interactivity change only reaches the compositor on a surface commit, and a
    // window with nothing to paint may never produce one - so a probe that stayed
    // empty could report "the compositor ignored the flip" when the flip was never
    // sent. The shell's bar has a webview in it and always paints.
    let label = gtk::Label::new(Some("kbflip"));
    window.add(&label);

    // The actual question, asked directly. Whether the compositor logs a focus
    // change is a proxy; whether a key ARRIVES is the thing the consent card needs
    // and the thing that is failing on the booted image. Send one with
    // `wtype` against this compositor's socket after the flip and read the answer
    // here: a line means the runtime flip works and the shell's problem is
    // elsewhere, silence means the flip does not carry focus.
    window.connect_key_press_event(|_, ev| {
        eprintln!("kbflip: KEY RECEIVED keyval={:?}", ev.keyval());
        gtk::glib::Propagation::Proceed
    });

    window.show_all();
    eprintln!(
        "kbflip: mapped with KeyboardMode::{}",
        if exclusive_from_start { "Exclusive (control)" } else { "None" }
    );

    if !exclusive_from_start {
        let window_for_timer = window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(hold_ms.into()), move || {
            window_for_timer.set_keyboard_mode(KeyboardMode::Exclusive);
            eprintln!("kbflip: set KeyboardMode::Exclusive");
        });
    }

    glib::timeout_add_seconds_local_once(120, gtk::main_quit);
    gtk::main();
}

fn main() {
    let hold_ms: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_200);
    let mode = std::env::args().nth(2);
    let animated = mode.as_deref() == Some("animated");

    if matches!(mode.as_deref(), Some("kbflip") | Some("kbexcl")) {
        gtk::init().expect("gtk init");
        run_keyboard_flip(hold_ms, mode.as_deref() == Some("kbexcl"));
        return;
    }

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
