//! `org.freedesktop.impl.portal.Settings` implementation.
//!
//! The one cross-toolkit channel that carries the person's appearance choice to
//! foreign apps without a per-release tax. A GTK4/libadwaita app, a Qt6 app,
//! Firefox and most terminals all read `color-scheme` off this interface, and
//! libadwaita 1.6 reads the accent off it in preference to any GNOME setting.
//! Everything else in `own-toolkit-themes-plan.md` is a fork somebody has to
//! re-cut when the toolkit moves; this is not, which is why it is first.
//!
//! Spec:
//! <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Settings.html>
//!
//! **What it exports, and what it deliberately does not.** The interface
//! documents four keys under `org.freedesktop.appearance`. Three of them have a
//! source in this tree and are served:
//!
//! * `color-scheme` - the resolved theme's variant. Exact, and the one that
//!   matters most: it is what makes a foreign app go dark with the rest of the
//!   desktop.
//! * `accent-color` - the resolved theme's accent, as sRGB in `[0,1]`, which is
//!   the same range `sdk/theme` already stores. Exact for Qt and CSS; libadwaita
//!   snaps it to the nearest of its nine named accents, so inside a libadwaita
//!   app it is approximate by construction and no amount of precision here
//!   changes that.
//! * `reduced-motion` - `appearance.toml [accessibility] reduce_motion`. Real,
//!   and honestly a hand-edit today: Settings ships the key in its default file
//!   but has no control that writes it, so the value a foreign app reads is
//!   whatever is in the file. Serving it costs nothing and makes the hand-edit
//!   work; not serving it would leave the key permanently absent instead.
//!
//! `contrast` is the fourth and is **not** served, because nothing in the tree
//! decides it. The accessibility page has an invert filter, which is a different
//! thing. A `Read` of an unserved key is an error by spec, the frontend then
//! answers its own default of 0, and that is the same answer we would give -
//! with the difference that we are not claiming to have measured it.
//!
//! **Read-only, and no request object.** Unlike the four siblings there is no
//! `Request` handle and nothing to cancel: two methods answering from files, and
//! one signal when those files change.

use std::collections::HashMap;

use arlen_theme::ArlenTheme;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Structure, Value};

use crate::interfaces::sender_is_frontend;

/// The only namespace this backend answers for.
pub const NAMESPACE: &str = "org.freedesktop.appearance";

const COLOR_SCHEME: &str = "color-scheme";
const ACCENT_COLOR: &str = "accent-color";
const REDUCED_MOTION: &str = "reduced-motion";

/// `color-scheme` values, from the interface documentation.
const SCHEME_NO_PREFERENCE: u32 = 0;
const SCHEME_DARK: u32 = 1;
const SCHEME_LIGHT: u32 = 2;

/// The sentinel for "no accent colour". The spec says an out-of-range component
/// is to be read as unset, and every component of this one is out of `[0,1]`.
/// It is what a caller gets when the person's theme files are present but do not
/// resolve - a broken choice is not a licence to invent a colour.
const ACCENT_UNSET: [f64; 3] = [-1.0, -1.0, -1.0];

/// Everything this backend can answer, resolved once.
///
/// Held together in one struct so the change signal can compare a whole reading
/// against the last one and emit only the keys that actually moved. A `Read` of
/// one key still resolves the lot, which is three small files and a TOML parse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    /// 0 no preference, 1 dark, 2 light.
    pub color_scheme: u32,
    /// sRGB in `[0,1]`, or [`ACCENT_UNSET`].
    pub accent: [f64; 3],
    /// 0 no preference, 1 reduced.
    pub reduced_motion: u32,
}

