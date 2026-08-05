// A meeting id only exists at runtime, so this route cannot be prerendered: the
// root layout turns prerendering on for the static pages, and SvelteKit then
// fails the whole build because it never crawled a concrete `/meeting/<id>` to
// render ("marked as prerenderable, but not prerendered"). That failure took the
// mkosi image build down with it, which is worse than it sounds - the image is
// the only place the assembled system runs.
//
// The static adapter already has `fallback: "index.html"`, so opting out here
// does not lose the route: it is served by the SPA fallback and resolves its id
// on the client, which is what a Tauri app does with every route anyway.
export const prerender = false;
