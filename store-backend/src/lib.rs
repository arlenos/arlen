//! Arlen app store backend (store-app.md section 9): the catalog merge/dedup model
//! (ST-2/ST-3 spine) that turns per-layer app entries (Flatpak, apt `.deb`, forage)
//! into one merged card per AppStream component-id with per-source install variants.
//!
//! This crate is the sovereign, local data model the `org.arlen.Store1` backend
//! serves and the `apps/store` frontend renders. The merge is pure (no I/O), so the
//! "one card, never N duplicates, never a silent pick" invariant is tested directly.

pub mod catalog;
pub mod client;
pub mod compose;
pub mod flatpak;
pub mod query;
pub mod serve;
pub mod view;

pub use catalog::{
    merge_catalog, AppCard, CapabilityFootprint, CatalogEntry, ComponentId, DisplayMeta, ItemKind,
    SourceLayer, TrustSignals, Variant,
};
pub use compose::{compose_catalog, dep11_entries, flathub_entries, forage_entry, SourceInputs};
pub use client::{request as store_request, request_default, ClientError};
pub use query::{answer, CapabilityFacet, Catalog, ObservedStatus, Request, Response};
pub use view::{store_card, store_cards, StoreCard, Tier};