impl Appearance {
    /// Read the person's own files and answer from them.
    ///
    /// A missing file is not a failure - `resolve_active` answers the bundled
    /// dark theme for a machine where nobody has chosen anything, which is the
    /// same answer Settings gives. Only a file that is present and does not
    /// resolve lands in the error arm, and there the honest answer is "no
    /// preference" rather than a guess.
    pub fn current() -> Self {
        let appearance_text = std::fs::read_to_string(ArlenTheme::user_appearance_path()).ok();
        let reduced_motion = reduce_motion_from(appearance_text.as_deref());
        match ArlenTheme::resolve_active(None) {
            Ok(theme) => {
                let a = theme.accent_rgb();
                Self {
                    color_scheme: if theme.is_dark() { SCHEME_DARK } else { SCHEME_LIGHT },
                    accent: [a[0] as f64, a[1] as f64, a[2] as f64],
                    reduced_motion: u32::from(reduced_motion),
                }
            }
            Err(e) => {
                tracing::warn!(
                    "the theme does not resolve, so the appearance portal answers no preference: {e}"
                );
                Self {
                    color_scheme: SCHEME_NO_PREFERENCE,
                    accent: ACCENT_UNSET,
                    reduced_motion: u32::from(reduced_motion),
                }
            }
        }
    }

    pub(crate) fn value_for(&self, key: &str) -> Option<OwnedValue> {
        let value = match key {
            COLOR_SCHEME => Value::U32(self.color_scheme),
            REDUCED_MOTION => Value::U32(self.reduced_motion),
            ACCENT_COLOR => Value::Structure(Structure::from((
                self.accent[0],
                self.accent[1],
                self.accent[2],
            ))),
            _ => return None,
        };
        value.try_to_owned().ok()
    }

    fn as_map(&self) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        for key in [COLOR_SCHEME, ACCENT_COLOR, REDUCED_MOTION] {
            if let Some(v) = self.value_for(key) {
                map.insert(key.to_string(), v);
            }
        }
        map
    }

    /// The keys whose value differs from `other`, in a stable order.
    pub fn changed_keys(&self, other: &Self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.color_scheme != other.color_scheme {
            out.push(COLOR_SCHEME);
        }
        if self.accent != other.accent {
            out.push(ACCENT_COLOR);
        }
        if self.reduced_motion != other.reduced_motion {
            out.push(REDUCED_MOTION);
        }
        out
    }
}

/// `[accessibility] reduce_motion` out of `appearance.toml`, defaulting to off.
///
/// Pure, because every branch is a real state of that file: absent (a machine
/// Settings has never written), present without the section, and present with a
/// value of the wrong type. None of the three is a preference for reduced
/// motion, and all three used to be the kind of thing a `unwrap_or_default`
/// chain gets subtly wrong.
pub fn reduce_motion_from(appearance_toml: Option<&str>) -> bool {
    appearance_toml
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|t| {
            t.get("accessibility")?
                .as_table()?
                .get("reduce_motion")?
                .as_bool()
        })
        .unwrap_or(false)
}

/// Whether a `ReadAll` namespace pattern selects `ns`.
///
/// The spec allows one shape of glob and only one: a trailing section, as in
/// `org.example.*`. An empty pattern - or an empty array, which the caller
/// turns into one empty pattern - matches everything.
pub fn namespace_matches(pattern: &str, ns: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    match pattern.strip_suffix('*') {
        Some(prefix) => ns.starts_with(prefix),
        None => pattern == ns,
    }
}

/// The error a `Read` of something we do not serve answers with.
///
/// `org.freedesktop.portal.Error.NotFound` is what the reference backends
/// return, and the frontend keys its fallback off a failed call rather than off
/// the name - but a caller that logs the name should see the one the rest of the
/// portal world uses.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
pub enum SettingsError {
    /// No such namespace, or no such key inside it.
    NotFound(String),
}

/// The `org.freedesktop.impl.portal.Settings` backend.
///
/// Stateless: every answer is read from the person's files at call time, so an
/// edit is picked up even by a caller that arrives between the write and the
/// change signal.
#[derive(Clone, Default)]
pub struct Settings;

