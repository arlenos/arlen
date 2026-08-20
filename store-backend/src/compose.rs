//! The catalog compose step (store-app.md section 9.3): map each source's app
//! records into [`CatalogEntry`]s the merge consumes. This module holds the forage
//! adapter (Arlen-native, the recipe schema is local); the Flathub AppStream-XML and
//! Debian DEP-11-YAML readers land alongside it as they are built.
//!
//! Pure mapping, no I/O: given an already-parsed forage `Recipe` and the layer its
//! cookbook resolves to (personal/community/official), produce one entry. The recipe
//! carries the same AppStream metadata a client renders (`recipe.md` ST-1), so a
//! forage app is a first-class catalog citizen, not a second-class listing.

use std::path::{Path, PathBuf};

use arlen_forage_recipe::{Capabilities, Recipe, ReproducibleStatus};

/// Which tracked cookbook a forage recipe came from.
///
/// Carried alongside the recipe rather than read here: composing is pure mapping
/// with no I/O, and the registry that knows this lives on disk. It is optional
/// because a recipe can reach the catalog without one - a directly configured
/// path has no cookbook, and inventing a publisher for it would be worse than
/// showing none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookbookOrigin {
    /// The tap name as the registry tracks it.
    pub name: String,
    /// Whether the registry holds a pinned TUF root for it. False for a cookbook
    /// tracked without a signed root, which resolution refuses to install from.
    pub pinned_root: bool,
}

use crate::catalog::{
    CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, ItemKind, SourceAttestation,
    SourceLayer, TrustSignals,
};

/// Map a parsed forage recipe to one catalog entry for the given source layer (the
/// tier its cookbook resolved to). Display comes from the recipe `[recipe]` metadata
/// (the same AppStream fields Flatpak/apt carry); the capability footprint is the
/// coarse categories the `[capabilities]` block declares (so a "needs network" /
/// "offline" facet is meaningful); the reproducible-build trust signal is populated
/// only when the recipe attests one (an unchecked status hides the row, section 9.2).
pub fn forage_entry(
    recipe: &Recipe,
    layer: SourceLayer,
    cookbook: Option<&CookbookOrigin>,
) -> CatalogEntry {
    let meta = &recipe.recipe;
    let display = DisplayMeta {
        name: meta.name.clone(),
        summary: meta.summary.clone(),
        description: meta.description.clone(),
        screenshots: meta.screenshots.clone(),
        // A recipe declares no icon reference; the client falls back to a default.
        icon: None,
    };
    // A recipe's `[capabilities]` IS the declaration: a recipe that asks for
    // nothing has genuinely asked for nothing, and that is the least-privilege
    // story the store exists to tell. Read, therefore, not unread.
    let capabilities =
        CapabilityFootprint::read(recipe.capabilities.as_ref().map(capability_labels).unwrap_or_default());
    let trust = TrustSignals {
        // The publisher a forage app has IS its cookbook: there is no separate
        // vendor identity to check, so a recipe from a tracked cookbook is
        // published by that cookbook and nothing more is claimed.
        verified_publisher: cookbook.map(|c| c.name.clone()),
        reproducible_build: recipe.reproducible.as_ref().and_then(|r| match r.status {
            ReproducibleStatus::Verified => Some("verified".to_string()),
            ReproducibleStatus::Expected => Some("expected".to_string()),
            ReproducibleStatus::Unreproducible => Some("unreproducible".to_string()),
            // Not yet checked: hide the row rather than assert a status.
            ReproducibleStatus::Unverified => None,
        }),
        install_count: None,
        odrs_score: None,
        // Deliberately empty, not unimplemented. Saying anything here needs a
        // per-app capability-USE feed, and nothing produces one: the audit ledger
        // records the AI taxonomy plus coarse app actions, `AuditKind::NetworkCall`
        // is the AI proxy's own egress rather than an app's, and a Grant node's
        // `use_count` is written 0 at every emit site and never incremented. The
        // per-app query answers `ObservedStatus::Unavailable` for the same reason
        // and with the same intent: an empty summary would render as a clean bill
        // of health the system cannot give. Fill this in with the observe-mode
        // feed (LCG-R8), not before.
        observed_vs_declared: None,
        // A cookbook's chain is TUF: the recipe is a signed target in metadata
        // this machine resolves against a root it pinned on first use. Absent
        // for a recipe that reached the catalog outside any tracked cookbook,
        // where there is no chain to name.
        attestation: cookbook.map(|c| SourceAttestation {
            chain: "tuf".to_string(),
            signer: c.name.clone(),
            pinned_here: c.pinned_root,
        }),
    };
    // A recipe carrying `[bridge] foreign_app` is a bridge, not a standalone app
    // (store-app.md section 8b); everything else is an app.
    let kind = if recipe.bridge.is_some() { ItemKind::Bridge } else { ItemKind::App };
    CatalogEntry {
        id: ComponentId(meta.id.clone()),
        layer,
        display,
        capabilities,
        trust,
        kind,
        // `forage install <name>` takes the recipe's own id.
        install_handle: Some(meta.id.clone()),
        // A `github-release` recipe follows tags and states no version; empty
        // means "this source does not say", which the update check treats as
        // nothing to compare rather than as a change.
        version: meta.version.clone().unwrap_or_default(),
    }
}

/// The coarse capability categories a recipe declares, as sorted, deduped display
/// labels: a `network`/`filesystem`/`notifications`/`clipboard`/`audio` token when
/// the app requests that category, each graph scope verbatim (`read:File`), and any
/// extra category key. Coarse on purpose - the store facets on categories; the
/// concrete hosts/paths are an app-detail concern.
fn capability_labels(caps: &Capabilities) -> Vec<String> {
    let mut labels = Vec::new();
    if !caps.network.is_empty() {
        labels.push("network".to_string());
    }
    if !caps.filesystem.is_empty() {
        labels.push("filesystem".to_string());
    }
    if caps.notifications {
        labels.push("notifications".to_string());
    }
    if caps.clipboard {
        labels.push("clipboard".to_string());
    }
    if caps.audio {
        labels.push("audio".to_string());
    }
    for scope in &caps.graph {
        labels.push(scope.clone());
    }
    for key in caps.extra.keys() {
        labels.push(key.clone());
    }
    labels.sort();
    labels.dedup();
    labels
}

/// Why a composed catalog could not be read.
#[derive(Debug)]
pub enum ComposeError {
    /// The XML did not parse (Flathub reader). XML is one atomic document, so a
    /// malformed catalog cannot be partially read; the DEP-11 reader, a multi-
    /// document stream, instead skips a single bad record (it is best-effort, not
    /// `Result`).
    Xml(String),
}

/// Parse a Flathub composed-AppStream catalog into one `CatalogEntry` per app.
///
/// A thin call into [`catalog_entries`]: a Flathub ref IS the component id, so the
/// install handle can be derived, which is not true of any other catalogue.
pub fn flathub_entries(xml: &str) -> Result<Vec<CatalogEntry>, ComposeError> {
    catalog_entries(xml, SourceLayer::Flatpak, true)
}

/// Parse a composed-AppStream catalog (`<components>` of `<component>`) into one
/// `CatalogEntry` per app, at `layer`. Display comes from the AppStream fields (the
/// UNLOCALIZED default element, ignoring `xml:lang` variants); the capability
/// footprint and the trust signals come from SEPARATE sources per section 9.2 and
/// stay empty here. A `<component>` with no id is skipped, not guessed.
///
/// `handle_from_id` says whether the component id is also the string that installs
/// the thing, which is true of Flathub alone. Otherwise the handle is the
/// component's own `<pkgname>` when it has one - an archive catalogue names the
/// package it came from - and nothing when it does not. A catalogue composed from
/// a staging tree has no package, so a forage app's entry carries no handle and
/// contributes pictures and prose to a variant the recipe made installable.
pub fn catalog_entries(
    xml: &str,
    layer: SourceLayer,
    handle_from_id: bool,
) -> Result<Vec<CatalogEntry>, ComposeError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ComposeError::Xml(e.to_string()))?;
    // The same base a DEP-11 header carries, in the attribute the XML form uses.
    // Flathub's catalogue states absolute URLs and needs none, so this is dormant
    // against what ships today - but it is the identical hole one serialisation
    // over, and the version that reads it costs a line.
    let media_base = doc
        .root_element()
        .attribute("media_baseurl")
        .map(str::to_string);
    let mut entries = Vec::new();
    for component in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "component")
    {
        let Some(id) = child_text(&component, "id") else {
            continue; // A component with no id is unusable; skip it.
        };
        let name = default_localized(&component, "name").unwrap_or_default();
        let display = DisplayMeta {
            name,
            summary: default_localized(&component, "summary"),
            description: description_text(&component),
            screenshots: screenshot_urls(&component)
                .into_iter()
                .map(|u| absolute_media_url(u, media_base.as_deref()))
                .collect(),
            // NOT the icon. `icon_ref` may hand back a CACHED NAME rather than a
            // URL, and a cached name resolves against the icons directory, not
            // against the media base - putting a base in front of one would turn
            // `app.png` into a URL that names nothing. The cached case is resolved
            // where the catalogue's own root is known, in `compose_catalog`.
            icon: icon_ref(&component),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id.clone()),
            layer,
            display,
            capabilities: CapabilityFootprint::unread(),
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            install_handle: if handle_from_id {
                Some(id)
            } else {
                child_text(&component, "pkgname")
            },
            version: latest_release_version(&component),
        });
    }
    Ok(entries)
}

