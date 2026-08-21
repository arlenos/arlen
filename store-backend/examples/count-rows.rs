//! Count what the store would show on THIS machine, without opening a window.
//!
//! The planner asked in one line whether the grid actually has rows where
//! `/usr/share/metainfo` is populated, because the rule in `store-app.md` is that
//! a store feature counts when rows appear. A backend composing nothing and a
//! backend composing eighty components are two different problems, and the number
//! says which one we have.

use std::path::PathBuf;

use arlen_store_backend::{compose_catalog, CatalogInput, SourceInputs};

fn read(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| arlen_store_backend::discover::read_catalog(p))
        .collect()
}

fn main() {
    let found = arlen_store_backend::discover::discover(&Default::default());
    println!(
        "on disk: {} dep11, {} composed xml, {} metainfo, {} flatpak, {} apt profiles",
        found.dep11_yaml.len(),
        found.catalog_xml.len(),
        found.metainfo_xml.len(),
        found.flatpak_metadata.len(),
        found.apt_profiles.len(),
    );

    let catalog = compose_catalog(SourceInputs {
        odrs: None,
        forage: Vec::new(),
        flathub_xml: read(&found.flathub_xml),
        dep11_yaml: read(&found.dep11_yaml).into_iter().map(CatalogInput::from).collect(),
        catalog_xml: Vec::new(),
        metainfo_xml: read(&found.metainfo_xml),
        flatpak_metadata: Vec::new(),
        apt_profiles: Vec::new(),
    });

    let cards = catalog.search("", &[]);
    let with_icon = cards.iter().filter(|c| c.display.icon.is_some()).count();
    let with_shots = cards.iter().filter(|c| !c.display.screenshots.is_empty()).count();
    println!("cards the grid would show: {}", cards.len());
    println!("  with an icon: {with_icon}");
    println!("  with at least one screenshot: {with_shots}");
    for c in cards.iter().take(6) {
        println!(
            "  {:<28} icon {:<7} {} screenshot(s)",
            c.display.name,
            if c.display.icon.is_some() { "yes" } else { "no" },
            c.display.screenshots.len()
        );
    }
}
