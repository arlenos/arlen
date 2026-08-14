/// Whether anything the launcher asked refused to answer this query.
///
/// The Waypointer fans a query out to ten providers - apps, windows, settings,
/// power, quick actions, files, clipboard, dictionary, projects, recents - and
/// each one's failure lands in a `.catch` that leaves its store empty. Empty is
/// also what a provider returns when it genuinely found nothing, so the two are
/// the same picture, and the line under them said the confident thing:
///
///     No results found.
///
/// Which is a claim about the world. With the backend down it is the answer to
/// every query, including one that matches ten installed apps. A person reads it
/// and concludes the app is not there.
///
/// So the refusals are counted. `nothing found` and `nothing answered` are
/// different sentences and this is what tells them apart - the same
/// absent/refused/empty split the file manager's `read-outcome` draws, on the
/// surface people touch most.
///
/// Reset at the START of each fan-out, not on success: a query is one question,
/// and the answer to it is whatever the providers said about THAT query.

import { writable } from "svelte/store";

/// Number of providers that refused the current query.
export const searchRefusals = writable(0);

/// Begin a new query. Everything said about the previous one stops applying.
export function beginSearch(): void {
  searchRefusals.set(0);
}

/// One provider refused. Call from the catch that would otherwise leave a store
/// empty and let it read as "found nothing".
export function noteRefusal(): void {
  searchRefusals.update((n) => n + 1);
}
