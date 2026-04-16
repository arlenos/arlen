/// Re-export svelte/store primitives.
///
/// Tailwind v4's Vite plugin has a known bug where `{ writable }` named
/// imports inside `.svelte` `<script>` blocks trigger CSS parse errors
/// ("Invalid declaration"). Importing from this `.ts` file avoids the
/// issue because Tailwind only applies its CSS transformer to `.svelte`
/// and `.css` files.
export { writable, derived, get } from "svelte/store";
export type { Writable, Readable } from "svelte/store";
