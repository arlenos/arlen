/// The top-bar badge this window wears, from the mailbox it is showing.
///
/// The shell keeps one badge per app and draws the focused app's beside its
/// name (`badges-api.md` FA2, rendered on `GlobalMenuBar`). Until now nothing in
/// the tree published one, so the slot was drawn and always empty; mail is the
/// first producer because it already counts its own unread mail for the folder
/// rail and the count is the thing a badge is for.
///
/// A COUNT, not a status. `badges-api.md` FA3 has the knowledge daemon promote
/// error and warning badges into the graph and deliberately not count-only ones,
/// so unread mail stays a number on a screen and does not become a row in
/// anybody's history. That is the right shape for it: how much mail is waiting
/// is not an event that happened.
///
/// Pure, so the rule is a test rather than something to boot the shell for.

import { badges, type BadgeKind } from "@arlen/tauri-plugin-shell";

/// The badge for an unread count, or null when there is nothing to say.
///
/// Zero clears rather than showing a nought: a badge is a thing waiting for you,
/// and "0 waiting" is the absence of one. A negative or non-finite count is
/// treated the same way - the count comes from a derived store over the
/// envelopes, so it cannot be either today, and a badge is not the place to find
/// out that it became possible.
export function badgeFor(unread: number): BadgeKind | null {
  if (!Number.isFinite(unread) || unread <= 0) return null;
  return { kind: "count", count: Math.floor(unread) };
}

/// Publish the badge for `unread`, or take the previous one down.
///
/// Best-effort by the same rule the menu registration follows: under vite there
/// is no shell relay and both calls reject, and a mail window that cannot reach
/// the top bar is still a mail window. Nothing downstream depends on the result.
export async function publishBadge(unread: number): Promise<void> {
  const badge = badgeFor(unread);
  try {
    if (badge) await badges.set(badge);
    else await badges.clear();
  } catch {
    // No shell (vite, or the permission seam not landed): no badge.
  }
}
