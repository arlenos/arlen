//! The catalog compose step (store-app.md section 9.3): map each source's app
//! records into [`CatalogEntry`]s the merge consumes. This module holds the forage
//! adapter (Arlen-native, the recipe schema is local); the Flathub AppStream-XML and
//! Debian DEP-11-YAML readers land alongside it as they are built.
//!
//! Pure mapping, no I/O: given an already-parsed forage `Recipe` and the layer its
//! cookbook resolves to (personal/community/official), produce one entry. The recipe
//! carries the same AppStream metadata a client renders (`recipe.md` ST-1), so a
//! forage app is a first-class catalog citizen, not a second-class listing.

use arlen_forage_recipe::{Capabilities, Recipe, ReproducibleStatus};

use crate::catalog::{
    ItemKind,
    CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, SourceLayer, TrustSignals,
};

/// Map a parsed forage recipe to one catalog entry for the given source layer (the
/// tier its cookbook resolved to). Display comes from the recipe `[recipe]` metadata
/// (the same AppStream fields Flatpak/apt carry); the capability footprint is the
/// coarse categories the `[capabilities]` block declares (so a "needs network" /
/// "offline" facet is meaningful); the reproducible-build trust signal is populated
/// only when the recipe attests one (an unchecked status hides the row, section 9.2).
pub fn forage_entry(recipe: &Recipe, layer: SourceLayer) -> CatalogEntry {
    let meta = &recipe.recipe;
    let display = DisplayMeta {
        name: meta.name.clone(),
        summary: meta.summary.clone(),
        description: meta.description.clone(),
        screenshots: meta.screenshots.clone(),
        // A recipe declares no icon reference; the client falls back to a default.
        icon: None,
    };
    let capabilities = CapabilityFootprint {
        // The tier BADGE is a trust property of the cookbook, not the recipe; the
        // caller sets it from the cookbook's verification, so it stays None here.
        tier: None,
        capabilities: recipe.capabilities.as_ref().map(capability_labels).unwrap_or_default(),
    };
    let trust = TrustSignals {
        verified_publisher: None,
        reproducible_build: recipe.reproducible.as_ref().and_then(|r| match r.status {
            ReproducibleStatus::Verified => Some("verified".to_string()),
            ReproducibleStatus::Expected => Some("expected".to_string()),
            ReproducibleStatus::Unreproducible => Some("unreproducible".to_string()),
            // Not yet checked: hide the row rather than assert a status.
            ReproducibleStatus::Unverified => None,
        }),
        install_count: None,
        odrs_score: None,
        observed_vs_declared: None,
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

/// Parse a Flathub composed-AppStream catalog (`<components>` of `<component>`) into
/// one `CatalogEntry` per desktop app (`layer = Flatpak`). Display comes from the
/// AppStream fields (the UNLOCALIZED default element, ignoring `xml:lang` variants);
/// the capability footprint (Flatpak `finish-args`) and the trust signals (Flathub
/// verification / stats, ODRS) come from SEPARATE sources per section 9.2 and stay
/// empty here. A `<component>` with no id is skipped, not guessed.
pub fn flathub_entries(xml: &str) -> Result<Vec<CatalogEntry>, ComposeError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ComposeError::Xml(e.to_string()))?;
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
            screenshots: screenshot_urls(&component),
            icon: icon_ref(&component),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id),
            layer: SourceLayer::Flatpak,
            display,
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            version: latest_release_version(&component),
        });
    }
    Ok(entries)
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
fn description_text(node: &roxmltree::Node) -> Option<String> {
    let desc = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "description")
        .find(|c| !c.attributes().any(|a| a.name() == "lang"))?;
    let paras: Vec<&str> = desc
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "p")
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
pub fn dep11_entries(yaml: &str) -> Vec<CatalogEntry> {
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
                .collect(),
            icon: comp.icon.and_then(dep11_icon_ref),
        };
        entries.push(CatalogEntry {
            id: ComponentId(id),
            layer: SourceLayer::Apt,
            display,
            capabilities: CapabilityFootprint::default(),
            trust: TrustSignals::default(),
            kind: ItemKind::default(),
            version: dep11_release_version(&comp.releases),
        });
    }
    entries
}

/// The best icon reference from a DEP-11 `Icon` block: a remote URL the store can
/// fetch if present, else the first cached icon's name.
fn dep11_icon_ref(icon: Dep11Icon) -> Option<String> {
    icon.remote
        .and_then(|r| r.into_iter().find_map(|i| i.url))
        .or_else(|| icon.cached.and_then(|c| c.into_iter().find_map(|i| i.name)))
}

// --- compose orchestration (section 9.3: "produces the one merged model") --------

/// The already-read source contents the compose step merges. Held as text (not file
/// paths) so the orchestration is pure and testable; the daemon reads the files.
#[derive(Debug, Default)]
pub struct SourceInputs {
    /// `(recipe.toml text, the cookbook's resolved tier)` per forage recipe.
    pub forage: Vec<(String, SourceLayer)>,
    /// The Flathub composed-AppStream catalog XML, one per configured remote.
    ///
    /// A LIST, not one document: a machine can have several Flatpak remotes, and
    /// Debian splits its catalog per suite and component (`main`, `contrib`,
    /// `non-free` are separate files). Taking only the first would silently
    /// serve a fraction of what is installable while looking complete.
    pub flathub_xml: Vec<String>,
    /// The Debian DEP-11 catalog YAML, one per suite/component file. See
    /// [`SourceInputs::flathub_xml`] for why this is a list.
    pub dep11_yaml: Vec<String>,
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
}

