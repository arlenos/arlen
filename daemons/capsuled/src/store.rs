//! Storing the frozen slice as a content-addressed blob (context-capsule.md §3).
//!
//! The frozen slice is materialized once, under the originator's own authority,
//! and kept as a content-addressed, refcounted blob in the forage store. The
//! store hashes the bytes (`ContentHash::of` is sha256), `read` re-hashes on the
//! way out and fails closed on mismatch, and `put_referenced` stores and roots in
//! one lock hold so the blob is never collectable in the window before the grant
//! references it. The content hash IS the capsule's identity (§3); `release` + the
//! store's gc is the forget primitive a revoke uses (CC-R5).

use arlen_capsule::slice::FrozenSlice;
use arlen_forage_store::{ContentHash, Store, StoreError};

/// Store a [`FrozenSlice`] as a content-addressed, refcounted blob rooted to
/// `owner`, returning its content hash (the capsule's identity). The slice is
/// serialized canonically first ([`FrozenSlice::canonical_bytes`]), so the same
/// logical subgraph always addresses to the same blob; rooting to `owner` keeps it
/// from being collected before the capsule grant references it.
pub fn store_frozen_slice(
    store: &Store,
    slice: &FrozenSlice,
    owner: &str,
) -> Result<ContentHash, StoreError> {
    store.put_referenced(&slice.canonical_bytes(), owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_capsule::slice::{SliceNode, SliceRelation, SliceValue};
    use std::collections::BTreeMap;

    fn slice() -> FrozenSlice {
        let mut fields = BTreeMap::new();
        fields.insert("path".to_string(), SliceValue::Text("/a/b.rs".to_string()));
        FrozenSlice {
            nodes: vec![SliceNode {
                id: "f1".to_string(),
                label: "File".to_string(),
                fields,
            }],
            relations: vec![SliceRelation {
                from: "f1".to_string(),
                rel_type: "FILE_PART_OF".to_string(),
                to: "p1".to_string(),
            }],
        }
    }

    #[test]
    fn stores_and_reads_back_the_canonical_bytes() {
        let dir = std::env::temp_dir().join(format!("capsule-store-test-{}", std::process::id()));
        let store = Store::open(&dir).expect("open store");
        let s = slice();

        let hash = store_frozen_slice(&store, &s, "capsule:test").expect("store slice");
        // The identity is exactly the content hash of the canonical bytes.
        assert_eq!(hash, ContentHash::of(&s.canonical_bytes()));
        // The blob reads back byte-for-byte (the store re-hashes and would fail
        // closed on any mismatch).
        assert_eq!(store.read(&hash).expect("read blob"), s.canonical_bytes());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identical_slices_address_to_the_same_blob() {
        let dir = std::env::temp_dir().join(format!("capsule-store-dedup-{}", std::process::id()));
        let store = Store::open(&dir).expect("open store");

        let a = store_frozen_slice(&store, &slice(), "capsule:a").expect("store a");
        let b = store_frozen_slice(&store, &slice(), "capsule:b").expect("store b");
        assert_eq!(a, b, "the same logical slice is the same content-addressed blob");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
