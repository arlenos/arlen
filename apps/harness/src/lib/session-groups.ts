/// Time-bucketing for the conversation rail (the Hollama lift): pinned
/// conversations float into their own group at the top, the rest fall into
/// Today / Yesterday / Previous 7 days / Earlier by their creation time. Pure
/// and unit-tested without the store; the rail renders the returned groups in
/// order, each as its own labelled section.
import type { Session } from "$lib/stores/conversation";

/// One labelled section of the rail: a heading and the conversations under it.
/// The label doubles as the `{#each}` key, so the set of labels is fixed and
/// unique.
export interface SessionGroup {
  /// The i18n key for the section heading (also the list key); the rail
  /// resolves it through the catalog so the heading follows the locale.
  label: string;
  /// The conversations in this section, in the incoming (newest-first) order.
  sessions: Session[];
}

const DAY_MS = 86_400_000;

/// Midnight at the start of the day containing `ts`, in local time.
function startOfDay(ts: number): number {
  const d = new Date(ts);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/// Partition sessions into rail sections. Pinned conversations form a single
/// "Pinned" section at the top (kept whole, never split across time buckets);
/// the remainder bucket by `createdAt` relative to `now`. Only non-empty
/// sections are returned, always in display order. Within every section the
/// incoming order is preserved, so the caller controls newest-first.
export function groupSessions(sessions: Session[], now: number): SessionGroup[] {
  const pinned = sessions.filter((s) => s.pinned);
  const rest = sessions.filter((s) => !s.pinned);

  const today = startOfDay(now);
  const yesterday = today - DAY_MS;
  const lastWeek = today - 7 * DAY_MS;

  const groups: SessionGroup[] = [];
  const push = (label: string, items: Session[]) => {
    if (items.length > 0) groups.push({ label, sessions: items });
  };

  push("h.group.pinned", pinned);
  push("h.group.today", rest.filter((s) => s.createdAt >= today));
  push(
    "h.group.yesterday",
    rest.filter((s) => s.createdAt >= yesterday && s.createdAt < today),
  );
  push(
    "h.group.previous7",
    rest.filter((s) => s.createdAt >= lastWeek && s.createdAt < yesterday),
  );
  push("h.group.earlier", rest.filter((s) => s.createdAt < lastWeek));

  return groups;
}
