//! The frozen slice and its canonical serialization (context-capsule.md §3-4).
//!
//! A capsule's identity is the content hash of its frozen slice, so the slice
//! must serialize to a **canonical** byte form: the same logical subgraph always
//! produces the same bytes, regardless of the order the graph happened to return
//! its rows. The existing `entities_to_jsonld` export does no key or element
//! sorting, so its output is not deterministic and cannot back a content hash;
//! this canonical serializer replaces it for the capsule path.
//!
//! This module is the pure serializer over an already-materialized slice. The
//! graph loader (a later piece) reads the scope's id manifest as of `T_mint`
//! through the bitemporal `valid_as_of` read, maps each cell to a [`SliceValue`],
//! applies the field projection, and builds the [`FrozenSlice`]; the forage store
//! then content-addresses [`FrozenSlice::canonical_bytes`].

use std::collections::BTreeMap;

use serde::Serialize;

/// A field value in a frozen slice. A small, canonically-serializable set:
/// timestamps are carried as their `i64` epoch-microsecond form ([`SliceValue::Int`],
/// "fixed-precision epoch micros" §4), never a float, so the canonical bytes are
/// deterministic. Floating-point fields are deliberately unsupported in the frozen
/// form (the loader stringifies or drops them) to keep the content hash stable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceValue {
    /// A text value.
    Text(String),
    /// An integer value, including timestamps as epoch microseconds.
    Int(i64),
    /// A boolean value.
    Bool(bool),
    /// An absent value.
    Null,
}

/// One node in a frozen slice: its id, label and projected fields. Fields are a
/// `BTreeMap`, so their keys serialize in sorted order with no work at write time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SliceNode {
    /// The node id (its stable graph identity).
    pub id: String,
    /// The node label (e.g. `File`, `Project`).
    pub label: String,
    /// The projected field set, sorted by key.
    pub fields: BTreeMap<String, SliceValue>,
}

/// One relation in a frozen slice: a typed edge between two included nodes. Both
/// endpoints are themselves in the slice (a capsule with dangling edges is not a
/// subgraph), enforced by the loader against the id manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SliceRelation {
    /// The source node id.
    pub from: String,
    /// The relation type.
    pub rel_type: String,
    /// The target node id.
    pub to: String,
}

/// A frozen slice: the scope-selected subgraph as of `T_mint`. Its
/// [`canonical_bytes`](FrozenSlice::canonical_bytes) is the content-addressed,
/// order-independent serialization the capsule's identity hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenSlice {
    /// The included nodes.
    pub nodes: Vec<SliceNode>,
    /// The relations among the included nodes.
    pub relations: Vec<SliceRelation>,
}

/// The canonical wire shape: sorted nodes and relations, serialized in a fixed
/// field order so the bytes depend only on the logical content.
#[derive(Serialize)]
struct Canonical<'a> {
    nodes: &'a [SliceNode],
    relations: &'a [SliceRelation],
}

impl FrozenSlice {
    /// The canonical byte serialization of the slice: nodes sorted by id,
    /// relations sorted by `(from, rel_type, to)`, field keys sorted (the
    /// `BTreeMap`), serialized to a deterministic JSON form. The same logical
    /// subgraph always yields the same bytes, so `(scope, T_mint)` content-
    /// addresses identically no matter the graph's row order. Node ids are unique
    /// in a slice, so the id sort is a total order; relations carry no payload, so
    /// the triple sort is total too.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut nodes = self.nodes.clone();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut relations = self.relations.clone();
        relations.sort();
        // serde_json over a fixed-field struct with sorted vecs and BTreeMap
        // fields is deterministic; values are Text/Int/Bool/Null only (no float),
        // so there is no precision ambiguity. Serialization cannot fail for these
        // plain owned types.
        serde_json::to_vec(&Canonical {
            nodes: &nodes,
            relations: &relations,
        })
        .expect("canonical slice serialization is infallible for plain owned data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str, fields: &[(&str, SliceValue)]) -> SliceNode {
        SliceNode {
            id: id.to_string(),
            label: label.to_string(),
            fields: fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        }
    }

    fn rel(from: &str, t: &str, to: &str) -> SliceRelation {
        SliceRelation {
            from: from.to_string(),
            rel_type: t.to_string(),
            to: to.to_string(),
        }
    }

    #[test]
    fn canonical_bytes_are_order_independent() {
        // Two slices with the same logical content but different node, relation
        // and field insertion order must serialize to identical bytes.
        let a = FrozenSlice {
            nodes: vec![
                node("p1", "Project", &[("name", SliceValue::Text("X".into())), ("size", SliceValue::Int(3))]),
                node("f1", "File", &[("path", SliceValue::Text("/a".into()))]),
            ],
            relations: vec![rel("f1", "FILE_PART_OF", "p1"), rel("f1", "ACCESSED_BY", "app")],
        };
        let b = FrozenSlice {
            nodes: vec![
                node("f1", "File", &[("path", SliceValue::Text("/a".into()))]),
                node("p1", "Project", &[("size", SliceValue::Int(3)), ("name", SliceValue::Text("X".into()))]),
            ],
            relations: vec![rel("f1", "ACCESSED_BY", "app"), rel("f1", "FILE_PART_OF", "p1")],
        };
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn canonical_bytes_differ_on_different_content() {
        let a = FrozenSlice {
            nodes: vec![node("f1", "File", &[("path", SliceValue::Text("/a".into()))])],
            relations: vec![],
        };
        let b = FrozenSlice {
            nodes: vec![node("f1", "File", &[("path", SliceValue::Text("/b".into()))])],
            relations: vec![],
        };
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
        // A different value type is also distinguished.
        let c = FrozenSlice {
            nodes: vec![node("f1", "File", &[("path", SliceValue::Null)])],
            relations: vec![],
        };
        assert_ne!(a.canonical_bytes(), c.canonical_bytes());
    }

    #[test]
    fn canonical_bytes_are_stable_across_calls() {
        let s = FrozenSlice {
            nodes: vec![node("n", "File", &[("k", SliceValue::Bool(true))])],
            relations: vec![rel("n", "R", "m")],
        };
        assert_eq!(s.canonical_bytes(), s.canonical_bytes());
    }
}
