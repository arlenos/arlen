<script lang="ts">
  /// Toasts that stay up, so the way they stack can be looked at.
  ///
  /// The shell raises real toasts on boot and they expire in seconds, which is
  /// long enough to be missed and short enough that a probe reaching for them
  /// usually finds nothing. That is how a real defect sat here: with the
  /// animation set to `fade` or `none`, an override killed `transform` on the
  /// toast, and `transform` is what carries svelte-sonner's collapsed stack - so
  /// every toast landed at one spot at full size, text drawn over text. I looked
  /// at that picture once and read it as the default, because I had no way to
  /// render either setting on purpose.
  ///
  /// `?anim=slide|fade|none` picks the flavour, `?pos=top-right|bottom-left|...`
  /// the corner, `?n=3` how many, `?duration=` how long they hold (default:
  /// until the page is closed). `?locale=de` renders the copy in German.
  ///
  /// Position is here because the layout hands the Toaster a fixed
  /// `offset={{ top, right }}` while the corner itself is a setting - so any
  /// corner other than the default gets an offset written for a different one,
  /// and nothing rendered the others to show what that looks like.
  import { onMount } from "svelte";
  import { toast } from "svelte-sonner";
  import { locale } from "@arlen/ui-kit/i18n";
  import { toastConfig } from "$lib/stores/toastConfig";

  const LINES = [
    "The focus mode you had set could not be restored.",
    "Could not read your Quick Settings layout.",
    "That panel could not be opened.",
    "The theme file could not be read, so this is the built-in one.",
  ];

  onMount(() => {
    const p = new URLSearchParams(window.location.search);
    if (p.get("locale")) locale.set(p.get("locale") as string);

    const pos = p.get("pos");
    if (pos) toastConfig.update((c) => ({ ...c, position: pos as typeof c.position }));

    const anim = p.get("anim");
    if (anim === "slide" || anim === "fade" || anim === "none") {
      toastConfig.update((c) => ({ ...c, animation: anim }));
    }

    // Infinity by default: a toast that dismisses itself cannot be photographed
    // reliably, which is the whole reason this route exists.
    const raw = p.get("duration");
    const duration = raw ? Number(raw) : Infinity;
    const n = Math.min(Number(p.get("n") ?? 3), LINES.length);
    // One per frame, not all in one tick. Raising them together left exactly one
    // standing: changing `toastConfig` re-renders the Toaster, and toasts queued
    // in the same tick as that change do not survive it. Measured - `n=4` and
    // `n=3` both produced a single toast until this was spaced out.
    for (let i = 0; i < n; i++) {
      setTimeout(() => toast.error(LINES[i], { duration }), i * 60);
    }
  });
</script>

<main>
  <h1>Toast presentation</h1>
  <p>
    Raised on load and held open. Add <code>?anim=fade</code>,
    <code>?anim=none</code>, <code>?n=4</code> or <code>?locale=de</code>.
  </p>
</main>

<style>
  main {
    padding: 2rem;
    color: var(--foreground);
    font-size: 0.9rem;
  }
  h1 {
    font-size: 1.1rem;
    margin-bottom: 0.5rem;
  }
  code {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    padding: 0 0.25rem;
    border-radius: 3px;
  }
</style>