/// Parse an installed app's MetaInfo document into a `CatalogEntry` (`layer =
/// Native`), or nothing when the document describes something that is not an app.
///
/// Same AppStream vocabulary as [`flathub_entries`], so the field extraction is
/// shared; the two differ in what the document IS. A Flathub catalog is one
/// `<components>` collection of things you can install. `/usr/share/metainfo`
/// holds one `<component>` per thing the DISTRIBUTION installed, which is why the
/// entry carries `install_handle: None`: the store has no route to install,
/// update or remove a pacman or dnf package, and offering one would be a lie.
///
/// Only apps. The 78 documents on the machine this was written against are 24
/// `desktop-application`, 18 `desktop` (the same thing, older spelling), 17
/// `addon`, 12 `font`, 4 `console-application` and 3 with no type at all. A store
/// listing fonts and codec addons as apps is noise, so the filter is the two
/// desktop spellings. `console-application` is EXCLUDED for now and this is the
/// line to change if that turns out to be wrong: those are real programs, they
/// just have no window, and widening the filter is a smaller decision than
/// pruning noise back out once people have seen it.
pub fn metainfo_entry(xml: &str) -> Option<CatalogEntry> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let component = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "component")?;
    let kind = component.attribute("type").unwrap_or_default();
    if kind != "desktop-application" && kind != "desktop" {
        return None;
    }
    let id = child_text(&component, "id")?;
    Some(CatalogEntry {
        id: ComponentId(id),
        layer: SourceLayer::Native,
        display: DisplayMeta {
            name: default_localized(&component, "name").unwrap_or_default(),
            summary: default_localized(&component, "summary"),
            description: description_text(&component),
            screenshots: screenshot_urls(&component),
            icon: icon_ref(&component),
        },
        capabilities: CapabilityFootprint::unread(),
        trust: TrustSignals::default(),
        kind: ItemKind::default(),
        // No install route exists for a distribution package, so the card shows
        // the app without offering an action it cannot perform.
        install_handle: None,
        version: latest_release_version(&component),
    })
}

/// The text of the first direct child element named `tag`.
fn child_text<'a>(node: &roxmltree::Node<'a, 'a>, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == tag)
        .and_then(|c| c.text())
        .map(str::to_string)
}

/// The text of the unlocalized `tag` child (the one without an `xml:lang`), the
/// default the store renders; localized variants are ignored.
fn default_localized(node: &roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == tag)
        .find(|c| !c.attributes().any(|a| a.name() == "lang"))
        .and_then(|c| c.text())
        .map(str::to_string)
}

/// The unlocalized `<description>`'s concatenated `<p>` paragraph texts.
///
/// Localization is filtered at BOTH levels, because AppStream puts it in both
/// places. A composed catalog carries one `<description xml:lang="..">` per
/// language; a `.metainfo.xml` shipped with an app keeps one `<description>` and
/// tags the PARAGRAPHS inside it. Filtering only the outer element passed the
/// inner ones straight through, so Helix's card described itself in English and
/// then again in Arabic, one after the other, in the same string. Nothing failed;
/// the card just read as nonsense, which is why no test caught it and looking at
/// the served card did.
fn description_text(node: &roxmltree::Node) -> Option<String> {
    let desc = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "description")
        .find(|c| !c.attributes().any(|a| a.name() == "lang"))?;
    let paras: Vec<&str> = desc
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "p")
        .filter(|p| !p.attributes().any(|a| a.name() == "lang"))
        .filter_map(|p| p.text())
        .collect();
    if paras.is_empty() {
        None
    } else {
        Some(paras.join("\n\n"))
    }
}

/// Every `<screenshots>/<screenshot>/<image>` URL, in document order.
fn screenshot_urls(node: &roxmltree::Node) -> Vec<String> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == "screenshots")
        .flat_map(|shots| shots.children())
        .filter(|c| c.is_element() && c.tag_name().name() == "screenshot")
        .flat_map(|shot| shot.children())
        .filter(|c| c.is_element() && c.tag_name().name() == "image")
        .filter_map(|img| img.text().map(str::to_string))
        .collect()
}

/// The first `<icon>` reference, preferring a `type="remote"` URL the store can
/// fetch, else the first icon's text (a cached/stock name).
/// The version this component's newest stable release states, or empty.
///
/// AppStream conventionally lists releases newest-first, but that is convention
/// and not a guarantee, so the newest `timestamp` wins where one is given and
/// document order only decides among releases that state none. Getting this
/// backwards would report an OLD version as available and make a current install
/// look outdated forever.
///
/// `type="development"` releases are skipped: they are pre-release builds, and
/// offering one as the available version would push users onto a track they did
/// not choose.
fn latest_release_version(node: &roxmltree::Node) -> String {
    let mut best: Option<(i64, usize, String)> = None;

    for (index, release) in node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "releases")
        .flat_map(|r| r.children())
        .filter(|c| c.is_element() && c.tag_name().name() == "release")
        .enumerate()
    {
        if release.attribute("type") == Some("development") {
            continue;
        }
        let Some(version) = release.attribute("version").filter(|v| !v.is_empty()) else {
            continue;
        };
        let timestamp = release
            .attribute("timestamp")
            .and_then(|t| t.parse::<i64>().ok())
            .unwrap_or(i64::MIN);

        // Later timestamp wins; among equals (including all-unstamped) the first
        // in document order does, which is the newest by AppStream convention.
        let better = match &best {
            None => true,
            Some((best_ts, best_index, _)) => {
                timestamp > *best_ts || (timestamp == *best_ts && index < *best_index)
            }
        };
        if better {
            best = Some((timestamp, index, version.to_string()));
        }
    }

    best.map(|(_, _, v)| v).unwrap_or_default()
}

fn icon_ref(node: &roxmltree::Node) -> Option<String> {
    let icons: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "icon")
        .collect();
    icons
        .iter()
        .find(|c| c.attribute("type") == Some("remote"))
        .or_else(|| icons.first())
        .and_then(|c| c.text())
        .map(str::to_string)
}

// --- Debian DEP-11 (AppStream-in-YAML) reader -----------------------------------

/// A DEP-11 component document. Fields are optional so the leading header document
/// (`File: DEP-11`, no `ID`) and any partial record parse without erroring; a record
/// with no `ID` is skipped by [`dep11_entries`]. Localized fields are locale maps
/// (`{C: ..., de: ...}`); only the `C` (unlocalized) value is rendered.
#[derive(serde::Deserialize)]
struct Dep11Component {
    #[serde(rename = "ID")]
    id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Summary")]
    summary: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Description")]
    description: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "Icon")]
    icon: Option<Dep11Icon>,
    #[serde(rename = "Screenshots")]
    screenshots: Option<Vec<Dep11Screenshot>>,
    #[serde(rename = "Releases")]
    releases: Option<Vec<Dep11Release>>,
    /// The Debian package that provides this component - the name `apt install`
    /// takes, and the key the curated permission profiles are filed under. Not
    /// derivable from `ID`, which is why DEP-11 states it separately.
    #[serde(rename = "Package")]
    package: Option<String>,
}

/// One DEP-11 release record. Only the fields the update check needs; unknown
/// keys are ignored, since a distro's catalog carries far more than this.
#[derive(serde::Deserialize)]
struct Dep11Release {
    version: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    unix_timestamp: Option<i64>,
}

