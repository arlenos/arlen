<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->
<!--
  Render a formatted message whose marked terms carry inline markup.

  The sentence stays one catalog entry; `mark(name)` is formatted into it in place
  of the term, and the snippet of the same name renders that term with whatever
  element it needs. See `rich.ts` for why the alternative - a `.pre`/`.post` pair
  per sentence - cannot be translated.

      <Rich text={$t("s.monitor.noDisplays", { proto: mark("proto") })}>
        {#snippet proto()}<code>wlr-output-management</code>{/snippet}
      </Rich>

  The snippets are declared inside the tag rather than handed over in a prop, which
  keeps each term next to the element that styles it. It also keeps the kit off the
  app's `Snippet` type: the kit is consumed by source alias, so a kit component and
  an app component resolve `svelte` from different node_modules, and a snippet-typed
  prop fails to typecheck across that line for a reason that has nothing to do with
  the code.

  A name with no snippet renders the name in place, visibly wrong rather than
  invisibly missing: dropping it silently would leave a hole that reads as a wording
  mistake and never gets reported.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { richParts } from "./rich";

  // Rest props are the mark snippets, keyed by name.
  let { text, ...marks }: { text: string } & Record<string, unknown> = $props();

  /// `{@render}` needs a snippet; a name the caller did not supply must fall back
  /// to text rather than throw, so the lookup is resolved before the markup.
  function snippetFor(name: string): Snippet | null {
    const s = marks[name];
    // The prop is `unknown` on purpose - see the note above - so the cast happens
    // here, once, guarded by the only check that is meaningful across two copies
    // of Svelte: a snippet is a function.
    return typeof s === "function" ? (s as Snippet) : null;
  }
</script>

<!-- prettier-ignore -->
{#each richParts(text) as part}{#if part.kind === "text"}{part.text}{:else}{@const snip = snippetFor(part.name)}{#if snip}{@render snip()}{:else}{part.name}{/if}{/if}{/each}
