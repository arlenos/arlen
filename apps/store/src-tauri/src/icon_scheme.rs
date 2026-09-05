//! Serving the catalogue's icons to the webview over `icon://`.
//!
//! WHY THIS EXISTS. Debian's AppStream catalogue ships its icons as files, and
//! `09b-appstream.sh.chroot` stages them onto the image beside the metadata, so
//! every one of the 2531 components has a picture sitting on the same disk as the
//! window that wants to draw it. `store-backend`'s `catalog.rs` deliberately
//! prefers that local file over a remote URL - taking Flathub's URL would trade a
//! file already there for a network fetch - so the wire carries an absolute path.
//!
//! A webview cannot open an absolute path, and `IconTile` says so in its own doc:
//! it paints a gradient, an http(s) URL, or a monogram. So the backend handed over
//! something the frontend was documented as unable to paint, and the store showed
//! 2531 grey letters over a catalogue whose pictures were already there. Nothing
//! was lying; the seam was never routed. This is the route.
//!
//! Same shape as the shell's `module_scheme.rs`, and the same discipline: the path
//! comes from a URL, so it is checked rather than trusted.
//!
//! WHAT IS SERVED, and nothing else. A request must resolve, after symlinks, to a
//! file under one of `ICON_ROOTS`, and carry an image extension. Both halves
//! matter and neither is enough alone: the root check without canonicalisation is
//! defeated by `..`, and canonicalisation without the extension check would serve
//! any readable file that happened to land under the icon tree. The catalogue is
//! parsed from files the archive publishes, so a hostile path in it is not a
//! fantasy.

use std::path::{Component, Path, PathBuf};

use arlen_store_backend::StoreCard;
use tauri::http::{Request as HttpRequest, Response as HttpResponse, StatusCode};

/// The directories an icon may come from. `/var/lib` is where the image stages
/// the archive's cache; `/usr/share` is where a distribution package puts its own.
const ICON_ROOTS: [&str; 2] = ["/var/lib/swcatalog/icons", "/usr/share/swcatalog/icons"];

/// The extensions this will serve, with the content type each is answered as.
const TYPES: [(&str, &str); 4] = [
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
    ("svgz", "image/svg+xml"),
    ("jpg", "image/jpeg"),
];

/// Whether a path is one the catalogue's icons live under.
///
/// Takes the roots so the rule is testable without the image's directories, which
/// is the only way it gets tested on a laptop at all.
fn under_roots(path: &Path, roots: &[&str]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

/// The content type for a path, or `None` when it is not an image this serves.
fn content_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
}

/// Whether a path is free of `..`, checked before touching the filesystem.
///
/// `canonicalize` would also resolve `..`, but only for a path that EXISTS. A
/// request for `/var/lib/swcatalog/icons/../../../etc/shadow` that does not
/// resolve would fail on the read instead, and a check that only fires when the
/// target is missing is not a check.
fn has_no_parent_segments(path: &Path) -> bool {
    !path.components().any(|c| c == Component::ParentDir)
}

/// The file a request names, or `None` when it names something this will not serve.
///
/// Split from the handler so every refusal is testable without a webview.
pub fn resolve(uri: &str, roots: &[&str]) -> Option<PathBuf> {
    // `icon://localhost/var/lib/swcatalog/icons/...` on Linux and macOS, which is
    // the shape tauri gives a custom scheme there (Windows and Android use
    // `http://icon.localhost/...`; this app ships on neither).
    let parsed = tauri::Url::parse(uri).ok()?;
    if parsed.scheme() != "icon" {
        return None;
    }
    // The percent-decoded path, since a filename may hold a space or a plus.
    let decoded = percent_encoding::percent_decode_str(parsed.path())
        .decode_utf8()
        .ok()?;
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() || !has_no_parent_segments(&path) {
        return None;
    }
    if !under_roots(&path, roots) || content_type(&path).is_none() {
        return None;
    }
    // And again after symlinks, because a symlink INSIDE the icon tree pointing
    // out of it passes every check above.
    let real = std::fs::canonicalize(&path).ok()?;
    if !under_roots(&real, roots) || content_type(&real).is_none() {
        return None;
    }
    Some(real)
}