/// The newest stable version a DEP-11 component states, or empty.
///
/// Same rule as the XML side: development releases are skipped, the newest
/// timestamp wins, and document order breaks ties.
fn dep11_release_version(releases: &Option<Vec<Dep11Release>>) -> String {
    let Some(releases) = releases else {
        return String::new();
    };
    let mut best: Option<(i64, usize, String)> = None;
    for (index, release) in releases.iter().enumerate() {
        if release.kind.as_deref() == Some("development") {
            continue;
        }
        let Some(version) = release.version.as_deref().filter(|v| !v.is_empty()) else {
            continue;
        };
        let timestamp = release.unix_timestamp.unwrap_or(i64::MIN);
        let better = match &best {
            None => true,
            Some((best_ts, best_index, _)) => {
                timestamp > *best_ts || (timestamp == *best_ts && index < *best_index)
            }
        };
        if better {
            best = Some((timestamp, index, version.to_string()));
        }
    }
    best.map(|(_, _, v)| v).unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct Dep11Icon {
    remote: Option<Vec<Dep11RemoteIcon>>,
    cached: Option<Vec<Dep11CachedIcon>>,
}

#[derive(serde::Deserialize)]
struct Dep11RemoteIcon {
    url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Dep11CachedIcon {
    name: Option<String>,
}

#[derive(serde::Deserialize)]
struct Dep11Screenshot {
    #[serde(rename = "source-image")]
    source_image: Option<Dep11Image>,
}

#[derive(serde::Deserialize)]
struct Dep11Image {
    url: Option<String>,
}

/// Parse a Debian DEP-11 catalog (a multi-document YAML stream: a header document
/// then one document per component) into one `CatalogEntry` per app (`layer = Apt`).
/// Display comes from the `C` (unlocalized) locale value of each field; the capability
/// footprint (the apt-enrolled profile, section 5) and the trust signals (Debian
/// keyring / popcon / reproduce.debian.net) come from SEPARATE sources per section 9.2
/// and stay empty here.
///
/// Best-effort per record, matching the module's skip-don't-guess philosophy: a
/// document with no `ID` (the header, a partial record) is skipped, and a single
/// document that fails to deserialize (a corrupt or non-conformant record, e.g. a
/// `Name:` that is a scalar rather than the expected locale map) is skipped too,
/// keeping the rest. A real DEP-11 catalog carries thousands of components, so one
/// bad record must not drop every Debian app from the store. There is no whole-stream
/// error: an entirely unparseable input simply yields no entries.
pub fn dep11_entries(input: &CatalogInput) -> Vec<CatalogEntry> {
    let yaml = input.text.as_str();
    // The header document declares the origin the icon cache is filed under. Read
    // it rather than deriving one from the filename: `trixie_main.yml.gz` is filed
    // under `debian-trixie-main`, so a filename-derived guess would miss every icon.
    let origin = dep11_header_field(yaml, "Origin");
    let media_base = dep11_header_field(yaml, "MediaBaseUrl");
    use serde::Deserialize;
    let mut entries = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let Ok(comp) = Dep11Component::deserialize(doc) else {
            continue; // A corrupt/non-conformant record is skipped, not fatal to the source.
        };
        let Some(id) = comp.id else {
            continue; // The header document or a record with no id.
        };
        let c = |m: &Option<std::collections::BTreeMap<String, String>>| {
            m.as_ref().and_then(|m| m.get("C").cloned())
        };
        let display = DisplayMeta {
            name: c(&comp.name).unwrap_or_default(),
            summary: c(&comp.summary),
            description: c(&comp.description),
            screenshots: comp
                .screenshots
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| s.source_image.and_then(|i| i.url))
                .map(|u| absolute_media_url(u, media_base.as_deref()))
                .collect(),
            icon: comp.icon.and_then(|i| {
                dep11_icon(i, input.root.as_deref(), origin.as_deref())
            }),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id),
            layer: SourceLayer::Apt,
            display,
            capabilities: CapabilityFootprint::unread(),
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            version: dep11_release_version(&comp.releases),
            // Absent when the record does not name one, which the caller must
            // read as "cannot install this variant" - guessing a package name
            // from the component id installs whatever happens to match.
            install_handle: comp.package,
        });
    }
    entries
}

