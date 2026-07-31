//! Graph-drift merge primitives (graph-drift.md §2).
//!
//! The four bi-temporal wall-clock stamps (`valid_at`/`invalid_at`/`created_at`/
//! `expired_at`) are human-meaningful times, but they cannot safely ORDER writes
//! that originate on different devices: two machines' clocks disagree, and two
//! writes at the same wall-clock instant have no tiebreak. The hybrid logical
//! clock (HLC) here is the net-new merge order: it fuses physical micros with a
//! logical counter so concurrent same-instant writes still totally order, and it
//! is monotonic across a receive so a device that sees a future remote stamp
//! never issues an earlier one afterwards.
//!
//! Pure + net-new. Its consumers (a per-edge HLC column, `resolve_membership`'s
//! trust-then-HLC winner rule, the write-socket origin) land in later GD slices,
//! so this is `#[allow(dead_code)]` until then, built mechanism-before-trigger
//! like the executor/compensation/canary cores.
#![allow(dead_code)]

use crate::provenance::Provenance;

/// A hybrid logical clock timestamp (Kulkarni et al.): physical wall-clock
/// micros fused with a logical counter.
///
/// The ordering is `(physical, logical)` lexicographically, which the derived
/// `Ord` gives for free since the fields are declared in that order. Two HLCs
/// that compare equal are genuinely concurrent same-instant writes; the merge
/// layer breaks that tie with the asserting device id (a separate stable
/// column), never with the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    /// Physical wall-clock component, micros since the epoch (the same unit the
    /// bi-temporal stamps use). Carried so an HLC stays human-readable and a
    /// live clock still drives ordering when writes are far apart in real time.
    pub physical: u64,
    /// Logical counter, advanced when two events share a physical micro (or when
    /// a local clock has not moved since the last event) so ordering stays
    /// total. Reset to 0 whenever the physical component strictly advances.
    pub logical: u32,
}

impl Hlc {
    /// The zero clock: before any event. `tick`/`merge` from here adopt the
    /// supplied physical time.
    pub const ZERO: Hlc = Hlc {
        physical: 0,
        logical: 0,
    };

    /// Construct an HLC from its two components (e.g. reading a stored column).
    pub fn new(physical: u64, logical: u32) -> Hlc {
        Hlc { physical, logical }
    }

    /// Advance the clock for a LOCAL event at physical time `now` (micros).
    ///
    /// If wall time has moved past the last physical component the counter
    /// resets; otherwise (same micro, or a clock that has not advanced / has
    /// gone backwards) the counter increments so the new stamp still strictly
    /// follows the last. The result is always `>= self`, so a device never
    /// issues a non-increasing local stamp even under a backwards clock.
    pub fn tick(self, now: u64) -> Hlc {
        let physical = self.physical.max(now);
        let logical = if physical == self.physical {
            self.logical.saturating_add(1)
        } else {
            0
        };
        Hlc { physical, logical }
    }

    /// Advance the clock on RECEIVING a `remote` HLC, at local physical time
    /// `now` (micros). The standard HLC receive rule: take the max physical of
    /// all three, then pick the logical counter from whichever input(s) own that
    /// physical component, +1. The result is monotonic: strictly greater than
    /// both `self` and `remote`, so ordering never regresses after a merge.
    pub fn merge(self, remote: Hlc, now: u64) -> Hlc {
        let physical = self.physical.max(remote.physical).max(now);
        let logical = if physical == self.physical && physical == remote.physical {
            self.logical.max(remote.logical).saturating_add(1)
        } else if physical == self.physical {
            self.logical.saturating_add(1)
        } else if physical == remote.physical {
            remote.logical.saturating_add(1)
        } else {
            0
        };
        Hlc { physical, logical }
    }
}

/// A membership edge in a (possibly cross-device unioned) set: `from` is a
/// member of `to` under relation `rel`, asserted with `origin` at `hlc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipEdge {
    pub from: String,
    pub rel: String,
    pub to: String,
    pub origin: Provenance,
    pub hlc: Hlc,
}