/// Compose the merged [`Catalog`] from every configured source. Best-effort: a source
/// that fails to parse (one malformed recipe, an unreadable catalog) is SKIPPED, never
/// fatal, so a single bad input cannot blank the whole store. Returns the deduped,
/// merged catalog the `org.arlen.Store1` backend serves.
pub fn compose_catalog(inputs: SourceInputs) -> crate::query::Catalog {
    let mut entries = Vec::new();
    for (toml, layer) in &inputs.forage {
        if let Ok(recipe) = arlen_forage_recipe::parse(toml) {
            entries.push(forage_entry(&recipe, *layer));
        }
    }
    // Every remote and every suite/component, merged. The dedupe in
    // `merge_catalog` collapses an app carried by two of them into one card.
    for xml in &inputs.flathub_xml {
        if let Ok(mut es) = flathub_entries(xml) {
            fuse_flatpak_metadata(&mut es, &inputs.flatpak_metadata);
            entries.extend(es);
        }
    }
    for yaml in &inputs.dep11_yaml {
        // Best-effort per record: a corrupt document is skipped inside, never fatal.
        let mut es = dep11_entries(yaml);
        fuse_apt_profiles(&mut es, &inputs.apt_profiles);
        entries.extend(es);
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
        for entry in entries.iter_mut().filter(|e| e.id.0 == *id) {
            entry.capabilities.capabilities = labels.clone();
        }
    }
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
        let labels = crate::profile_caps::profile_labels(&profile);
        for entry in entries.iter_mut().filter(|e| e.id.0 == *id) {
            entry.capabilities.capabilities = labels.clone();
        }
    }
}

#[cfg(test)]
mod tests {
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
    fn an_apt_entry_takes_its_footprint_from_the_enrolled_profile() {
        let catalog = compose_catalog(SourceInputs {
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
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
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
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
            forage: vec![],
            flathub_xml: Vec::new(),
            dep11_yaml: vec![APT_YAML.into()],
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
        let e = forage_entry(&r, SourceLayer::Official);
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
        let e = forage_entry(&r, SourceLayer::Community);
        // Sorted + deduped coarse categories.
        assert_eq!(e.capabilities.capabilities, vec!["network", "notifications", "read:File"]);
    }

    #[test]
    fn an_unverified_reproducible_status_hides_the_row() {
        // No [reproducible] block -> None (hidden).
        let r = recipe_toml("org.demo.App", "");
        assert!(forage_entry(&r, SourceLayer::Official).trust.reproducible_build.is_none());
        // An explicit verified status is shown.
        let r = recipe_toml("org.demo.App", "[reproducible]\nstatus = \"verified\"");
        assert_eq!(
            forage_entry(&r, SourceLayer::Official).trust.reproducible_build.as_deref(),
            Some("verified")
        );
    }

    #[test]
    fn a_forage_entry_flows_through_the_merge() {
        let r = recipe_toml("org.demo.App", "");
        let cards = merge_catalog(vec![forage_entry(&r, SourceLayer::Official)]);
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
        let entries = dep11_entries(yaml);
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
        let entries = dep11_entries(DEP11_YAML);
        // Header doc + the id-less record are both skipped.
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, ComponentId("org.gnome.gedit".into()));
        assert_eq!(e.layer, SourceLayer::Apt);
        assert_eq!(e.display.name, "Text Editor", "the C locale, not the de one");
        assert_eq!(e.display.summary.as_deref(), Some("Edit text files"));
        assert_eq!(e.display.description.as_deref(), Some("<p>A GNOME text editor.</p>"));
        assert_eq!(e.display.screenshots, vec!["https://debian.example/shot.png"]);
        // The remote icon URL wins over the cached name.
        assert_eq!(e.display.icon.as_deref(), Some("https://debian.example/gedit.png"));
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
        let entries = dep11_entries(yaml);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.0.as_str()).collect();
        assert_eq!(ids, vec!["good.one", "good.three"], "the corrupt record is skipped, the rest kept");
    }

    #[test]
    fn dep11_entries_flow_through_the_merge() {
        let cards = merge_catalog(dep11_entries(DEP11_YAML));
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
            forage: vec![(FORAGE_TOML.to_string(), SourceLayer::Community)],
            flathub_xml: vec![FLATHUB_XML.to_string()],
            dep11_yaml: vec![DEP11_YAML.to_string()],
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
            dep11_yaml: vec![DEP11_YAML.to_string(), second],
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
            forage: vec![("this is not valid toml {{{".to_string(), SourceLayer::Personal)],
            flathub_xml: vec!["<not xml".to_string()],
            dep11_yaml: vec![DEP11_YAML.to_string()],
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
        let entry = forage_entry(&recipe, SourceLayer::Community);
        assert_eq!(entry.kind, ItemKind::Bridge);
    }

    /// An ordinary recipe must NOT be labelled a bridge.
    #[test]
    fn a_plain_recipe_produces_an_app_entry() {
        let recipe = arlen_forage_recipe::parse(FORAGE_TOML).unwrap();
        let entry = forage_entry(&recipe, SourceLayer::Community);
        assert_eq!(entry.kind, ItemKind::App);
    }

    /// The Flathub and DEP-11 sources cannot express a bridge, so they must
    /// default to App rather than inherit anything.
    #[test]
    fn foreign_sources_default_to_app() {
        let flathub = flathub_entries(FLATHUB_XML).unwrap();
        assert!(flathub.iter().all(|e| e.kind == ItemKind::App));
        let dep11 = dep11_entries(DEP11_YAML);
        assert!(dep11.iter().all(|e| e.kind == ItemKind::App));
    }

}