/// A top-level scalar from a DEP-11 catalogue's header document.
///
/// Read with a line scan rather than by deserialising the header: the header is the
/// first document of a multi-document stream and carries no `ID`, so the record loop
/// already skips it, and a second full parse to reach one scalar is not worth it. The
/// scan stops at the first record separator, so a component field of the same name in
/// a later document cannot be mistaken for it.
fn dep11_header_field(yaml: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    let mut seen_body = false;
    for line in yaml.lines() {
        if line.trim() == "---" {
            if seen_body {
                return None; // Past the header and nothing declared.
            }
            continue;
        }
        seen_body = true;
        if let Some(rest) = line.strip_prefix(&prefix) {
            let v = rest.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Make a DEP-11 media reference loadable: its URLs are RELATIVE to the header's
/// `MediaBaseUrl`, and every screenshot in the Debian archive is one of these
/// (`org/gnome/World.Secrets/d0219.../screenshots/image-1_orig.png`). Without the
/// base a renderer has a path with nothing in front of it.
///
/// An already-absolute URL is left alone, and so is a relative one with no base to
/// put in front of it - there is nothing better to do with it than hand it on, and
/// dropping it would lose a picture that a caller with its own base could still use.
fn absolute_media_url(url: String, base: Option<&str>) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url;
    }
    match base {
        Some(b) => format!("{}/{}", b.trim_end_matches('/'), url.trim_start_matches('/')),
        None => url,
    }
}

/// The icon a card should show for a DEP-11 component: the cached file's path when
/// it can be found on disk, else the remote URL, else nothing.
fn dep11_icon(icon: Dep11Icon, root: Option<&Path>, origin: Option<&str>) -> Option<String> {
    let cached = icon
        .cached
        .as_ref()
        .and_then(|c| c.iter().find_map(|i| i.name.clone()));
    if let (Some(name), Some(root), Some(origin)) = (cached.as_deref(), root, origin) {
        if let Some(path) = cached_icon_path(root, origin, name) {
            return Some(path);
        }
    }
    // No cache on this machine (a fixture, an env override, a catalogue whose icon
    // tarballs were never fetched): the remote URL is the only thing a renderer can
    // actually use. A bare cached name would render as a broken image.
    icon.remote.and_then(|r| r.into_iter().find_map(|i| i.url))
}

/// Turn a cached icon NAME into the path of the file it names, or `None` when there
/// is no such file.
///
/// `<root>/icons/<origin>/<size>/<name>` is the layout both halves of the catalogue
/// use: it is how Debian's `icons-*.tar.gz` unpacks, and the forage pipeline passes
/// `appstreamcli compose` an icons directory shaped the same way so there is one
/// rule here and not two.
///
/// 128 before 64. Both are published and a card wants both - a list row and a detail
/// header - but the field holds one string, so the choice is which mistake to make.
/// A 128 scaled down to a row looks like the picture; a 64 scaled up to a header
/// looks like a mistake.
///
/// `None` rather than the bare name when nothing is there, because a name no one can
/// resolve is not better than no icon: it renders as a broken image instead of as
/// the blank the card already handles.
fn cached_icon_path(root: &Path, origin: &str, name: &str) -> Option<String> {
    for size in ["128x128", "64x64"] {
        let p = root.join("icons").join(origin).join(size).join(name);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}


/// Which layer a composed XML catalogue's apps belong to, from its origin name and
/// the recipes the store already knows about.
///
/// The origin is the filename stem, because `appstreamcli compose` does not write
/// an `origin` attribute into the document (checked against 1.1.5). That is fine:
/// whatever installs a catalogue chooses its filename, so the name is a statement
/// by whoever put it there rather than a guess about its contents.
///
/// For a forage package's own catalogue the layer comes from the recipe with the
/// same component id, since the store enumerates cookbooks and already knows each
/// recipe's tier. A component with no such recipe yields `None` and is skipped: it
/// is an app on the machine that no cookbook offers, so there is no install route
/// to put behind it, and its metainfo already lists it.
///
/// The lookup is scoped to a forage catalogue on purpose. Matching by id in general
/// would drag an archive catalogue's entry for an app that ALSO has a recipe onto
/// the cookbook layer, inventing a forage install for a package that came from apt.
/// Anything else is the archive's own catalogue in the XML serialisation, the same
/// content DEP-11 carries, so it gets the layer DEP-11 gets and the two forms of one
/// catalogue cannot become two variants.
pub fn layer_for_catalog_origin(
    origin: &str,
    recipe_layer: Option<SourceLayer>,
) -> Option<SourceLayer> {
    match origin {
        arlen_forage_recipe::CATALOG_ORIGIN => recipe_layer,
        _ => Some(SourceLayer::Apt),
    }
}

// --- compose orchestration (section 9.3: "produces the one merged model") --------

/// One catalogue file, with enough about where it came from to find its pictures.
///
/// The text alone is not enough. A cached icon in either serialisation is a bare
/// flat name (`gitg_org.gnome.gitg.png`); the file it names lives at
/// `<root>/icons/<origin>/<size>/<name>`, and neither the root nor the origin is
/// recoverable from the document by anything downstream. The store read the file,
/// so the store is the one party that knows - which is why this carries it rather
/// than leaving a renderer to reconstruct a directory by guesswork.
#[derive(Debug, Default, Clone)]
pub struct CatalogInput {
    /// The catalogue document.
    pub text: String,
    /// The `swcatalog` directory the file was found under, when it came off disk.
    /// `None` for a fixture or an env override, and then icons stay bare names.
    pub root: Option<PathBuf>,
    /// The origin, for the XML form whose filename carries it. DEP-11 documents
    /// declare their own in the header, so this stays `None` for them.
    pub origin: Option<String>,
}

impl From<String> for CatalogInput {
    fn from(text: String) -> Self {
        Self { text, ..Default::default() }
    }
}

impl From<&str> for CatalogInput {
    fn from(text: &str) -> Self {
        Self { text: text.to_string(), ..Default::default() }
    }
}

/// The already-read source contents the compose step merges. Held as text (not file
/// paths) so the orchestration is pure and testable; the daemon reads the files.
#[derive(Debug, Default)]
pub struct SourceInputs {
    /// `(recipe.toml text, the cookbook's resolved tier)` per forage recipe.
    pub forage: Vec<(String, SourceLayer, Option<CookbookOrigin>)>,
    /// The Flathub composed-AppStream catalog XML, one per configured remote.
    ///
    /// A LIST, not one document: a machine can have several Flatpak remotes, and
    /// Debian splits its catalog per suite and component (`main`, `contrib`,
    /// `non-free` are separate files). Taking only the first would silently
    /// serve a fraction of what is installable while looking complete.
    pub flathub_xml: Vec<String>,
    /// The Debian DEP-11 catalog YAML, one per suite/component file. See
    /// [`SourceInputs::flathub_xml`] for why this is a list.
    pub dep11_yaml: Vec<CatalogInput>,
    /// One composed AppStream catalogue in XML form per file. Its `origin` decides
    /// the layer, per [`layer_for_catalog_origin`].
    pub catalog_xml: Vec<CatalogInput>,
    /// One MetaInfo document per app the distribution installed, from
    /// `/usr/share/metainfo`. Not an availability catalog: see [`metainfo_entry`].
    pub metainfo_xml: Vec<String>,
    /// `(component-id, Flatpak `metadata` file text)` for Flathub apps whose
    /// sandbox permissions are known (SC-3). The composed AppStream catalog
    /// carries no `finish-args`, so the footprint is fused in from here; an id
    /// with no entry keeps an empty footprint rather than a guessed one.
    pub flatpak_metadata: Vec<(String, String)>,
    /// `(component-id, the app's enrolled permission profile TOML)` for apt apps
    /// (SC-3's apt half). A `.deb` declares no capabilities, so the footprint
    /// comes from the profile the enrol hook wrote - which is also what confines
    /// the app at runtime, so it describes what the app IS allowed rather than
    /// what someone inferred it might do.
    ///
    /// The CALLER supplies these pairs rather than the composer reading
    /// `~/.config/permissions` itself. The composer maps catalog files; knowing
    /// which apps are installed locally, and which component-id an enrolled
    /// profile belongs to, is local state it has no business guessing at. Same
    /// split as `outdated()` taking the installed set from its caller.
    pub apt_profiles: Vec<(String, String)>,
    /// The ODRS ratings document, when one has been fetched.
    ///
    /// Supplied by the caller for the same reason the profiles are: this
    /// composer maps catalog files, and whether the machine has talked to
    /// `odrs.gnome.org` recently is not its business to decide. `None` means
    /// nobody has asked, which the card renders as no row rather than as a bad
    /// score.
    pub odrs: Option<crate::odrs::Ratings>,
}

/// Compose the merged [`Catalog`] from every configured source. Best-effort: a source
/// that fails to parse (one malformed recipe, an unreadable catalog) is SKIPPED, never
/// fatal, so a single bad input cannot blank the whole store. Returns the deduped,
/// merged catalog the `org.arlen.Store1` backend serves.
pub fn compose_catalog(inputs: SourceInputs) -> crate::query::Catalog {
    let mut entries = Vec::new();
    for (toml, layer, cookbook) in &inputs.forage {
        if let Ok(recipe) = arlen_forage_recipe::parse(toml) {
            entries.push(forage_entry(&recipe, *layer, cookbook.as_ref()));
        }
    }
    // Every remote and every suite/component, merged. The dedupe in
    // `merge_catalog` collapses an app carried by two of them into one card.
    for xml in &inputs.flathub_xml {
        if let Ok(es) = flathub_entries(xml) {
            entries.extend(es);
        }
    }
    // One document per installed app, so a malformed one costs that app and
    // nothing else - the same best-effort rule the other sources follow.
    for xml in &inputs.metainfo_xml {
        if let Some(entry) = metainfo_entry(xml) {
            entries.push(entry);
        }
    }
    for yaml in &inputs.dep11_yaml {
        // Best-effort per record: a corrupt document is skipped inside, never fatal.
        entries.extend(dep11_entries(yaml));
    }
    // Same best-effort rule as the others: an unparseable catalogue costs its own
    // apps and nothing else. Read LAST, because a forage package's catalogue takes
    // its layer from the recipe entry for the same app, which must already be in
    // hand - the recipe knows the cookbook, the composed document cannot.
    let recipe_layers: std::collections::HashMap<ComponentId, SourceLayer> = entries
        .iter()
        .filter(|e| {
            matches!(
                e.layer,
                SourceLayer::Personal | SourceLayer::Community | SourceLayer::Official
            )
        })
        .map(|e| (e.id.clone(), e.layer))
        .collect();
    for input in &inputs.catalog_xml {
        let origin = input.origin.clone().unwrap_or_default();
        // The layer is per component, so parse at a placeholder and restamp: only
        // the id in the document can say which recipe it belongs to.
        let Ok(es) = catalog_entries(&input.text, SourceLayer::Apt, false) else {
            continue;
        };
        for mut e in es {
            let Some(layer) = layer_for_catalog_origin(&origin, recipe_layers.get(&e.id).copied())
            else {
                continue; // A forage catalogue for an app no cookbook offers.
            };
            e.layer = layer;
            // A cached icon here is a bare name too, under the same layout.
            if let (Some(root), Some(name)) = (input.root.as_deref(), e.display.icon.as_deref()) {
                if !name.contains('/') {
                    e.display.icon = cached_icon_path(root, &origin, name);
                }
            }
            entries.push(e);
        }
    }
    // AFTER every source, not inside the Flathub branch. It sat there until 19
    // August, so on a machine with Flatpaks installed and no Flathub catalogue -
    // which is every machine that has not downloaded one, including this one -
    // each app's `metadata` was read off disk and then dropped, and the card the
    // person sees carried no permissions at all. The fuse matches by id and does
    // not care which source produced the entry: an installed Flatpak that
    // reached the catalogue through its exported MetaInfo is the same app.
    fuse_flatpak_metadata(&mut entries, &inputs.flatpak_metadata);
    // The apt fuse moves out of the DEP-11 loop for the same reason: an enrolled
    // `.deb` whose card reached the catalogue through its installed MetaInfo,
    // rather than through a DEP-11 document, is the same app and its profile is
    // the same declaration. Matching is by id either way.
    fuse_apt_profiles(&mut entries, &inputs.apt_profiles);
    // Last, and over every source: an app carried by Debian and by Flathub is
    // one card, and what other people made of it does not depend on which one
    // of them the machine happened to read.
    if let Some(ratings) = &inputs.odrs {
        for entry in &mut entries {
            entry.trust.odrs_score = ratings.score_for(&entry.id.0);
        }
    }
    crate::query::Catalog::new(crate::catalog::merge_catalog(entries))
}

/// Fill each Flatpak entry's capability footprint from its `metadata` file
/// (SC-3). Matching is by component-id; an entry with no metadata is left with
/// its empty footprint, which the app renders as "not known" rather than as
/// "asks for nothing".
fn fuse_flatpak_metadata(entries: &mut [CatalogEntry], metadata: &[(String, String)]) {
    for (id, text) in metadata {
        let labels = crate::flatpak::context_labels(text);
        for entry in entries.iter_mut().filter(|e| same_app(&e.id.0, id)) {
            // READ, and marked so: this app's own `metadata` file is the
            // persisted form of its `finish-args`, so a Flatpak that asks for
            // nothing here has genuinely asked for nothing. Without the flag the
            // facets would stay withdrawn for an app we did read.
            entry.capabilities = CapabilityFootprint::read(labels.clone());
        }
    }
}

/// Is this AppStream component the same app as this Flatpak ref?
///
/// Usually the ids are identical. The exception is the AppStream convention of
/// suffixing a desktop component with `.desktop` (`org.gnome.gedit.desktop`)
/// where the Flatpak ref has none (`org.gnome.gedit`) - a naming convention, not
/// a resemblance, so matching across it is not a guess. Nothing else is
/// accepted: two ids that merely look alike are two apps.
fn same_app(component_id: &str, flatpak_id: &str) -> bool {
    component_id == flatpak_id
        || component_id.strip_suffix(".desktop").is_some_and(|base| base == flatpak_id)
}

/// Fill each apt entry's capability footprint from the app's enrolled profile.
///
/// An id with no enrolled profile keeps an EMPTY footprint rather than a guessed
/// one - the same rule the Flatpak fuse follows. That matters for the
/// least-privilege sort: a blank footprint must mean "we do not know", never
/// "asks for nothing", or the ordering would reward apps we have no data on.
/// A profile that does not parse is skipped for the same reason.
fn fuse_apt_profiles(entries: &mut [CatalogEntry], profiles: &[(String, String)]) {
    for (id, toml) in profiles {
        let Ok(profile) = toml::from_str::<arlen_permissions::PermissionProfile>(toml) else {
            continue;
        };
        let labels = arlen_extensions::profile::profile_labels(&profile);
        for entry in entries.iter_mut().filter(|e| same_app(&e.id.0, id)) {
            // An enrolled profile IS the declaration, so this footprint is read.
            entry.capabilities = CapabilityFootprint::read(labels.clone());
        }
    }
}

#[cfg(test)]
mod tests {

    /// A Flatpak whose `metadata` was read must count as READ, not only carry the
    /// labels. The distinction is what the negative facets rest on: without the
    /// flag, an app we did read keeps its claim withdrawn for ever, which is the
    /// opposite error to the one the flag exists to prevent.
    /// The whole point of reading a Flatpak's `metadata`: an app that arrived
    /// through its exported MetaInfo, with no Flathub catalogue anywhere, still
    /// gets its real permissions. Until 19 August the fuse only ran inside the
    /// Flathub branch, so on a machine with Flatpaks installed and no catalogue
    /// downloaded - this one - every file was read and then dropped.
    #[test]
    fn an_installed_flatpak_gets_its_permissions_without_a_flathub_catalogue() {
        let inputs = super::SourceInputs {
            metainfo_xml: vec![
                "<component type=\"desktop-application\"><id>com.example.Recorder</id>\
                 <name>Recorder</name></component>"
                    .to_string(),
            ],
            flatpak_metadata: vec![(
                "com.example.Recorder".to_string(),
                "[Application]\nname=com.example.Recorder\n\n[Context]\n\
                 shared=network;\nfilesystems=home;\n"
                    .to_string(),
            )],
            ..Default::default()
        };
        let catalog = super::compose_catalog(inputs);
        let card = &catalog.search("", &[])[0];
        let footprint = &card.variants[0].capabilities;
        assert!(footprint.known, "its own metadata was read");
        assert!(footprint.capabilities.contains(&"network".to_string()));
        assert!(footprint.capabilities.contains(&"filesystem".to_string()));
    }

    /// AppStream suffixes a desktop component with `.desktop` where the Flatpak
    /// ref has none. That is a naming convention rather than a resemblance, so
    /// matching across it is sound - and without it, `org.gnome.gedit.desktop`
    /// never meets `org.gnome.gedit` and the app keeps no permissions at all.
    #[test]
    fn the_appstream_desktop_suffix_still_matches_the_flatpak_ref() {
        assert!(super::same_app("org.gnome.gedit.desktop", "org.gnome.gedit"));
        assert!(super::same_app("com.obsproject.Studio", "com.obsproject.Studio"));
        assert!(
            !super::same_app("org.gnome.gedit.plugin", "org.gnome.gedit"),
            "only the suffix convention, not any shared prefix"
        );
    }

    /// The apt half of the same move: an enrolled `.deb` whose card came from
    /// its installed MetaInfo, with no DEP-11 document anywhere, still gets its
    /// declared capabilities. Inside the DEP-11 loop the fuse only ever reached
    /// entries that loop produced.
    #[test]
    fn an_enrolled_deb_gets_its_profile_without_a_dep11_catalogue() {
        let inputs = super::SourceInputs {
            metainfo_xml: vec![
                "<component type=\"desktop-application\"><id>org.example.Notes</id>\
                 <name>Notes</name></component>"
                    .to_string(),
            ],
            apt_profiles: vec![(
                "org.example.Notes".to_string(),
                "[info]\napp_id = \"org.example.Notes\"\ntier = \"third-party\"\n\
                 \n[filesystem]\nhome = true\n"
                    .to_string(),
            )],
            ..Default::default()
        };
        let catalog = super::compose_catalog(inputs);
        let card = &catalog.search("", &[])[0];
        assert!(card.variants[0].capabilities.known, "the profile was read");
        assert!(
            card.variants[0].capabilities.capabilities.contains(&"filesystem".to_string()),
            "and it declared filesystem access: {:?}",
            card.variants[0].capabilities.capabilities
        );
    }

    #[test]
    fn a_fused_flatpak_footprint_counts_as_read() {
        let mut entries = vec![super::CatalogEntry {
            id: super::ComponentId("org.example.Fused".into()),
            layer: super::SourceLayer::Flatpak,
            display: super::DisplayMeta { name: "Fused".into(), ..Default::default() },
            capabilities: super::CapabilityFootprint::unread(),
            trust: super::TrustSignals::default(),
            version: String::new(),
            install_handle: Some("org.example.Fused".into()),
            kind: Default::default(),
        }];
        let metadata = vec![(
            "org.example.Fused".to_string(),
            "[Application]\nname=org.example.Fused\n\n[Context]\nshared=network;ipc;\n".to_string(),
        )];
        super::fuse_flatpak_metadata(&mut entries, &metadata);
        assert!(entries[0].capabilities.known, "we read this one");
        assert_eq!(entries[0].capabilities.capabilities, vec!["network"]);
    }

    /// And an id with no metadata stays unread rather than becoming a clean slate.
    #[test]
    fn a_flatpak_without_metadata_stays_unread() {
        let mut entries = vec![super::CatalogEntry {
            id: super::ComponentId("org.example.Absent".into()),
            layer: super::SourceLayer::Flatpak,
            display: super::DisplayMeta { name: "Absent".into(), ..Default::default() },
            capabilities: super::CapabilityFootprint::unread(),
            trust: super::TrustSignals::default(),
            version: String::new(),
            install_handle: Some("org.example.Absent".into()),
            kind: Default::default(),
        }];
        super::fuse_flatpak_metadata(&mut entries, &[]);
        assert!(!entries[0].capabilities.known);
    }

    #[test]
    fn an_apt_entry_carries_the_package_apt_would_be_told_to_install() {
        // Without this the store can describe a Debian app and not install it:
        // the component id is not the package name, and guessing one from the
        // other installs whatever happens to match.
        let yaml = "\
---
File: DEP-11
---
ID: org.example.Thing
Package: example-thing
Name:
  C: Thing
";
        let entries = dep11_entries(&yaml.into());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].install_handle.as_deref(), Some("example-thing"));
    }

    #[test]
    fn a_record_naming_no_package_is_not_guessed_at() {
        let yaml = "\
---
File: DEP-11
---
ID: org.example.Thing
Name:
  C: Thing
";
        let entries = dep11_entries(&yaml.into());
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].install_handle.is_none(),
            "no package stated means not installable, not `org.example.Thing`"
        );
    }
    use super::*;
    use crate::catalog::merge_catalog;

    const APT_YAML: &str = "---\nID: org.example.App\nName:\n  C: Example\n";

    fn enrolled(network: bool) -> String {
        format!(
            "[info]\napp_id = \"org.example.App\"\nname = \"Example\"\ntier = \"third-party\"\n\n[network]\nallow_all = {network}\n"
        )
    }

    /// SC-3's apt half end to end: a Debian card's footprint comes from the
    /// enrolled profile, so the capability panel is no longer blank.
    #[test]
    fn a_rating_reaches_the_card_whatever_source_carried_the_app() {
        // The fuse runs after every source for the same reason the others do: an
        // app carried by Debian and by Flathub is one card, and what other
        // people made of it does not depend on which catalogue the machine
        // happened to read.
        let ratings = crate::odrs::Ratings::parse(
            r#"{"org.gnome.gedit": {"star0": 1, "star5": 3, "total": 4}}"#,
        )
        .unwrap();
        let catalog = compose_catalog(SourceInputs {
            odrs: Some(ratings),
            dep11_yaml: vec![DEP11_YAML.into()],
            ..Default::default()
        });
        let card = catalog
            .card(&ComponentId("org.gnome.gedit".into()))
            .expect("the dep11 fixture carries gedit");
        // Trust lives per VARIANT: the same app can be carried by two layers and
        // each says its own thing about provenance. A rating is about the app, so
        // every variant of it reports the same one.
        assert!(
            card.variants.iter().all(|v| v.trust.odrs_score == Some(5.0)),
            "three five-star ratings and one unrated"
        );
    }

    #[test]
    fn an_app_with_no_rating_carries_no_score_rather_than_a_zero() {
        let catalog = compose_catalog(SourceInputs {
            odrs: Some(crate::odrs::Ratings::parse("{}").unwrap()),
            dep11_yaml: vec![DEP11_YAML.into()],
            ..Default::default()
        });
        let card = catalog
            .card(&ComponentId("org.gnome.gedit".into()))
            .expect("the dep11 fixture carries gedit");
        assert!(card.variants.iter().all(|v| v.trust.odrs_score.is_none()));
    }

    #[test]
    fn an_apt_entry_takes_its_footprint_from_the_enrolled_profile() {
        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml: Vec::new(),
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
            metainfo_xml: Vec::new(),
            flatpak_metadata: vec![],
            apt_profiles: vec![("org.example.App".into(), enrolled(true))],
        });
        let card = catalog
            .card(&crate::catalog::ComponentId("org.example.App".into()))
            .expect("the apt entry should be in the catalog");
        assert_eq!(card.variants[0].capabilities.capabilities, vec!["network".to_string()]);
    }

    /// An app with no enrolled profile keeps an EMPTY footprint. Blank must mean
    /// "we do not know", never "asks for nothing", or the least-privilege sort
    /// would reward the apps we have no data on.
    #[test]
    fn an_apt_entry_without_a_profile_keeps_an_empty_footprint() {
        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml: Vec::new(),
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
            metainfo_xml: Vec::new(),
            flatpak_metadata: vec![],
            apt_profiles: vec![],
        });
        let card = catalog
            .card(&crate::catalog::ComponentId("org.example.App".into()))
            .expect("the apt entry should be in the catalog");
        assert!(card.variants[0].capabilities.capabilities.is_empty());
    }

    /// A profile that does not parse is skipped, not guessed at.
    #[test]
    fn an_unparseable_profile_leaves_the_footprint_empty() {
        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml: Vec::new(),
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
            metainfo_xml: Vec::new(),
            flatpak_metadata: vec![],
            apt_profiles: vec![("org.example.App".into(), "not = = toml".into())],
        });
        let card = catalog
            .card(&crate::catalog::ComponentId("org.example.App".into()))
            .expect("the apt entry should be in the catalog");
        assert!(card.variants[0].capabilities.capabilities.is_empty());
    }

    fn recipe_toml(id: &str, extra: &str) -> Recipe {
        let text = format!(
            r#"
[recipe]
id = "{id}"
name = "Demo App"
summary = "a demo"
description = "a longer demo description"
screenshots = ["https://example.org/a.png"]
maintainer = "key1"
{extra}

[[source]]
type = "git"
url = "https://github.com/example/demo"
commit = "0000000000000000000000000000000000000000"
"#
        );
        arlen_forage_recipe::parse(&text).expect("valid recipe")
    }

    #[test]
    fn maps_recipe_metadata_to_the_display() {
        let r = recipe_toml("org.demo.App", "");
        let e = forage_entry(&r, SourceLayer::Official, None);
        assert_eq!(e.id, ComponentId("org.demo.App".into()));
        assert_eq!(e.layer, SourceLayer::Official);
        assert_eq!(e.display.name, "Demo App");
        assert_eq!(e.display.summary.as_deref(), Some("a demo"));
        assert_eq!(e.display.screenshots.len(), 1);
    }

    #[test]
    fn coarse_capability_labels_enable_faceting() {
        let r = recipe_toml(
            "org.demo.App",
            "[capabilities]\nnetwork = [\"example.org:443\"]\nnotifications = true\ngraph = [\"read:File\"]",
        );
        let e = forage_entry(&r, SourceLayer::Community, None);
        // Sorted + deduped coarse categories.
        assert_eq!(e.capabilities.capabilities, vec!["network", "notifications", "read:File"]);
    }

    #[test]
    fn a_recipe_from_a_pinned_cookbook_names_its_chain_and_its_publisher() {
        let recipe = arlen_forage_recipe::parse(FORAGE_TOML).unwrap();
        let origin = CookbookOrigin { name: "arlen-official".into(), pinned_root: true };
        let entry = forage_entry(&recipe, SourceLayer::Official, Some(&origin));
        let a = entry.trust.attestation.expect("a tracked cookbook has a chain");
        assert_eq!(a.chain, "tuf");
        assert_eq!(a.signer, "arlen-official");
        assert!(a.pinned_here);
        // The publisher a forage app has is that same cookbook.
        assert_eq!(entry.trust.verified_publisher.as_deref(), Some("arlen-official"));
    }

    /// A tracked cookbook without a signed root still names its publisher, but
    /// the row must not read as pinned - nothing here is holding it to a chain.
    #[test]
    fn an_unpinned_cookbook_is_named_but_not_pinned() {
        let recipe = arlen_forage_recipe::parse(FORAGE_TOML).unwrap();
        let origin = CookbookOrigin { name: "local-notes".into(), pinned_root: false };
        let entry = forage_entry(&recipe, SourceLayer::Personal, Some(&origin));
        assert!(!entry.trust.attestation.expect("still a named signer").pinned_here);
    }

    /// A recipe that reached the catalog outside any cookbook: no publisher and
    /// no chain, rather than an invented one.
    #[test]
    fn a_recipe_with_no_cookbook_claims_nothing() {
        let recipe = arlen_forage_recipe::parse(FORAGE_TOML).unwrap();
        let entry = forage_entry(&recipe, SourceLayer::Official, None);
        assert!(entry.trust.attestation.is_none());
        assert!(entry.trust.verified_publisher.is_none());
    }

    #[test]
    fn an_unverified_reproducible_status_hides_the_row() {
        // No [reproducible] block -> None (hidden).
        let r = recipe_toml("org.demo.App", "");
        assert!(forage_entry(&r, SourceLayer::Official, None).trust.reproducible_build.is_none());
        // An explicit verified status is shown.
        let r = recipe_toml("org.demo.App", "[reproducible]\nstatus = \"verified\"");
        assert_eq!(
            forage_entry(&r, SourceLayer::Official, None).trust.reproducible_build.as_deref(),
            Some("verified")
        );
    }

    #[test]
    fn a_forage_entry_flows_through_the_merge() {
        let r = recipe_toml("org.demo.App", "");
        let cards = merge_catalog(vec![forage_entry(&r, SourceLayer::Official, None)]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Official);
    }

    const FLATHUB_XML: &str = r#"<?xml version="1.0"?>
