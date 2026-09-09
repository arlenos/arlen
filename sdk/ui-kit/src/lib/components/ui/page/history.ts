/// Where the person was before this page, so the door on a sub-route can go
/// back with the parent's state rather than reload it (design-system.md §6.9).
///
/// The kit imports nothing from `$app`, so it does not know the router; it
/// knows only which `Page` mounted last. That is enough: when the previous
/// page IS the parent the door names, stepping back through history restores
/// the parent as it was (SvelteKit restores scroll on popstate and any
/// `snapshot` the parent exports). When it is not - the person arrived from
/// the sidebar, a search result or another sub-route - the door navigates to
/// the parent afresh, which is the honest thing, since there is no state of
/// that page to return to.
let previous: string | null = null;
let current: string | null = null;

/// Record that a page mounted on `pathname`. A remount on the same path (a
/// param change, hot reload) is not a move.
export function notePage(pathname: string): void {
  if (pathname === current) return;
  previous = current;
  current = pathname;
}

/// Whether the page before this one was `href` (path only; query and hash
/// do not make it a different room).
export function cameFrom(href: string): boolean {
  return previous !== null && pathOf(previous) === pathOf(href);
}

function pathOf(href: string): string {
  const cut = href.search(/[?#]/);
  return cut === -1 ? href : href.slice(0, cut);
}
