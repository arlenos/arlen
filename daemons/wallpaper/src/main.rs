//! `arlen-wallpaperd`: renders the configured wallpaper on the Background layer
//! of every output via `wlr-layer-shell`. The pixel work (decode + Fill/Zoom
//! compose) is the pure `decode` module; this binary is the Wayland client
//! ([`arlen_wallpaper::render`]) plus manifest loading.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arlen_wallpaper::config;
use arlen_wallpaper::manifest::WallpaperManifest;
use arlen_wallpaper::render::Wallpaper;
use arlen_wallpaper::schedule::TimeContext;
use wayland_client::Connection;

/// How long the manifest directory must go quiet before a reload. Long enough to
/// swallow one save's burst of inotify events, short enough that a picked
/// wallpaper still feels immediate.
const RELOAD_SETTLE: Duration = Duration::from_millis(150);

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(manifest) = load_manifest() else {
        tracing::info!("no wallpaper manifest configured; nothing to render");
        return;
    };
    let time = TimeContext::at_minute(minute_of_day());

    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("no wayland display: {e}");
            return;
        }
    };
    let (wallpaper, queue) = match Wallpaper::new(&conn, manifest, time) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("wallpaper init failed: {e}");
            return;
        }
    };
    tracing::info!("arlen-wallpaperd rendering");
    if let Err(e) = run(conn, queue, wallpaper) {
        tracing::error!("wallpaper event loop ended: {e}");
    }
}

/// Drive the Wayland queue AND the manifest watch in one event loop.
///
/// A single `blocking_dispatch` loop waits on the Wayland fd alone, so a
/// manifest written while the desktop is idle would not be noticed until some
/// unrelated Wayland event arrived - the wallpaper would appear to change at
/// random moments, which is worse than not changing at all. calloop lets the
/// loop wait on both.
fn run(
    conn: Connection,
    queue: wayland_client::EventQueue<Wallpaper>,
    mut wallpaper: Wallpaper,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: calloop::EventLoop<Wallpaper> = calloop::EventLoop::try_new()?;
    let handle = event_loop.handle();
    // The connection here must be the one `queue` came from. Opening a second
    // one polls a socket nothing ever writes to while the real queue sits
    // unread, so no output is ever announced, no surface is created and the
    // daemon renders an empty screen while logging that it is rendering.
    calloop_wayland_source::WaylandSource::new(conn, queue).insert(handle.clone())?;

    // The watcher runs on its own thread and hands changes over the loop's own
    // channel, which is what actually wakes the dispatch.
    let (tx, rx) = calloop::channel::channel::<()>();
    let _watcher = watch_manifest(tx);

    // Reload on the TRAILING edge of a burst. One save is many inotify events -
    // measured under the nested compositor, a single temp-and-rename produced a
    // dozen - and each would otherwise decode the image and repaint every
    // output. Waiting for the burst to go quiet also avoids the subtler bug: the
    // first event of the burst is the temp file appearing, so reloading on it
    // reads the manifest that is about to be replaced.
    //
    // The pending timer is REMOVED before a new one is inserted. Dropping the
    // token is not enough - a source stays registered without it, so the first
    // cut of this scheduled one reload per event and made the problem worse
    // rather than better (42 reloads for one change, measured).
    let timer_handle = handle.clone();
    let pending: std::rc::Rc<std::cell::RefCell<Option<calloop::RegistrationToken>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    handle.insert_source(rx, move |event, _, _: &mut Wallpaper| {
        if !matches!(event, calloop::channel::Event::Msg(())) {
            return;
        }
        if let Some(token) = pending.borrow_mut().take() {
            timer_handle.remove(token);
        }
        let slot = pending.clone();
        let inserted = timer_handle.insert_source(
            calloop::timer::Timer::from_duration(RELOAD_SETTLE),
            move |_, _, state: &mut Wallpaper| {
                slot.borrow_mut().take();
                match load_manifest() {
                    Some(m) => {
                        tracing::info!("wallpaper manifest changed; redrawing");
                        state.set_manifest(m, TimeContext::at_minute(minute_of_day()));
                    }
                    // A manifest that stopped loading leaves the current one on
                    // screen: the alternative is a bare desktop while the user
                    // fixes a typo.
                    None => tracing::warn!(
                        "wallpaper manifest changed but does not load; keeping the current one"
                    ),
                }
                calloop::timer::TimeoutAction::Drop
            },
        );
        match inserted {
            Ok(token) => *pending.borrow_mut() = Some(token),
            Err(e) => tracing::warn!("could not schedule the wallpaper reload: {e}"),
        }
    })?;

    event_loop.run(None, &mut wallpaper, |_| {})?;
    Ok(())
}