<components>
  <component type="desktop-application">
    <id>org.gnome.Calculator</id>
    <name>Calculator</name>
    <name xml:lang="de">Taschenrechner</name>
    <summary>Do calculations</summary>
    <summary xml:lang="de">Rechnen</summary>
    <description><p>A powerful calculator.</p><p>It has modes.</p></description>
    <icon type="cached">org.gnome.Calculator.png</icon>
    <icon type="remote">https://dl.flathub.org/icon.png</icon>
    <screenshots>
      <screenshot type="default"><image>https://dl.flathub.org/a.png</image></screenshot>
      <screenshot><image>https://dl.flathub.org/b.png</image></screenshot>
    </screenshots>
  </component>
  <component type="desktop-application">
    <name>No Id</name>
  </component>
</components>"#;

    /// AppStream lists releases newest-first by convention, but the timestamp is
    /// the fact. Trusting document order over a stamp would report an OLD version
    /// as available and leave a current install looking outdated forever.
    #[test]
    fn the_newest_stamped_release_wins_over_document_order() {
        let xml = r#"<components>
          <component type="desktop-application">
            <id>org.example.App</id>
            <releases>
              <release version="1.0" timestamp="1600000000"/>
              <release version="3.0" timestamp="1700000000"/>
              <release version="2.0" timestamp="1650000000"/>
            </releases>
          </component>
        </components>"#;
        let entries = flathub_entries(xml).unwrap();
        assert_eq!(entries[0].version, "3.0");
    }

    /// A pre-release is not what a user gets by updating; offering it as the
    /// available version would push them onto a track they never chose.
    #[test]
    fn a_development_release_is_not_the_available_version() {
        let xml = r#"<components>
          <component type="desktop-application">
            <id>org.example.App</id>
            <releases>
              <release version="4.0-beta" type="development" timestamp="1800000000"/>
              <release version="3.0" timestamp="1700000000"/>
            </releases>
          </component>
        </components>"#;
        assert_eq!(flathub_entries(xml).unwrap()[0].version, "3.0");
    }

    /// Unstamped releases fall back to document order, which is AppStream's
    /// newest-first convention.
    #[test]
    fn unstamped_releases_keep_the_newest_first_convention() {
        let xml = r#"<components>
          <component type="desktop-application">
            <id>org.example.App</id>
            <releases>
              <release version="2.0"/>
              <release version="1.0"/>
            </releases>
          </component>
        </components>"#;
        assert_eq!(flathub_entries(xml).unwrap()[0].version, "2.0");
    }

    /// A component stating no release must yield empty, which `outdated` reads as
    /// nothing to compare rather than as a change.
    #[test]
    fn a_component_without_releases_states_no_version() {
        let entries = flathub_entries(FLATHUB_XML).unwrap();
        assert_eq!(entries[0].version, "");
    }

    #[test]
    fn a_dep11_component_takes_its_newest_stable_release() {
        let yaml = "---\nID: org.example.App\nName:\n  C: App\nReleases:\n  - version: '9.0'\n    type: development\n    unix_timestamp: 1800000000\n  - version: '2.0'\n    unix_timestamp: 1700000000\n  - version: '1.0'\n    unix_timestamp: 1600000000\n";
        let entries = dep11_entries(&yaml.into());
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].version, "2.0");
    }

    #[test]
    fn flathub_reader_maps_the_unlocalized_appstream_fields() {
        let entries = flathub_entries(FLATHUB_XML).unwrap();
        // The id-less component is skipped.
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, ComponentId("org.gnome.Calculator".into()));
        assert_eq!(e.layer, SourceLayer::Flatpak);
        assert_eq!(e.display.name, "Calculator", "the unlocalized name, not the de one");
        assert_eq!(e.display.summary.as_deref(), Some("Do calculations"));
        assert_eq!(
            e.display.description.as_deref(),
            Some("A powerful calculator.\n\nIt has modes.")
        );
        assert_eq!(e.display.screenshots, vec!["https://dl.flathub.org/a.png", "https://dl.flathub.org/b.png"]);
        // The remote icon wins over the cached one.
        assert_eq!(e.display.icon.as_deref(), Some("https://dl.flathub.org/icon.png"));
    }

    #[test]
    fn flathub_reader_rejects_malformed_xml() {
        assert!(matches!(flathub_entries("<components><oops"), Err(ComposeError::Xml(_))));
    }

    /// One installed app's MetaInfo, the shape `/usr/share/metainfo` actually
    /// holds: a single `<component>`, localized paragraphs INSIDE one
    /// `<description>`, and no install route.
    const METAINFO_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>com.example.Editor</id>
  <name>Editor</name>
  <summary>An editor</summary>
  <description>
    <p>The English paragraph.</p>
    <p xml:lang="ar">The Arabic paragraph.</p>
  </description>
