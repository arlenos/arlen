//! Serving the catalogue's icons to the webview over `icon://`.
//!
//! WHY THERE IS A SCHEME AT ALL. Debian's AppStream catalogue ships its icons as
//! files and `09b-appstream.sh.chroot` stages them onto the image, so every one of
//! the 2531 components has a picture on the same disk as the window that wants to
//! draw it. `store-backend` deliberately prefers that local file over a remote
//! URL (taking Flathub's URL would trade a file already there for a network
//! fetch), so a card's icon is an absolute path, and a webview cannot open a path.
//! Without a route every tile falls to its monogram, which is what the 3 September
//! image showed: a full catalogue of grey letters over pictures that were there.
//!
//! WHY THE WINDOW DOES NOT READ THE FILE, which is the part I got wrong first.
//! This app's permission profile has no `[filesystem]` section, and says why at
//! length: "the catalogue, the reviews, the remote and the icons are the
//! store-backend's to read", and "the only paths in this crate are the ones inside
//! the requests it forwards". A handler here that opened
//! `/var/lib/swcatalog/icons/...` would have made that profile false about its own
//! code, and it would have kept working, because confinement is off today, right
//! up until somebody turned it on. So the read stays in the backend, which is a
//! separate principal with its own reach, and this file goes back to what every
//! other command in the crate is: a proxy.
//!
//! WHAT TRAVELS. The URL carries a COMPONENT ID, never a path. The window
//! therefore has no roots, no traversal rule and no extension list - it cannot be
//! talked into reading a file because it never names one. `Request::Icon` resolves
//! the id against the catalogue and applies those rules where the catalogue is.

use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use tauri::http::{Request as HttpRequest, Response as HttpResponse, StatusCode};

use arlen_store_backend::{ComponentId, Request, Response, StoreCard};

/// The component id an `icon://` request names, or `None` when it names nothing
/// usable.
///
/// Split out so the URL half is testable without a socket.
pub fn requested_id(uri: &str) -> Option<String> {
    let parsed = tauri::Url::parse(uri).ok()?;
    if parsed.scheme() != "icon" {
        return None;
    }
    let id = percent_decode_str(parsed.path().trim_start_matches('/'))
        .decode_utf8()
        .ok()?
        .to_string();
    // An id is what the catalogue keys on; a slash would mean the URL was carrying
    // something with structure, which this op does not take.
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id)
}

/// Rewrite a card's icon into something the webview can load.
///
/// Three shapes arrive, because DEP-11 uses three. A URL is already paintable and
/// passes through. An absolute path has a file behind it, and becomes an `icon://`
/// URL naming this component - the path itself is dropped here rather than
/// forwarded, since the backend finds it again from the id and this crate has no
/// business carrying it. A bare theme name has no file, so it becomes `None` and
/// the tile draws its monogram at once rather than after a round trip that was
/// always going to come back empty.
pub fn paintable(id: &str, icon: Option<String>) -> Option<String> {
    let icon = icon?;
    if icon.starts_with("http://") || icon.starts_with("https://")
        || icon.starts_with("linear-gradient(")
    {
        return Some(icon);
    }
    if !icon.starts_with('/') {
        return None;
    }
    Some(format!(
        "icon://localhost/{}",
        utf8_percent_encode(id, NON_ALPHANUMERIC)
    ))
}

/// Rewrite one card's icon.
pub fn repaint(mut card: StoreCard) -> StoreCard {
    card.icon = paintable(&card.id, card.icon.take());
    card
}

/// The same for a list, which is the catalogue read.
pub fn repaint_all(cards: Vec<StoreCard>) -> Vec<StoreCard> {
    cards.into_iter().map(repaint).collect()
}

/// Answer one `icon://` request by asking the backend for the bytes.
pub async fn handle(request: HttpRequest<Vec<u8>>) -> HttpResponse<Vec<u8>> {
    let refused = |code: StatusCode| {
        HttpResponse::builder()
            .status(code)
            .body(Vec::new())
            .expect("a bodyless response always builds")
    };
    let Some(id) = requested_id(&request.uri().to_string()) else {
        return refused(StatusCode::BAD_REQUEST);
    };
    let asked = arlen_store_backend::request_default(&Request::Icon {
        id: ComponentId(id),
    })
    .await;
    match asked {
        Ok(Response::Icon(Some(icon))) => HttpResponse::builder()
            .status(StatusCode::OK)
            .header("content-type", icon.content_type)
            // The catalogue's icons do not change while the app is open and a
            // scrolling grid asks for the same tile repeatedly.
            .header("cache-control", "max-age=3600")
            .body(icon.bytes)
            .expect("a response with a read body always builds"),
        // No picture for this component, which is ordinary rather than a fault.
        // The tile's `onerror` takes it to a monogram.
        Ok(Response::Icon(None)) => refused(StatusCode::NOT_FOUND),
        // Anything else means the backend is not answering. Same outcome for the
        // reader - a letter instead of a picture - but it is logged, because a
        // whole catalogue of letters and a daemon that is down look identical on
        // screen and are not the same thing.
        Ok(other) => {
            log::warn!("icon: unexpected store response: {other:?}");
            refused(StatusCode::BAD_GATEWAY)
        }
        Err(e) => {
            log::warn!("icon: store backend unreachable: {e}");
            refused(StatusCode::BAD_GATEWAY)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_path_becomes_a_url_naming_the_component() {
        assert_eq!(
            paintable(
                "abiword",
                Some("/var/lib/swcatalog/icons/debian-trixie-main/64x64/abiword_abiword.png".into())
            )
            .as_deref(),
            Some("icon://localhost/abiword")
        );
    }

    #[test]
    fn the_path_does_not_travel() {
        // The point of going by id: nothing the catalogue says about a file
        // reaches the URL, so nothing the window composes can name one.
        let url = paintable("x", Some("/var/lib/swcatalog/icons/a.png".into())).expect("a url");
        assert!(!url.contains("swcatalog"), "{url}");
    }

    #[test]
    fn a_remote_icon_is_left_alone() {
        let url = "https://flathub.org/a.png".to_string();
        assert_eq!(paintable("x", Some(url.clone())), Some(url));
    }

    #[test]
    fn a_bare_theme_name_paints_no_picture() {
        // No file behind it, so the tile takes its monogram now rather than after
        // a round trip that was always going to come back empty.
        assert_eq!(paintable("gimp", Some("gimp".into())), None);
        assert_eq!(paintable("gimp", None), None);
    }

    #[test]
    fn a_url_this_emits_is_one_the_handler_reads_back() {
        // The joint. `paintable` writes and `requested_id` reads, and an id with
        // characters that need encoding is where they would disagree.
        let id = "org.gnome.Chess+extra";
        let url = paintable(id, Some("/var/lib/swcatalog/icons/a.png".into())).expect("a url");
        assert_eq!(requested_id(&url).as_deref(), Some(id));
    }

    #[test]
    fn a_request_carrying_structure_is_refused() {
        // An id is a key, not a path. Nothing downstream would read it as one,
        // and it is refused here so that stays true.
        assert_eq!(requested_id("icon://localhost/a%2Fb"), None);
        assert_eq!(requested_id("icon://localhost/"), None);
    }

    #[test]
    fn another_scheme_is_refused() {
        assert_eq!(requested_id("file:///var/lib/swcatalog/icons/a.png"), None);
    }
}
