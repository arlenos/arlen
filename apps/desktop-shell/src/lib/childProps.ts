// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// Running a trigger's own handler alongside ours.
///
/// bits-ui hands a `child` snippet the trigger's attributes as an untyped
/// record, and among them are event handlers. Svelte applies attributes in
/// source order, so `{...props}` must come FIRST or it replaces whatever we
/// wrote before it - which is how three buttons in this app ended up dead, the
/// media panel among them, reachable by no other click.
///
/// With the spread first, the component's handler is the one we overwrite, so it
/// has to be called by hand. The record is untyped, hence the guard: a value that
/// is not a function is simply not called.
export function alsoRun(handler: unknown, event: Event): void {
  if (typeof handler === "function") {
    (handler as (e: Event) => void)(event);
  }
}