impl Settings {
    /// Build the interface.
    pub fn new() -> Self {
        Self
    }
}

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    /// Interface version.
    ///
    /// 1, and that is the whole of it: the interface on the frontend we build
    /// against has exactly `ReadAll`, `Read` and `SettingChanged`. `ReadOne`
    /// lives on the app-facing `org.freedesktop.portal.Settings`, where the
    /// frontend implements it by calling this `Read` and unwrapping.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    /// Every key in every namespace the caller asked for.
    async fn read_all(
        &self,
        namespaces: Vec<String>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> HashMap<String, HashMap<String, OwnedValue>> {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            tracing::warn!("refusing a Settings ReadAll from a sender that is not the portal frontend");
            return HashMap::new();
        }
        // An empty array matches all, per the spec, and so does an empty string
        // inside it - which `namespace_matches` already answers true for.
        let wanted = namespaces.is_empty()
            || namespaces.iter().any(|p| namespace_matches(p, NAMESPACE));
        let mut out = HashMap::new();
        if wanted {
            out.insert(NAMESPACE.to_string(), Appearance::current().as_map());
        }
        out
    }

    /// One key. An unknown namespace or key is an error, not an empty value.
    async fn read(
        &self,
        namespace: &str,
        key: &str,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> Result<OwnedValue, SettingsError> {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            tracing::warn!("refusing a Settings Read from a sender that is not the portal frontend");
            return Err(SettingsError::NotFound(
                "caller is not the xdg-desktop-portal frontend".to_string(),
            ));
        }
        if namespace != NAMESPACE {
            return Err(SettingsError::NotFound(format!(
                "no such namespace: {namespace}"
            )));
        }
        Appearance::current()
            .value_for(key)
            .ok_or_else(|| SettingsError::NotFound(format!("no such key: {namespace} {key}")))
    }

    /// Emitted when one of the keys above changes value.
    #[zbus(signal)]
    pub async fn setting_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        namespace: String,
        key: String,
        value: OwnedValue,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_pattern_matches_everything() {
        assert!(namespace_matches("", NAMESPACE));
        assert!(namespace_matches("", "com.example.anything"));
    }

    #[test]
    fn a_trailing_star_matches_the_prefix_and_nothing_else() {
        assert!(namespace_matches("org.freedesktop.*", NAMESPACE));
        assert!(namespace_matches("*", NAMESPACE));
        assert!(!namespace_matches("org.gnome.*", NAMESPACE));
        // The glob is trailing-only: a star in the middle is not a wildcard, so
        // this is an exact comparison that fails, not a match.
        assert!(!namespace_matches("org.*.appearance", NAMESPACE));
    }

    #[test]
    fn a_plain_pattern_is_an_exact_comparison() {
        assert!(namespace_matches(NAMESPACE, NAMESPACE));
        assert!(!namespace_matches("org.freedesktop", NAMESPACE));
        assert!(!namespace_matches("org.freedesktop.appearances", NAMESPACE));
    }

    /// The three states of `appearance.toml` that are not a preference.
    #[test]
    fn reduce_motion_defaults_off_for_every_shape_of_missing() {
        assert!(!reduce_motion_from(None));
        assert!(!reduce_motion_from(Some("[theme]\nactive = \"dark\"\n")));
        assert!(!reduce_motion_from(Some(
            "[accessibility]\nreduce_motion = \"yes\"\n"
        )));
        assert!(!reduce_motion_from(Some("not toml at all {{{")));
    }

    #[test]
    fn reduce_motion_reads_a_real_value() {
        assert!(reduce_motion_from(Some(
            "[theme]\nactive = \"dark\"\n\n[accessibility]\nreduce_motion = true\n"
        )));
        assert!(!reduce_motion_from(Some(
            "[accessibility]\nreduce_motion = false\n"
        )));
    }

    #[test]
    fn every_served_key_has_a_value_and_nothing_else_does() {
        let a = Appearance {
            color_scheme: SCHEME_LIGHT,
            accent: [0.1, 0.2, 0.3],
            reduced_motion: 1,
        };
        assert!(a.value_for(COLOR_SCHEME).is_some());
        assert!(a.value_for(ACCENT_COLOR).is_some());
        assert!(a.value_for(REDUCED_MOTION).is_some());
        // `contrast` is documented by the interface and deliberately unserved:
        // nothing in the tree decides it, so a caller gets the frontend's
        // default rather than a number we made up.
        assert!(a.value_for("contrast").is_none());
        assert_eq!(a.as_map().len(), 3);
    }

    /// The value a caller reads back is the one that went in, through the
    /// variant round-trip the bus actually performs.
    #[test]
    fn the_accent_survives_the_variant_round_trip() {
        let a = Appearance {
            color_scheme: SCHEME_DARK,
            accent: [0.25, 0.5, 0.75],
            reduced_motion: 0,
        };
        let owned = a.value_for(ACCENT_COLOR).unwrap();
        let back: (f64, f64, f64) = owned.try_into().unwrap();
        assert_eq!(back, (0.25, 0.5, 0.75));

        let scheme = a.value_for(COLOR_SCHEME).unwrap();
        assert_eq!(u32::try_from(scheme).unwrap(), SCHEME_DARK);
    }

    #[test]
    fn only_the_keys_that_moved_are_reported_changed() {
        let a = Appearance {
            color_scheme: SCHEME_DARK,
            accent: [0.1, 0.2, 0.3],
            reduced_motion: 0,
        };
        assert!(a.changed_keys(&a).is_empty());

        let mut b = a;
        b.color_scheme = SCHEME_LIGHT;
        assert_eq!(b.changed_keys(&a), vec![COLOR_SCHEME]);

        let mut c = a;
        c.accent = [0.9, 0.2, 0.3];
        c.reduced_motion = 1;
        assert_eq!(c.changed_keys(&a), vec![ACCENT_COLOR, REDUCED_MOTION]);
    }

    /// An unset accent is out of range in every component, which is how the
    /// spec spells "there is no accent" - a reader that clamps instead of
    /// checking the range would paint with -1.
    #[test]
    fn the_unset_accent_is_out_of_range_everywhere() {
        assert!(ACCENT_UNSET.iter().all(|c| !(0.0..=1.0).contains(c)));
    }

    /// The whole surface, over a real bus, from a caller the backend attests as
    /// the frontend - and from one it does not.
    ///
    /// Everything above this is a pure function. This is the part that has been
    /// wrong before in this daemon and cannot be reasoned about: whether the
    /// interface is actually on the bus under the name and path the frontend
    /// queries, whether the attestation admits the frontend and refuses
    /// everybody else, and whether the values survive the wire as the types the
    /// spec names.
    ///
    /// `#[ignore]`d because it needs a session bus of its own. It claims
    /// `org.freedesktop.portal.Desktop`, so do not run it against a live
    /// session:
    ///
    ///   dbus-run-session -- cargo test -p xdg-desktop-portal-arlen -- --ignored
    #[tokio::test]
    #[ignore = "needs a private session bus; see the doc comment for the command"]
    async fn the_frontend_reads_the_appearance_over_a_real_bus() {
        const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.arlen";
        const PATH: &str = "/org/freedesktop/portal/desktop";
        const IFACE: &str = "org.freedesktop.impl.portal.Settings";

        let _backend = zbus::connection::Builder::session()
            .unwrap()
            .name(BUS_NAME)
            .unwrap()
            .serve_at(PATH, Settings::new())
            .unwrap()
            .build()
            .await
            .expect("serve the backend");

        // A connection owning the frontend's name is what the attestation looks
        // for, so this stands in for the frontend exactly.
        let frontend = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .build()
            .await
            .expect("claim the frontend name");

        let reply = frontend
            .call_method(Some(BUS_NAME), PATH, Some(IFACE), "ReadAll", &(Vec::<String>::new(),))
            .await
            .expect("ReadAll");
        let all: HashMap<String, HashMap<String, OwnedValue>> =
            reply.body().deserialize().expect("the a{sa{sv}} shape");
        let ns = all.get(NAMESPACE).expect("the appearance namespace");
        assert_eq!(ns.len(), 3, "three keys, not four: {ns:?}");

        // The scheme resolved from real files: dark or light, never the
        // no-preference that means the theme did not resolve at all.
        let scheme = u32::try_from(ns[COLOR_SCHEME].clone()).expect("color-scheme is a u32");
        assert!(scheme == SCHEME_DARK || scheme == SCHEME_LIGHT, "scheme {scheme}");
        let accent: (f64, f64, f64) =
            ns[ACCENT_COLOR].clone().try_into().expect("accent-color is (ddd)");
        for c in [accent.0, accent.1, accent.2] {
            assert!((0.0..=1.0).contains(&c), "accent component out of sRGB range: {accent:?}");
        }

        // A key we do not serve is an error, which is what lets the frontend
        // fall through to its own default instead of reading a made-up number.
        let err = frontend
            .call_method(Some(BUS_NAME), PATH, Some(IFACE), "Read", &(NAMESPACE, "contrast"))
            .await;
        assert!(err.is_err(), "contrast must not answer: {err:?}");

        // And the other direction: a caller that is not the frontend is refused.
        // Without this the assertion above only proves the happy path exists.
        let stranger = zbus::Connection::session().await.expect("a plain connection");
        let reply = stranger
            .call_method(Some(BUS_NAME), PATH, Some(IFACE), "ReadAll", &(Vec::<String>::new(),))
            .await
            .expect("ReadAll from a stranger still returns");
        let all: HashMap<String, HashMap<String, OwnedValue>> =
            reply.body().deserialize().expect("the a{sa{sv}} shape");
        assert!(all.is_empty(), "an unattested caller reads nothing: {all:?}");

        // The push half. A backend that answers correctly but cannot signal
        // leaves every foreign app holding the appearance it started with, and
        // the failure is silent: the object-server lookup would log a warning
        // into a journal nobody reads. So drive the real emit path - the same
        // function the file watcher calls - and catch the signal on the
        // frontend's connection.
        use futures_util::StreamExt;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(IFACE)
            .unwrap()
            .member("SettingChanged")
            .unwrap()
            .build();
        let mut signals = zbus::MessageStream::for_match_rule(rule, &frontend, Some(4))
            .await
            .expect("subscribe to SettingChanged");

        let changed = Appearance {
            color_scheme: SCHEME_LIGHT,
            accent: [0.0, 0.5, 1.0],
            reduced_motion: 0,
        };
        crate::emit_changed(&_backend, &changed, &[COLOR_SCHEME, ACCENT_COLOR]).await;

        let mut seen = Vec::new();
        for _ in 0..2 {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(2), signals.next())
                .await
                .expect("a SettingChanged arrives")
                .expect("the stream is live")
                .expect("a well-formed signal");
            let (ns, key, value): (String, String, OwnedValue) =
                msg.body().deserialize().expect("the (ssv) signal body");
            assert_eq!(ns, NAMESPACE);
            if key == COLOR_SCHEME {
                assert_eq!(u32::try_from(value).unwrap(), SCHEME_LIGHT);
            } else if key == ACCENT_COLOR {
                let a: (f64, f64, f64) = value.try_into().unwrap();
                assert_eq!(a, (0.0, 0.5, 1.0));
            }
            seen.push(key);
        }
        seen.sort();
        assert_eq!(seen, vec![ACCENT_COLOR.to_string(), COLOR_SCHEME.to_string()]);
    }
}
