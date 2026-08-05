// The one dynamic route in the app: a component id cannot be prerendered, and the
// static adapter's index.html fallback serves it as SPA. Same as settings' own
// `apps/[id]`; without it `npm run build` fails the whole app on an uncrawled
// prerenderable route, which is how this app was until the frontend job started
// building as well as type-checking.
export const prerender = false;
