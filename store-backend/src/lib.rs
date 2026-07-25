//! Arlen app store backend (store-app.md section 9): the catalog merge/dedup model
//! (ST-2/ST-3 spine) that turns per-layer app entries (Flatpak, apt `.deb`, forage)
//! into one merged card per AppStream component-id with per-source install variants.
//!
//! This crate is the sovereign, local data model the `org.arlen.Store1` backend
//! serves and the `apps/store` frontend renders. The merge is pure (no I/O), so the
//! "one card, never N duplicates, never a silent pick" invariant is tested directly.

pub mod catalog;

pub use catalog::{
    merge_catalog, AppCard, CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta,
    SourceLayer, TrustSignals, Variant,
};