impl MembershipEdge {
    /// The total-order key deciding the winner of a `(from, rel)` slot: the
    /// §5.6 trust rank first (a higher rank wins; an unranked `Graph`/unknown
    /// origin - `None` - sorts below EVERY ranked origin because `Option`
    /// orders `None < Some(_)`, so it never out-ranks a user/agent/model
    /// assertion on a later clock), then the HLC (the later write wins a trust
    /// tie), then a deterministic content tiebreak (`to`, `from`, origin key) so
    /// the winner is total even when trust and HLC coincide.
    ///
    /// Section 4 names a per-device id as the semantic breaker for a same-HLC
    /// cross-device clash. That column exists on the edge now, and this key
    /// still does not use it, deliberately: a candidate here carries no device
    /// id, and the case it would break is one where every other field already
    /// matches - same endpoints, same origin, same clock - so the two rows are
    /// the same assertion made twice and which one wins is immaterial. If a
    /// candidate ever carries the device id for another reason, revisit this
    /// rather than reaching for it because it is there.
    fn winner_key(&self) -> (Option<u8>, Hlc, &str, &str, &'static str) {
        (
            self.origin.trust_rank(),
            self.hlc,
            self.to.as_str(),
            self.from.as_str(),
            self.origin.as_key(),
        )
    }
}

/// The resolution of one `(from, rel)` slot: the single surviving edge and the
/// edges the merge must close to restore single-membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotResolution {
    /// The edge that stays live for this `(from, rel)`.
    pub winner: MembershipEdge,
    /// Every other edge in the slot; the caller closes each (append a close
    /// stamp, never delete - close-never-delete).
    pub closed: Vec<MembershipEdge>,
}

/// Resolve single-membership over a unioned edge set (graph-drift.md §2 / GD-R6).
///
/// The built single-writer close-then-append enforces "one live membership per
/// `(from, rel)`" only because it runs on the serial graph thread with one
/// writer. Union two devices' histories and each closed only what it could see,
/// so two live edges to different targets coexist. This pass restores the
/// invariant globally: it groups by `(from, rel)`, picks the deterministic
/// winner (trust rank, then HLC, then a content tiebreak) and returns every
/// other edge in the slot as `closed`. Slots are returned in sorted
/// `(from, rel)` order so the result is reproducible across devices.
///
/// Pure and total (an empty input yields no slots; a single edge is its own
/// winner with nothing closed). The caller applies the closes on the serial
/// thread. Unwired until the executor-live-gated merge consumes it.
pub fn resolve_membership(edges: Vec<MembershipEdge>) -> Vec<SlotResolution> {
    use std::collections::BTreeMap;
    let mut slots: BTreeMap<(String, String), Vec<MembershipEdge>> = BTreeMap::new();
    for e in edges {
        slots
            .entry((e.from.clone(), e.rel.clone()))
            .or_default()
            .push(e);
    }
    let mut out = Vec::with_capacity(slots.len());
    for (_slot, mut group) in slots {
        let win_idx = group
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.winner_key().cmp(&b.winner_key()))
            .map(|(i, _)| i)
            .expect("a grouped slot is never empty");
        let winner = group.remove(win_idx);
        out.push(SlotResolution {
            winner,
            closed: group,
        });
    }
    out
}

/// Whether a string is a well-formed device id (any canonical UUID). A
/// malformed or empty file is treated as absent and regenerated, so a truncated
/// or corrupt write never pins a junk id.
fn is_valid_device_id(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// Load the stable per-device id from `path`, generating and persisting a fresh
/// v7 UUID when the file is absent or malformed (graph-drift.md §2, "a stable
/// device id").
///
/// The id is the intended breaker for a genuine same-HLC cross-device clash:
/// resolve_membership tie-breaks on a content field until an edge carries this
/// id, at which point it slots in ahead of that content tiebreak. It is a
/// per-replica identity (it lives beside the store, one per KG), stable across
/// restarts and distinct per device. It is an identifier, not a secret, so it is
/// a plain file; the write is atomic (temp + rename) so a crash mid-write leaves
/// the old id or none, never a torn one. Single-writer by construction (one
/// daemon per replica), so no inter-process race. Unwired until the merge
/// consumes it.
pub fn device_id_at(path: &std::path::Path) -> std::io::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim();
        if is_valid_device_id(existing) {
            return Ok(existing.to_string());
        }
    }
    let id = uuid::Uuid::now_v7().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &id)?;
    std::fs::rename(&tmp, path)?;
    Ok(id)
}

/// The device-wide merge clock: one per KG replica, shared by every writer so
/// their HLCs are comparable. It fuses the monotonic HLC with the stable device
/// id - the two fields a merged edge carries (cross-device ordering + the
/// same-instant tiebreak).
///
/// Held behind a mutex so a `stamp` is atomic across the promotion and the agent
/// write paths (a single device must advance one clock, not two). Shared as
/// `Arc<DeviceClock>` created ONCE at daemon startup and passed by reference -
/// deliberately NOT a module `static`, which would compile to a separate
/// instance in each of the lib and bin crates (this module's `project`/`daemon`
/// writers live in the bin tree while `drift` is lib-only) and silently break
/// monotonicity with two clocks. Unwired until the write paths stamp with it.
pub struct DeviceClock {
    hlc: std::sync::Mutex<Hlc>,
    device_id: String,
}

