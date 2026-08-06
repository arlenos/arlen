import { dev } from "$app/environment";
import { error } from "@sveltejs/kit";

/// A dev surface, and only a dev surface. Without this the route ships in the
/// packaged shell, where a look-mock is a page users can reach by typing a path.
export function load() {
  if (!dev) error(404, "not found");
  return {};
}
