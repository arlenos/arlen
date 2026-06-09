//! Context Capsule materialization, daemon side (context-capsule.md §4, loader
//! option (b)).
//!
//! The knowledge daemon owns the graph and the bitemporal as-of read, so it
//! materializes a capsule's frozen slice server-side: given a
//! [`arlen_capsule::scope::CapsuleScope`] and a mint time, it expands the scope
//! over the graph, reads each included node's projected fields and the edges among
//! them as of `T_mint`, and returns the [`arlen_capsule::slice::FrozenSlice`] the
//! caller (`capsuled`) content-addresses and signs. This keeps the as-of read and
//! the projection where the graph is, rather than duplicating the bitemporal
//! `valid_as_of` logic in an outward reader.
//!
//! This module is being built bottom-up. The first piece is the pure cell
//! conversion; the graph-backed scope expansion, the projected as-of node load and
//! the read op follow.

// API ahead of its consumer: the cell conversion is used by the capsule read op
// (the next piece) and by tests; until the op wires it into the bin tree it reads
// unused in a plain build, like the other not-yet-consumed daemon read APIs.
#![allow(dead_code)]

use arlen_capsule::slice::SliceValue;

use crate::graph::CellValue;

/// Map a graph [`CellValue`] to a frozen-slice [`SliceValue`], or `None` if the
/// value cannot be carried in the canonical frozen form.
///
/// `String`/`Int64`/`Bool`/`Null` map directly (a genuine null field stays a
/// present-but-null `SliceValue::Null`). A `Float` maps to `None`: the canonical
/// slice form deliberately carries no floating point (its byte form must be
/// deterministic for content addressing, and cross-platform float formatting is
/// not), so a float-typed field is **omitted** from the slice rather than encoded
/// lossily. Timestamps are stored as `Int64` epoch microseconds in the graph, so
/// they map faithfully to `SliceValue::Int`; only genuine floats are dropped, a
/// documented frozen-form limit (§4).
pub(crate) fn cell_to_slice_value(cell: &CellValue) -> Option<SliceValue> {
    match cell {
        CellValue::String(s) => Some(SliceValue::Text(s.clone())),
        CellValue::Int64(i) => Some(SliceValue::Int(*i)),
        CellValue::Bool(b) => Some(SliceValue::Bool(*b)),
        CellValue::Null => Some(SliceValue::Null),
        CellValue::Float(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_carriable_cell_types() {
        assert_eq!(
            cell_to_slice_value(&CellValue::String("x".into())),
            Some(SliceValue::Text("x".into()))
        );
        assert_eq!(
            cell_to_slice_value(&CellValue::Int64(-7)),
            Some(SliceValue::Int(-7))
        );
        assert_eq!(
            cell_to_slice_value(&CellValue::Bool(true)),
            Some(SliceValue::Bool(true))
        );
        // A genuine null is a present-but-null field, distinct from an omitted one.
        assert_eq!(cell_to_slice_value(&CellValue::Null), Some(SliceValue::Null));
    }

    #[test]
    fn drops_a_float_field_to_keep_the_form_deterministic() {
        assert_eq!(cell_to_slice_value(&CellValue::Float(1.5)), None);
    }
}