/// Whether `event` is a change to the manifest, rather than a read of it.
///
/// The reason this exists is a feedback loop, measured rather than guessed: the
/// watch is on the DIRECTORY, and the inotify mask `notify` requests includes
/// ACCESS - so reloading the manifest READ it, which produced another event,
/// which scheduled another reload, one every 150 ms for the life of the daemon.
/// The log showed a reload followed 0.1 ms later by its own event, six times
/// over and climbing.
///
/// So the filter is on two axes: the event must be a modification (create,
/// write, remove, rename) and it must name the manifest or the temp file it is
/// renamed from. An access, and any other file in `~/.config/arlen`, is not a
/// wallpaper change.
fn changes_manifest(event: &notify::Event, manifest: &Path) -> bool {
    use notify::EventKind;
    let modifying = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    );
    if !modifying {
        return false;
    }
    event.paths.iter().any(|p| {
        p == manifest
            || p.file_name().is_some_and(|n| {
                manifest
                    .file_name()
                    .is_some_and(|m| n.to_string_lossy().starts_with(&*m.to_string_lossy()))
            })
    })
}

/// Watch the manifest's DIRECTORY and signal `tx` on any change in it.
///
/// The directory rather than the file: Settings writes the manifest with a
/// temp-and-rename, which replaces the inode, so a watch on the file itself
/// would go deaf after the first change. Directory events are coarse - any file
/// in `~/.config/arlen` wakes it - and the handler reloads a small TOML, so
/// coarse is cheaper than clever here.
///
/// Returns the watcher, which must be kept alive; dropping it stops the watch.
fn watch_manifest(tx: calloop::channel::Sender<()>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let path = config::user_manifest_path()?;
    let dir = path.parent()?.to_path_buf();
    // The directory may not exist until Settings first writes a manifest, so
    // create it rather than silently never watching.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), "cannot watch for wallpaper changes: {e}");
        return None;
    }
    let manifest = path.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if changes_manifest(&event, &manifest) {
            let _ = tx.send(());
        }
    })
    .map_err(|e| tracing::warn!("wallpaper watcher unavailable: {e}"))
    .ok()?;
    watcher
        .watch(&dir, notify::RecursiveMode::NonRecursive)
        .map_err(|e| tracing::warn!(dir = %dir.display(), "cannot watch: {e}"))
        .ok()?;
    tracing::info!(dir = %dir.display(), "watching for wallpaper manifest changes");
    Some(watcher)
}

/// Load the wallpaper manifest through the shared resolver: the
/// `ARLEN_WALLPAPER_MANIFEST` override for testing, else the user's, else the
/// distro default. `None` when none of them loads, and the daemon renders
/// nothing.
///
/// This used to stop at the user path, so a machine with only the shipped
/// default rendered no wallpaper at all - the fallback was written and never
/// reached.
fn load_manifest() -> Option<WallpaperManifest> {
    config::active_manifest(
        std::env::var_os("ARLEN_WALLPAPER_MANIFEST").map(std::path::PathBuf::from),
        |path, e| tracing::warn!(path = %path.display(), "could not load wallpaper manifest: {e}"),
    )
}

/// Minutes since midnight for the time-of-day source selection. Uses UTC as a
/// stable approximation (a static wallpaper - the common case - has no
/// time-of-day variants, so this only affects a `[[variants]]` manifest; real
/// local-time/sun-time selection is the schedule refinement).
fn minute_of_day() -> u32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    ((secs % 86_400) / 60) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, EventKind, ModifyKind};

    fn event(kind: EventKind, path: &str) -> notify::Event {
        notify::Event {
            kind,
            paths: vec![std::path::PathBuf::from(path)],
            attrs: Default::default(),
        }
    }

    #[test]
    fn reading_the_manifest_is_not_a_change_to_it() {
        // The whole reason this filter exists: the watch is on the directory and
        // the mask includes ACCESS, so reloading read the file, which looked like
        // a change, which scheduled another reload - forever, every 150 ms.
        let manifest = Path::new("/home/u/.config/arlen/wallpaper.toml");
        assert!(!changes_manifest(
            &event(EventKind::Access(AccessKind::Read), "/home/u/.config/arlen/wallpaper.toml"),
            manifest
        ));
    }

    #[test]
    fn a_write_or_a_rename_of_the_manifest_is_a_change() {
        let manifest = Path::new("/home/u/.config/arlen/wallpaper.toml");
        assert!(changes_manifest(
            &event(EventKind::Modify(ModifyKind::Any), "/home/u/.config/arlen/wallpaper.toml"),
            manifest
        ));
        // Settings writes `wallpaper.toml.tmp` and renames it; the temp file's
        // events are the ones that arrive first.
        assert!(changes_manifest(
            &event(EventKind::Create(CreateKind::File), "/home/u/.config/arlen/wallpaper.toml.tmp"),
            manifest
        ));
    }

    #[test]
    fn another_file_in_the_same_directory_is_not_a_wallpaper_change() {
        // `~/.config/arlen` holds every app's config, so most of what happens in
        // there has nothing to do with the wallpaper.
        let manifest = Path::new("/home/u/.config/arlen/wallpaper.toml");
        for other in ["/home/u/.config/arlen/shell.toml", "/home/u/.config/arlen/ai.toml"] {
            assert!(
                !changes_manifest(&event(EventKind::Modify(ModifyKind::Any), other), manifest),
                "{other} must not trigger a wallpaper reload"
            );
        }
    }
}
