// The second dynamic route in the app, and dynamic for the same reason as
// `apps/[id]`: the ids are what is INSTALLED on the machine looking at the page,
// which a build machine cannot know. The static adapter's index.html fallback
// serves it as an SPA.
//
// The list reaches it through an interpolated `goto`, so the prerender crawler
// never sees the route either - it follows literal `<a href>` only. That is what
// turned CI red on `91545bf90`: not a broken link, an undiscoverable one over a
// page that must not be enumerated in the first place. An `entries()` export
// would answer the crawler by inventing a build-time list of somebody's
// extensions; `handleUnseenRoutes` would silence the class for every future
// route as well.
export const prerender = false;