</component>"#;

    #[test]
    fn an_installed_apps_metainfo_becomes_a_card_that_cannot_be_installed() {
        let entry = metainfo_entry(METAINFO_XML).expect("a desktop app should compose");
        assert_eq!(entry.id.0, "com.example.Editor");
        assert_eq!(entry.layer, SourceLayer::Native);
        assert_eq!(entry.display.name, "Editor");
        // The store has no route to a distribution package, so it must not offer one.
        assert_eq!(entry.install_handle, None);
    }

    #[test]
    fn a_localized_paragraph_stays_out_of_the_description() {
        let entry = metainfo_entry(METAINFO_XML).unwrap();
        let description = entry.display.description.expect("the English text");
        assert_eq!(description, "The English paragraph.");
        assert!(!description.contains("Arabic"));
    }

    #[test]
    fn metainfo_that_is_not_an_app_composes_nothing() {
        for kind in ["addon", "font", "console-application", "runtime"] {
            let xml = METAINFO_XML.replace("desktop-application", kind);
            assert!(
                metainfo_entry(&xml).is_none(),
                "a {kind} component is not an app card"
            );
        }
    }

    #[test]
    fn a_native_card_never_outranks_a_real_install_route() {
        // The derived Ord puts Native last, so merging an installed app with the
        // same app from Flathub keeps the installable variant as the default.
        let mut entries = flathub_entries(FLATHUB_XML).unwrap();
        let id = entries[0].id.0.clone();
        let mut native = metainfo_entry(METAINFO_XML).unwrap();
        native.id = crate::catalog::ComponentId(id);
        entries.push(native);
        let cards = merge_catalog(entries);
        assert_eq!(cards.len(), 1, "one id is one card");
        assert_eq!(cards[0].variants[cards[0].default_variant].layer, SourceLayer::Flatpak);
    }

    /// A real `appstreamcli compose` product, copied verbatim off the tool
    /// (1.1.5) rather than written to look like one. Note what is NOT here: no
    /// `origin` attribute, though the run was given `--origin=forage`, and no
    /// `<pkgname>`, because a catalogue composed from a staging tree has no
    /// package. Both are why the reader takes the origin from the filename and
    /// leaves the install route to the recipe.
    const COMPOSED_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<components version="1.0">
  <component type="desktop-application">
    <id>org.example.demo</id>
    <name>Demo</name>
    <summary>A demo</summary>
    <project_license>MIT</project_license>
    <description>
      <p>Demo app.</p>
    </description>
    <launchable type="desktop-id">org.example.demo.desktop</launchable>
    <icon type="cached" width="64" height="64">org.example.demo.png</icon>
    <icon type="stock">org.example.demo</icon>
    <categories>
      <category>Utility</category>
    </categories>
  </component>
