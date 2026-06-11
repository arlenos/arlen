<script lang="ts">
  /// The origin marker IS the prompt char (terminal.md §4.7, glyph
  /// treatment per Tim's June 2026 decision): every block carries ❯;
  /// a user command renders it quiet, an agent-drafted command
  /// renders it in accent. The visible word "agent" in the block
  /// header meta says the same thing in plain language, so the char
  /// is not the only carrier. Origin is backend-stamped — this
  /// component only renders what the contract says, it never infers.
  import type { Origin } from "$lib/contract";

  let { origin }: { origin: Origin } = $props();
</script>

<span class="om" class:om-agent={origin === "agent"} aria-hidden="true">❯</span>

<style>
  .om {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .om-agent {
    color: var(--color-accent, var(--primary));
  }
</style>
