<script lang="ts">
  /// The consent surface's page. It carries no markup of its own.
  ///
  /// The card itself is mounted by the root layout, gated on this window's label,
  /// because that gate is where the one-request-one-surface rule already lives: an
  /// earlier boot had two windows each polling the broker and rendering their own
  /// copy, and the same request got answered twice. Keeping the mount in one place
  /// keeps that rule in one place.
  ///
  /// The window exists at all because keyboard focus on a layer surface is decided
  /// when it maps. The card used to live in the top bar, which maps once at startup
  /// with no keyboard interactivity, and flipping that property later changed
  /// nothing anyone read again - so Escape-to-deny never fired. This window maps
  /// exclusive, the way the waypointer does.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  /// Reports what this window actually paints, because guessing cost three image
  /// rebuilds. With the card up the frame is 11.4% non-black and the desktop is
  /// gone; the waypointer, another secondary window built the same way and
  /// rendering this same layout, sits at 94% and shows the desktop through. So the
  /// difference is here, and three plausible causes have already been refuted by
  /// measurement (an RGBA visual, `AmbientOverlay`, the rest of the desktop
  /// chrome). This prints the computed grounds instead of arguing about them.
  onMount(() => {
    const report = () => {
      const style = (el: Element | null) =>
        el ? getComputedStyle(el).backgroundColor : "absent";
      const msg =
        `consent surface grounds: html=${style(document.documentElement)}` +
        ` body=${style(document.body)}` +
        ` overlay=${style(document.querySelector('[data-slot="dialog-overlay"]'))}` +
        ` card=${style(document.querySelector(".arlen-consent-card"))}` +
        ` bg-overlay-token=${getComputedStyle(document.documentElement)
          .getPropertyValue("--color-bg-overlay")
          .trim() || "unset"}`;
      void invoke("frontend_log", { level: "info", msg }).catch(() => {});
    };
    // Once at mount for the empty case, and again after a card has had time to
    // appear, since the overlay does not exist until a request is pending.
    report();
    const t = setTimeout(report, 20_000);
    return () => clearTimeout(t);
  });
</script>