</components>"#;

    #[test]
    fn an_xml_catalogue_screenshot_gets_the_base_the_document_declares() {
        // The XML form states the base as an attribute on `<components>` where
        // DEP-11 states it in its header. Flathub's file needs none - its URLs
        // are absolute - so this guards the case a mirrored catalogue would make.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<components version="1.0" media_baseurl="https://media.example/base">
  <component type="desktop-application">
    <id>org.example.demo</id>
    <name>Demo</name>
    <icon type="cached">org.example.demo.png</icon>
    <screenshots>
      <screenshot type="default">
        <image type="source">shots/one.png</image>
      </screenshot>
    </screenshots>
  </component>
</components>"#;
        let entries = catalog_entries(xml, SourceLayer::Apt, false).unwrap();
        assert_eq!(
            entries[0].display.screenshots,
            vec!["https://media.example/base/shots/one.png"],
        );
        // The cached icon is a NAME and must not be turned into a URL by it.
        assert_eq!(entries[0].display.icon.as_deref(), Some("org.example.demo.png"));
    }

    #[test]
    fn a_composed_catalogue_lands_at_its_origin_layer_with_a_local_icon() {
        let entries = catalog_entries(COMPOSED_XML, SourceLayer::Official, false).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].layer, SourceLayer::Official);
        assert_eq!(entries[0].display.name, "Demo");
        assert_eq!(
            entries[0].display.icon.as_deref(),
            Some("org.example.demo.png"),
            "the cached icon, which is the one on local disk",
        );
        assert!(
            entries[0].install_handle.is_none(),
            "no package in the document, so no install route to claim",
        );
    }

    #[test]
    fn a_forage_catalogue_takes_its_layer_from_the_recipe_and_an_unknown_one_is_the_archive() {
        assert_eq!(
            layer_for_catalog_origin(arlen_forage_recipe::CATALOG_ORIGIN, Some(SourceLayer::Community)),
            Some(SourceLayer::Community),
        );
        assert_eq!(
            layer_for_catalog_origin(arlen_forage_recipe::CATALOG_ORIGIN, None),
            None,
            "no cookbook offers it, so there is no install route to file it under",
        );
        // The same layer DEP-11 gets, so the two serialisations of one archive
        // catalogue cannot show up as two variants of the same app. The recipe
        // layer is ignored here on purpose: an app that has both a recipe and a
        // .deb must not have its apt entry filed as a cookbook install.
        assert_eq!(
            layer_for_catalog_origin("debian_main", Some(SourceLayer::Official)),
            Some(SourceLayer::Apt),
        );
    }

    /// The whole way across: an installed app's directory, laid out the way
    /// installd lays one out, with the catalogue gzipped the way
    /// `appstreamcli compose` writes it. Everything before this fed compose a
    /// fixture STRING, which cannot catch a path that is never scanned or a `.gz`
    /// that is never decompressed.
    #[test]
    fn an_installed_app_reaches_the_catalogue_from_disk() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let sw = dir.path().join("apps/org.example.demo/share/swcatalog");
        std::fs::create_dir_all(sw.join("xml")).unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(COMPOSED_XML.as_bytes()).unwrap();
        std::fs::write(sw.join("xml/forage.xml.gz"), enc.finish().unwrap()).unwrap();
        // The icon compose extracted, where compose puts it.
        let icons = sw.join("icons/forage/64x64");
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(icons.join("org.example.demo.png"), b"png").unwrap();

        let roots = crate::discover::SourceRoots {
            flatpak_dirs: vec![],
            dep11_dirs: vec![],
            metainfo_dirs: vec![],
            profiles_dir: dir.path().join("permissions"),
            apps_dir: dir.path().join("apps"),
        };
        let found = crate::discover::discover(&roots);
        let catalog_xml: Vec<CatalogInput> = found
            .catalog_xml
            .iter()
            .filter_map(|(o, p)| {
                Some(CatalogInput {
                    text: crate::discover::read_catalog(p)?,
                    root: p.parent().and_then(|d| d.parent()).map(|d| d.to_path_buf()),
                    origin: Some(o.clone()),
                })
            })
            .collect();
        assert_eq!(catalog_xml.len(), 1, "found and read: {found:?}");

        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml,
            forage: vec![(
                r#"
[recipe]
id = "org.example.demo"
name = "Demo"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/demo"
commit = "0000000000000000000000000000000000000000"
"#
                .into(),
                SourceLayer::Community,
                None,
            )],
            flathub_xml: Vec::new(),
            dep11_yaml: Vec::new(),
            flatpak_metadata: Vec::new(),
            apt_profiles: Vec::new(),
            metainfo_xml: Vec::new(),
        });
        let card = catalog
            .card(&ComponentId("org.example.demo".into()))
            .expect("the installed app has a card");
        assert_eq!(
            card.display.icon.as_deref(),
            Some(icons.join("org.example.demo.png").to_string_lossy().as_ref()),
            "the name in the document, resolved to the file it names",
        );
        assert_eq!(card.variants.len(), 1);
        assert_eq!(card.variants[0].layer, SourceLayer::Community);
    }

    #[test]
    fn a_forage_catalogue_for_an_app_no_cookbook_offers_is_left_out() {
        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml: vec![CatalogInput {
                text: COMPOSED_XML.into(),
                origin: Some(arlen_forage_recipe::CATALOG_ORIGIN.into()),
                ..Default::default()
            }],
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: Vec::new(),
            flatpak_metadata: Vec::new(),
            apt_profiles: Vec::new(),
            metainfo_xml: Vec::new(),
        });
        assert!(
            catalog.card(&ComponentId("org.example.demo".into())).is_none(),
            "a card with no install route is worse than no card",
        );
    }

    #[test]
    fn a_composed_catalogue_gives_a_recipe_its_pictures_without_taking_the_install() {
        let catalog = compose_catalog(SourceInputs {
            odrs: None,
            catalog_xml: vec![CatalogInput {
                text: COMPOSED_XML.into(),
                origin: Some(arlen_forage_recipe::CATALOG_ORIGIN.into()),
                ..Default::default()
            }],
            forage: vec![(
                r#"
[recipe]
id = "org.example.demo"
name = "Demo"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/demo"
commit = "0000000000000000000000000000000000000000"
"#
                .into(),
                SourceLayer::Official,
                None,
            )],
            flathub_xml: Vec::new(),
            dep11_yaml: Vec::new(),
            flatpak_metadata: Vec::new(),
            apt_profiles: Vec::new(),
            metainfo_xml: Vec::new(),
        });
        let card = catalog
            .card(&ComponentId("org.example.demo".into()))
            .expect("one card for the app");
        assert_eq!(card.variants.len(), 1, "one Official variant, not two");
        assert_eq!(card.display.icon.as_deref(), Some("org.example.demo.png"));
        assert!(
            card.variants[0].install_handle.is_some(),
            "the recipe still says how to install it",
        );
    }

    #[test]
    fn flathub_entries_flow_through_the_merge() {
        let entries = flathub_entries(FLATHUB_XML).unwrap();
        let cards = merge_catalog(entries);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Flatpak);
    }

    const DEP11_YAML: &str = r#"File: DEP-11
Version: '0.8'
Origin: debian-bookworm-main
---
Type: desktop-application
ID: org.gnome.gedit
Name:
  C: Text Editor
  de: Texteditor
Summary:
  C: Edit text files
Description:
  C: <p>A GNOME text editor.</p>
Icon:
  cached:
    - name: org.gnome.gedit.png
      width: 64
  remote:
    - url: https://debian.example/gedit.png
Screenshots:
  - default: true
    source-image:
      url: https://debian.example/shot.png
---
Type: desktop-application
Name:
  C: No Id App
