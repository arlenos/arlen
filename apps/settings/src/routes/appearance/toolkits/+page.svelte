<script lang="ts">
  /// Toolkits: where the one theme reaches. Six rows, one per output, each
  /// saying its ceiling (the badge), what it reaches (the description) and
  /// whether it is in place on this machine (the readout at the end). The
  /// settings-panel archetype like every other Appearance page: kit Section
  /// and Rows, no card of its own.
  ///
  /// The sentences come from having put every toolkit on screen beside an Arlen
  /// window (`dev/screenshot/shoot-toolkits.sh`, 7 and 8 September), which is
  /// the only way to know what a colour floor reaches and what it cannot.
  ///
  /// No switch and no override: the per-toolkit on/off and the accent override
  /// have no command behind them yet (the resolver reads `[override.<toolkit>]`,
  /// nothing writes it), and a control that changes nothing is a lie with a
  /// thumb. They return with their commands.
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import {
    TOOLKITS,
    coverageBadge,
    reach,
    loadReach,
    prereqs,
    loadPrereqs,
    loadEnabled,
    loadOverrides,
    showPrereq,
    type ToolkitReach,
  } from "$lib/stores/themeToolkits";

  // Read-only, so asking on mount costs nothing and a row is never a claim
  // about a check that did not run. The switch and the accent are read here
  // too, although the controls that write them are not back yet: the page
  // holding the machine's answer is what lets a control open on it rather than
  // on a default, which is the shape that put a knob in front of a value
  // nobody had read.
  onMount(() => {
    void loadReach();
    void loadPrereqs();
    void loadEnabled();
    void loadOverrides();
  });

  /// The readout for one row: a posture for the dot and the sentence beside it.
  /// Nothing until the host has answered, because "in place" over an unasked
  /// question is the claim this page exists to stop.
  function readout(id: string, native: boolean | undefined, r: ToolkitReach | undefined) {
    if (native) return { posture: "ours", text: $t("s.toolkit.reach.always") };
    if (!r) return null;
    if (r.state === "ours") return { posture: "ours", text: $t("s.toolkit.reach.ours") };
    if (r.state === "blocked") return { posture: "away", text: $t("s.toolkit.blocked", { file: r.blockedBy ?? "" }) };
    if (r.state === "unselected") return { posture: "away", text: $t("s.toolkit.unselected", { name: r.blockedBy ?? "" }) };
    return { posture: "unknown", text: $t("s.toolkit.absent") };
  }
</script>

<Page title={$t("s.tk.title")} description={$t("s.tk.desc")}>
  <SectionGrid>
    <Section label={$t("s.tk.reaches")} class="span-full">
      {#each TOOLKITS as tk (tk.id)}
        {@const badge = coverageBadge(tk.coverage)}
        {@const state = readout(tk.id, tk.native, $reach[tk.id])}
        {@const unmet = tk.prereqKey && showPrereq($prereqs, tk.id)}
        {#snippet ceiling()}
          <Badge variant={badge.tone}>{$t(badge.labelKey)}</Badge>
        {/snippet}
        {#snippet where()}
          {#if state}
            <span class="readout" class:away={state.posture === "away"}>
              <span class="found" data-posture={state.posture} aria-hidden="true"></span>
              {state.text}
            </span>
          {/if}
        {/snippet}
        <!-- Two rows rather than a conditional `below` prop: the kit's Snippet
             type and this app's are two copies of one name, and a snippet
             passed as a value does not typecheck across them. -->
        {#if unmet}
          <Row label={$t(tk.nameKey)} description={$t(tk.noteKey)} id={`toolkit-${tk.id}`}>
            {#snippet leading()}{@render ceiling()}{/snippet}
            {#snippet control()}{@render where()}{/snippet}
            {#snippet below()}
              <p class="prereq">{$t(tk.prereqKey ?? "")}</p>
            {/snippet}
          </Row>
        {:else}
          <Row label={$t(tk.nameKey)} description={$t(tk.noteKey)} id={`toolkit-${tk.id}`}>
            {#snippet leading()}{@render ceiling()}{/snippet}
            {#snippet control()}{@render where()}{/snippet}
          </Row>
        {/if}
      {/each}
    </Section>
  </SectionGrid>
</Page>

<style>
  /* The readout: a dot of the house family (6px on the chip radius, as the
     privacy readout draws its findings) and a VALUE, not a sentence - "In
     place", "Adwaita selected" - because a sentence here pushed the label
     column aside (Tim, 9 September). Success is quiet, a file or a theme in
     the way is the warning tone, an unwritten output the ring. */
  .readout {
    display: inline-block;
    /* Bounded by the window, not by its own text: the kit's control slot does
       not shrink, so a sentence-wide readout took the label's room at 720px and
       set "GTK3" one word per line. This wraps instead. */
    max-width: clamp(7rem, 22vw, 20rem);
    font-size: var(--text-xs);
    line-height: 1.35;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    text-align: end;
  }
  .readout.away {
    color: var(--color-warning);
  }
  /* The dot is part of the text run, not a flex column beside it: when the
     value wraps to two lines, a column put the dot between the lines and away
     from a shorter first line. Inline, it stays glued to the first word. */
  .found {
    display: inline-block;
    vertical-align: middle;
    margin-inline-end: 0.45rem;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-chip, 4px);
  }
  .found[data-posture="ours"] {
    background: var(--color-success);
  }
  .found[data-posture="away"] {
    background: var(--color-warning);
  }
  .found[data-posture="unknown"] {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  /* A prerequisite this machine does not meet: one caution line under the row,
     only while it is true. */
  .prereq {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--color-warning);
  }
</style>