/// Rewrite a catalogue icon reference into something the webview can load.
///
/// A URL is already paintable and passes through. An absolute path under the icon
/// roots becomes an `icon://` URL. Anything else - a bare theme name, a path
/// somewhere else - returns `None`, so the tile falls back to its monogram rather
/// than to a broken image.
pub fn paintable(icon: Option<String>, roots: &[&str]) -> Option<String> {
    let icon = icon?;
    if icon.starts_with("http://") || icon.starts_with("https://")
        || icon.starts_with("linear-gradient(")
    {
        return Some(icon);
    }
    let path = Path::new(&icon);
    if !path.is_absolute() || !under_roots(path, roots) || content_type(path).is_none() {
        return None;
    }
    let encoded = percent_encoding::utf8_percent_encode(&icon, percent_encoding::CONTROLS);
    Some(format!("icon://localhost{encoded}"))
}

/// Rewrite one card's icon into something the webview can load.
///
/// Applied where a card leaves for the frontend rather than inside
/// `store-backend`, because the scheme belongs to this window: the daemon serves
/// other clients and a path is the right thing for it to say.
pub fn repaint(mut card: StoreCard) -> StoreCard {
    card.icon = paintable(card.icon.take(), &ICON_ROOTS);
    card
}

/// The same for a list, which is the catalogue read.
pub fn repaint_all(cards: Vec<StoreCard>) -> Vec<StoreCard> {
    cards.into_iter().map(repaint).collect()
}

/// Answer one `icon://` request.
pub fn handle(request: &HttpRequest<Vec<u8>>) -> HttpResponse<Vec<u8>> {
    let refused = |code: StatusCode| {
        HttpResponse::builder()
            .status(code)
            .body(Vec::new())
            .expect("a bodyless response always builds")
    };
    let Some(path) = resolve(&request.uri().to_string(), &ICON_ROOTS) else {
        return refused(StatusCode::FORBIDDEN);
    };
    let Some(kind) = content_type(&path) else {
        return refused(StatusCode::FORBIDDEN);
    };
    match std::fs::read(&path) {
        Ok(bytes) => HttpResponse::builder()
            .status(StatusCode::OK)
            .header("content-type", kind)
            // The catalogue's icons do not change while the app is open, and a
            // scrolling grid asks for the same tile repeatedly.
            .header("cache-control", "max-age=3600")
            .body(bytes)
            .expect("a response with a read body always builds"),
        Err(_) => refused(StatusCode::NOT_FOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOTS: [&str; 1] = ["/var/lib/swcatalog/icons"];

    #[test]
    fn a_path_outside_the_icon_roots_is_refused() {
        assert!(resolve("icon://localhost/etc/shadow", &ROOTS).is_none());
    }

    #[test]
    fn a_parent_segment_is_refused_before_the_filesystem_is_touched() {
        // The target does not exist, so a check that relied on `canonicalize`
        // would refuse this for the wrong reason and pass its sibling that does.
        assert!(resolve(
            "icon://localhost/var/lib/swcatalog/icons/../../../etc/shadow.png",
            &ROOTS
        )
        .is_none());
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused() {
        assert!(resolve("icon://localhost/var/lib/swcatalog/icons/notes.txt", &ROOTS).is_none());
    }

    #[test]
    fn another_scheme_is_refused() {
        assert!(resolve("file:///var/lib/swcatalog/icons/a.png", &ROOTS).is_none());
    }

    #[test]
    fn a_remote_icon_is_left_alone() {
        let url = "https://flathub.org/a.png".to_string();
        assert_eq!(paintable(Some(url.clone()), &ROOTS), Some(url));
    }

    #[test]
    fn a_local_icon_becomes_a_url_the_webview_can_load() {
        assert_eq!(
            paintable(
                Some("/var/lib/swcatalog/icons/debian-trixie-main/64x64/gimp.png".to_string()),
                &ROOTS
            ),
            Some("icon://localhost/var/lib/swcatalog/icons/debian-trixie-main/64x64/gimp.png".to_string())
        );
    }

    #[test]
    fn a_bare_theme_name_paints_no_picture() {
        // The third icon shape DEP-11 uses. There is no file to point at, so the
        // tile takes its monogram rather than a URL that would 404.
        assert_eq!(paintable(Some("gimp".to_string()), &ROOTS), None);
    }

    #[test]
    fn a_local_path_somewhere_else_paints_no_picture() {
        assert_eq!(paintable(Some("/home/tim/secret.png".to_string()), &ROOTS), None);
    }

    #[test]
    fn no_icon_stays_no_icon() {
        assert_eq!(paintable(None, &ROOTS), None);
    }
}
