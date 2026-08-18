/// Hold the row ORDER still while someone reads it, without holding the numbers
/// still (system-monitor-plan.md (a): "freeze-the-refresh (hold a modifier so
/// rows stop reordering while you read)").
///
/// The distinction is the whole feature. Stopping the poll would freeze the
/// figures too, so a held modifier would hand you a screenshot of two seconds
/// ago and call it the system state - the exact class of surface this tree keeps
/// finding and removing. What actually makes a task manager unusable is the row
/// you are aiming at moving as you click, so it is the ORDER that is pinned and
/// the values keep arriving underneath it.

/// Anything with a stable identity. `Process` satisfies it; the function is
/// written against the id alone so it can be tested without a process model.
export interface Identified {
  id: number;
}

/// Reorder `fresh` to follow `pinned`, an id order captured when the freeze
/// began.
///
/// Rows that were in the pinned order keep their positions, in that order. Rows
/// that have appeared since are appended, in whatever order `fresh` already had
/// (which is the live sort, so a new hog still lands sensibly among its
/// newcomers). Rows that have gone are simply absent - there is nothing to hold
/// a place for, and leaving a gap would be inventing a process that exited.
///
/// Not frozen when `pinned` is empty, so a caller that has captured nothing gets
/// the live order rather than an empty list.
export function pinnedOrder<T extends Identified>(fresh: T[], pinned: number[]): T[] {
  if (pinned.length === 0) return fresh;
  const byId = new Map(fresh.map((r) => [r.id, r]));
  const held: T[] = [];
  for (const id of pinned) {
    const row = byId.get(id);
    if (row) {
      held.push(row);
      byId.delete(id);
    }
  }
  // `byId` now holds only the newcomers, and a Map preserves insertion order, so
  // they come out in the order `fresh` had them.
  return [...held, ...byId.values()];
}