impl DeviceClock {
    /// Build a fresh device clock over an already-loaded stable device id
    /// (see [`device_id_at`]). The HLC starts at [`Hlc::ZERO`]; the first
    /// `stamp` adopts the supplied physical time.
    pub fn new(device_id: String) -> Self {
        Self {
            hlc: std::sync::Mutex::new(Hlc::ZERO),
            device_id,
        }
    }

    /// The stable device id (the merge same-HLC tiebreak).
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Advance the clock for a local write at physical time `now` (micros) and
    /// return the stamp. Monotonic: every stamp strictly follows the previous,
    /// even under a backwards wall clock, so this device never issues a
    /// non-increasing HLC. Atomic under the mutex, so concurrent writers on the
    /// two paths still get a strict total order.
    pub fn stamp(&self, now: u64) -> Hlc {
        let mut c = self.hlc.lock().expect("device clock mutex poisoned");
        *c = c.tick(now);
        *c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_physical_then_logical() {
        // Physical dominates the logical counter.
        assert!(Hlc::new(10, 0) < Hlc::new(11, 0));
        assert!(Hlc::new(10, 5) < Hlc::new(11, 0));
        // Same physical: the logical counter decides.
        assert!(Hlc::new(10, 0) < Hlc::new(10, 1));
        assert_eq!(Hlc::new(10, 3), Hlc::new(10, 3));
    }

    #[test]
    fn tick_resets_logical_when_wall_time_advances() {
        let a = Hlc::new(100, 7);
        let b = a.tick(200);
        assert_eq!(b, Hlc::new(200, 0));
        assert!(b > a);
    }

    #[test]
    fn tick_increments_logical_at_the_same_micro() {
        let a = Hlc::new(100, 0);
        let b = a.tick(100);
        assert_eq!(b, Hlc::new(100, 1));
        assert!(b > a);
    }

    #[test]
    fn tick_is_monotonic_under_a_backwards_clock() {
        // The clock jumps back (100 -> 50): the stamp must still advance.
        let a = Hlc::new(100, 0);
        let b = a.tick(50);
        assert_eq!(b, Hlc::new(100, 1));
        assert!(b > a);
    }

    #[test]
    fn merge_adopts_a_future_remote_and_stays_ahead_of_both() {
        let local = Hlc::new(100, 4);
        let remote = Hlc::new(300, 2);
        let merged = local.merge(remote, 120);
        // Remote physical wins; its logical is the base + 1.
        assert_eq!(merged, Hlc::new(300, 3));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_at_a_shared_physical_takes_the_higher_logical_plus_one() {
        let local = Hlc::new(200, 5);
        let remote = Hlc::new(200, 9);
        let merged = local.merge(remote, 200);
        assert_eq!(merged, Hlc::new(200, 10));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_with_a_stale_remote_advances_local() {
        let local = Hlc::new(500, 2);
        let remote = Hlc::new(100, 0);
        // now == local physical: local owns it, logical + 1.
        let merged = local.merge(remote, 500);
        assert_eq!(merged, Hlc::new(500, 3));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_when_local_wall_clock_leads_both_resets_logical() {
        let local = Hlc::new(100, 6);
        let remote = Hlc::new(100, 6);
        // A fresh wall time past both: pure physical advance, counter resets.
        let merged = local.merge(remote, 400);
        assert_eq!(merged, Hlc::new(400, 0));
        assert!(merged > local);
    }

    fn edge(from: &str, to: &str, origin: Provenance, hlc: Hlc) -> MembershipEdge {
        MembershipEdge {
            from: from.to_string(),
            rel: "FILE_PART_OF".to_string(),
            to: to.to_string(),
            origin,
            hlc,
        }
    }

    #[test]
    fn empty_input_resolves_to_no_slots() {
        assert!(resolve_membership(vec![]).is_empty());
    }

    #[test]
    fn a_single_edge_is_its_own_winner_with_nothing_closed() {
        let r = resolve_membership(vec![edge("f", "p1", Provenance::Agent, Hlc::new(10, 0))]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].winner.to, "p1");
        assert!(r[0].closed.is_empty());
    }

    #[test]
    fn a_higher_trust_origin_wins_the_slot() {
        // f is linked to p1 by the user and to p2 by the agent: the user wins.
        let r = resolve_membership(vec![
            edge("f", "p2", Provenance::Agent, Hlc::new(50, 0)),
            edge("f", "p1", Provenance::User, Hlc::new(10, 0)),
        ]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].winner.to, "p1", "user membership survives");
        assert_eq!(r[0].closed.len(), 1);
        assert_eq!(r[0].closed[0].to, "p2");
    }

    #[test]
    fn trust_beats_a_later_clock() {
        // The agent edge is newer (HLC 900 > 10) but the user's older edge wins:
        // trust is the primary axis, the clock only breaks a trust tie.
        let r = resolve_membership(vec![
            edge("f", "p2", Provenance::Agent, Hlc::new(900, 0)),
            edge("f", "p1", Provenance::User, Hlc::new(10, 0)),
        ]);
        assert_eq!(r[0].winner.to, "p1");
    }

    #[test]
    fn the_later_clock_breaks_a_trust_tie() {
        // Two agent assertions to different projects: the later HLC wins.
        let r = resolve_membership(vec![
            edge("f", "p1", Provenance::Agent, Hlc::new(10, 0)),
            edge("f", "p2", Provenance::Agent, Hlc::new(10, 5)),
        ]);
        assert_eq!(r[0].winner.to, "p2");
        assert_eq!(r[0].winner.hlc, Hlc::new(10, 5));
    }

    #[test]
    fn an_unranked_graph_origin_never_out_ranks_a_ranked_one() {
        // Graph is unranked (None). Even with a far later clock it loses to a
        // ranked origin, because None sorts below every Some on the trust axis.
        let r = resolve_membership(vec![
            edge("f", "p1", Provenance::External, Hlc::new(10, 0)),
            edge("f", "p2", Provenance::Graph, Hlc::new(9999, 0)),
        ]);
        assert_eq!(r[0].winner.to, "p1", "even External (rank 0) beats unranked Graph");
        assert_eq!(r[0].closed[0].origin, Provenance::Graph);
    }

    #[test]
    fn distinct_slots_resolve_independently_in_sorted_order() {
        let r = resolve_membership(vec![
            edge("b", "p9", Provenance::Agent, Hlc::new(1, 0)),
            edge("a", "p1", Provenance::Agent, Hlc::new(1, 0)),
        ]);
        assert_eq!(r.len(), 2);
        // Sorted by (from, rel): "a" before "b".
        assert_eq!(r[0].winner.from, "a");
        assert_eq!(r[1].winner.from, "b");
        assert!(r[0].closed.is_empty() && r[1].closed.is_empty());
    }

    #[test]
    fn resolution_is_deterministic_regardless_of_input_order() {
        let a = edge("f", "p1", Provenance::User, Hlc::new(10, 0));
        let b = edge("f", "p2", Provenance::Agent, Hlc::new(50, 0));
        let one = resolve_membership(vec![a.clone(), b.clone()]);
        let two = resolve_membership(vec![b, a]);
        assert_eq!(one[0].winner.to, two[0].winner.to);
        assert_eq!(one[0].winner.to, "p1");
    }

    #[test]
    fn device_id_generates_persists_and_is_stable_across_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("sub").join("device-id");
        let first = device_id_at(&path).unwrap();
        assert!(is_valid_device_id(&first));
        assert!(path.exists(), "the id is persisted");
        // A second call returns the same id (stable across restarts).
        let second = device_id_at(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_corrupt_device_id_file_is_regenerated() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("device-id");
        std::fs::write(&path, "not-a-uuid\n").unwrap();
        let id = device_id_at(&path).unwrap();
        assert!(is_valid_device_id(&id), "a junk file regenerates a valid id");
        // And it now persists the valid id, so it is stable henceforth.
        assert_eq!(id, device_id_at(&path).unwrap());
    }

    #[test]
    fn device_clock_stamps_strictly_increase() {
        let clock = DeviceClock::new("dev-1".to_string());
        assert_eq!(clock.device_id(), "dev-1");
        // Same physical micro twice: the logical counter advances.
        let a = clock.stamp(100);
        let b = clock.stamp(100);
        assert!(b > a, "a same-micro second stamp still strictly follows");
        // A later physical time advances physical and resets logical.
        let c = clock.stamp(200);
        assert!(c > b);
        assert_eq!(c, Hlc::new(200, 0));
    }

    #[test]
    fn device_clock_is_monotonic_under_a_backwards_clock() {
        let clock = DeviceClock::new("dev-1".to_string());
        let a = clock.stamp(500);
        // The wall clock jumps back: the stamp must still advance.
        let b = clock.stamp(100);
        assert!(b > a);
        assert_eq!(b, Hlc::new(500, 1));
    }
}