"#;

    #[test]
    fn dep11_reader_maps_the_c_locale_fields() {
        let entries = dep11_entries(&DEP11_YAML.into());
        // Header doc + the id-less record are both skipped.
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, ComponentId("org.gnome.gedit".into()));
        assert_eq!(e.layer, SourceLayer::Apt);
        assert_eq!(e.display.name, "Text Editor", "the C locale, not the de one");
        assert_eq!(e.display.summary.as_deref(), Some("Edit text files"));
        assert_eq!(e.display.description.as_deref(), Some("<p>A GNOME text editor.</p>"));
        assert_eq!(e.display.screenshots, vec!["https://debian.example/shot.png"]);
        // No icon cache to look in (this is a fixture, so the input carries no
        // root), and a bare cached name is not something a renderer can open. The
        // remote URL is what is left, and it is at least loadable.
        assert_eq!(e.display.icon.as_deref(), Some("https://debian.example/gedit.png"));
    }

    #[test]
    fn a_dep11_cached_icon_becomes_the_path_of_the_file_it_names() {
        let dir = tempfile::tempdir().unwrap();
        // The layout the archive's `icons-*.tar.gz` unpacks into, and the one the
        // image carries: `<root>/icons/<origin>/<size>/<name>`. Both sizes, to show
        // 128 wins.
        for size in ["64x64", "128x128"] {
            let d = dir.path().join("icons/debian-bookworm-main").join(size);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("org.gnome.gedit.png"), b"png").unwrap();
        }
        let entries = dep11_entries(&CatalogInput {
            text: DEP11_YAML.into(),
            root: Some(dir.path().to_path_buf()),
            origin: None, // Read from the header, not passed in.
        });
        assert_eq!(
            entries[0].display.icon.as_deref(),
            Some(
                dir.path()
                    .join("icons/debian-bookworm-main/128x128/org.gnome.gedit.png")
                    .to_string_lossy()
                    .as_ref()
            ),
        );
    }

    #[test]
    fn a_relative_screenshot_gets_the_base_the_header_declares() {
        // The shape EVERY screenshot in the Debian archive has: a path relative to
        // the header's `MediaBaseUrl`. Handed on as-is it is a URL with nothing in
        // front of it, which is a broken image on every detail page that has one.
        let yaml = r#"File: DEP-11
Version: '1.0'
Origin: debian-trixie-main
MediaBaseUrl: https://appstream.debian.org/media/trixie
---
Type: desktop-application
ID: org.gnome.Secrets
Name:
  C: Secrets
Screenshots:
- default: true
  source-image:
    url: org/gnome/Secrets/d02/screenshots/image-1_orig.png
"#;
        let entries = dep11_entries(&yaml.into());
        assert_eq!(
            entries[0].display.screenshots,
            vec!["https://appstream.debian.org/media/trixie/org/gnome/Secrets/d02/screenshots/image-1_orig.png"],
        );
    }

    #[test]
    fn an_absolute_screenshot_is_left_alone_and_a_baseless_one_is_kept() {
        assert_eq!(
            absolute_media_url("https://elsewhere/a.png".into(), Some("https://base")),
            "https://elsewhere/a.png",
        );
        // Nothing better to do than hand it on: a caller with its own base can
        // still use it, and dropping it would lose the picture outright.
        assert_eq!(absolute_media_url("a/b.png".into(), None), "a/b.png");
        // One slash, whichever side wrote one.
        assert_eq!(absolute_media_url("/a.png".into(), Some("https://base/")), "https://base/a.png");
    }

    #[test]
    fn an_origin_the_filename_would_have_got_wrong_comes_off_the_header() {
        // `trixie_main.yml.gz` is filed under `debian-trixie-main`. Anything derived
        // from the filename misses every icon in the archive.
        assert_eq!(
            dep11_header_field(DEP11_YAML, "Origin").as_deref(),
            Some("debian-bookworm-main"),
        );
        assert_eq!(dep11_header_field("File: DEP-11\nVersion: '1.0'\n", "Origin"), None);
    }

    #[test]
    fn dep11_reader_skips_a_corrupt_record_and_keeps_the_rest() {
        // A real DEP-11 catalog is thousands of documents; one non-conformant record
        // (here a `Name:` that is a scalar, not the expected locale map) must not
        // drop every Debian app. The two well-formed records survive it.
        let yaml = r#"File: DEP-11
---
Type: desktop-application
ID: good.one
Name:
  C: Good One
---
Type: desktop-application
ID: bad.two
Name: "not a locale map"
---
Type: desktop-application
ID: good.three
Name:
  C: Good Three
"#;
        let entries = dep11_entries(&yaml.into());
        let ids: Vec<&str> = entries.iter().map(|e| e.id.0.as_str()).collect();
        assert_eq!(ids, vec!["good.one", "good.three"], "the corrupt record is skipped, the rest kept");
    }

    #[test]
    fn dep11_entries_flow_through_the_merge() {
        let cards = merge_catalog(dep11_entries(&DEP11_YAML.into()));
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].variants[0].layer, SourceLayer::Apt);
    }

    /// A bridge recipe: `[bridge]` plus the two-halves `[install]` it requires.
    const BRIDGE_TOML: &str = r#"
[recipe]
id = "org.forage.ObsidianBridge"
name = "Obsidian Bridge"
summary = "bridges obsidian"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/bridge"
commit = "0000000000000000000000000000000000000000"

[bridge]
foreign_app = "obsidian"

[install]
arlen_side = ["entities.toml", "bridge.toml"]

[install.foreign_side]
into = "$VAULT/.obsidian/plugins/md-obsidian-bridge/"
files = ["main.js", "manifest.json"]
"#;

    const FORAGE_TOML: &str = r#"
[recipe]
id = "org.forage.Tool"
name = "Forage Tool"
summary = "a tool"
maintainer = "key1"

[[source]]
type = "git"
url = "https://github.com/example/tool"
commit = "0000000000000000000000000000000000000000"
"#;

    #[test]
    fn compose_catalog_merges_every_source() {
        let inputs = SourceInputs {
            forage: vec![(FORAGE_TOML.to_string(), SourceLayer::Community, None)],
            flathub_xml: vec![FLATHUB_XML.to_string()],
            dep11_yaml: vec![DEP11_YAML.into()],
            ..Default::default()
        };
        let catalog = compose_catalog(inputs);
        // One forage + one Flathub + one DEP-11 app, all distinct ids -> 3 cards.
        assert!(catalog.card(&ComponentId("org.forage.Tool".into())).is_some());
        assert!(catalog.card(&ComponentId("org.gnome.Calculator".into())).is_some());
        assert!(catalog.card(&ComponentId("org.gnome.gedit".into())).is_some());
    }

    /// Debian splits its catalog per suite and component, so `main`, `contrib`
    /// and `non-free` arrive as separate documents. Reading only one would serve
    /// a fraction of what is installable while looking like the whole store.
    #[test]
    fn every_dep11_document_contributes_its_apps() {
        let second = DEP11_YAML.replace("org.gnome.gedit", "org.gnome.Maps");
        let inputs = SourceInputs {
            dep11_yaml: vec![DEP11_YAML.into(), second.into()],
            ..Default::default()
        };
        let catalog = compose_catalog(inputs);
        assert!(catalog.card(&ComponentId("org.gnome.gedit".into())).is_some());
        assert!(
            catalog.card(&ComponentId("org.gnome.Maps".into())).is_some(),
            "the second catalog's app is missing, so only one document was read"
        );
    }

    #[test]
    fn compose_catalog_skips_a_malformed_source() {
        let inputs = SourceInputs {
            forage: vec![("this is not valid toml {{{".to_string(), SourceLayer::Personal, None)],
            flathub_xml: vec!["<not xml".to_string()],
            dep11_yaml: vec![DEP11_YAML.into()],
            ..Default::default()
        };
        // The bad forage + bad XML are skipped; the good DEP-11 app still lands.
        let catalog = compose_catalog(inputs);
        assert!(catalog.card(&ComponentId("org.gnome.gedit".into())).is_some());
    }
    /// A recipe carrying `[bridge]` is a bridge, so the store can browse it as
    /// one (store-app.md section 8b). Without this the card model has no way to
    /// tell a bridge from a standalone app.
    #[test]
    fn a_bridge_recipe_produces_a_bridge_entry() {
        let recipe = arlen_forage_recipe::parse(BRIDGE_TOML).unwrap();
        let entry = forage_entry(&recipe, SourceLayer::Community, None);
        assert_eq!(entry.kind, ItemKind::Bridge);
    }

    /// An ordinary recipe must NOT be labelled a bridge.
    #[test]
    fn a_plain_recipe_produces_an_app_entry() {
        let recipe = arlen_forage_recipe::parse(FORAGE_TOML).unwrap();
        let entry = forage_entry(&recipe, SourceLayer::Community, None);
        assert_eq!(entry.kind, ItemKind::App);
    }

    /// The Flathub and DEP-11 sources cannot express a bridge, so they must
    /// default to App rather than inherit anything.
    #[test]
    fn foreign_sources_default_to_app() {
        let flathub = flathub_entries(FLATHUB_XML).unwrap();
        assert!(flathub.iter().all(|e| e.kind == ItemKind::App));
        let dep11 = dep11_entries(&DEP11_YAML.into());
        assert!(dep11.iter().all(|e| e.kind == ItemKind::App));
    }

}
