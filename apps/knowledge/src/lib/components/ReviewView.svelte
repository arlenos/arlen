<script lang="ts">
  /// The review list (bitemporal-knowledge-graph.md §4.12): the graph asks
  /// whether two entries are the same person, place or thing, and the person
  /// answers. One group per entry, its possible matches beneath it, so three
  /// spellings of one person read as one question.
  ///
  /// What a row shows is what a person recognises: the name, the one detail
  /// that tells it apart, where it is known from, and a sentence for why the
  /// graph asks. No score: a number invites trust the evidence has not earned,
  /// and the order already puts the strongest evidence first.
  ///
  /// An answer stays where it was given until the view is left, with the way
  /// back beside it, so a slip is one press from undone. After that it lives
  /// under Decided, and nothing there expires.
  import { MapPin, Shapes, User } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import {
    suggestions,
    openCount,
    decidedCount,
    loadReview,
    decide,
    type Candidate,
    type EntryKind,
  } from "$lib/stores/review";
  import { t } from "$lib/i18n/messages";

  const ICON = { person: User, place: MapPin, thing: Shapes } satisfies Record<EntryKind, unknown>;

  let show: string = $state("open");

  /// Answered or passed over while this view has been open. An answered row
  /// stays in the open list with its way back; a passed-over one steps out of
  /// the way until the list is opened again, and nothing is stored for it.
  let answeredHere: Set<string> = $state(new Set());
  let passedOver: Set<string> = $state(new Set());

  onMount(() => {
    // The render loop asks for the empty and the long list by name.
    const shape = import.meta.env.DEV ? page.url.searchParams.get("review") : null;
    loadReview(shape === "none" || shape === "many" ? shape : "few");
  });

  function answer(c: Candidate, decision: "same" | "different"): void {
    answeredHere = new Set(answeredHere).add(c.id);
    decide(c.id, decision);
  }

  function takeBack(c: Candidate): void {
    const next = new Set(answeredHere);
    next.delete(c.id);
    answeredHere = next;
    decide(c.id, null);
  }

  function passOver(c: Candidate): void {
    passedOver = new Set(passedOver).add(c.id);
  }

  function listed(c: Candidate): boolean {
    if (show === "decided") return c.decision !== null;
    if (passedOver.has(c.id)) return false;
    return c.decision === null || answeredHere.has(c.id);
  }

  const groups = $derived(
    $suggestions
      .map((g) => ({ entry: g.entry, candidates: g.candidates.filter(listed) }))
      .filter((g) => g.candidates.length > 0),
  );
</script>

<div class="rv">
  <div class="rv-head">
    <div class="note"><Notice tone="neutral" text={$t("k.sample")} /></div>
    <SegmentedControl
      id="review-show"
      ariaLabel={$t("k.rv.show")}
      bind:value={show}
      options={[
        { value: "open", label: $t("k.rv.open", { n: $openCount }) },
        { value: "decided", label: $t("k.rv.decided", { n: $decidedCount }) },
      ]}
    />
    <p class="rv-what">{$t("k.rv.what")}</p>
  </div>

  <div class="rv-scroll">
    {#if groups.length === 0}
      <p class="rv-empty">{$t(show === "decided" ? "k.rv.emptyDecided" : "k.empty.review")}</p>
    {:else}
      {#each groups as g (g.entry.id)}
        {@const EntryIcon = ICON[g.entry.kind]}
        <section class="rv-group" aria-label={g.entry.name}>
          <div class="rv-entry">
            <EntryIcon size={15} strokeWidth={1.75} aria-hidden="true" />
            <span class="rv-name">{g.entry.name}</span>
            <span class="rv-detail">{g.entry.detail}</span>
            <span class="rv-source">{g.entry.source}</span>
          </div>
          {#each g.candidates as c (c.id)}
            <div class="rv-cand" class:rv-answered={c.decision !== null}>
              <div class="rv-line">
                <span class="rv-name">{c.name}</span>
                <span class="rv-detail">{c.detail}</span>
                <span class="rv-source">{c.source}</span>
              </div>
              <div class="rv-ask">
                {#if c.decision === null}
                  <span class="rv-why">{$t(`k.rv.why.${c.reason}`)}</span>
                  <span class="rv-verbs">
                    <Button variant="outline" size="sm" data-verb="same" onclick={() => answer(c, "same")}>{$t("k.rv.same")}</Button>
                    <Button variant="outline" size="sm" data-verb="different" onclick={() => answer(c, "different")}>{$t("k.rv.different")}</Button>
                    <Button variant="ghost" size="sm" data-verb="pass" onclick={() => passOver(c)}>{$t("k.rv.notSure")}</Button>
                  </span>
                {:else}
                  <span class="rv-why">{$t(c.decision === "same" ? "k.rv.isSame" : "k.rv.isDifferent", { name: g.entry.name })}</span>
                  <span class="rv-verbs">
                    <Button variant="ghost" size="sm" data-verb="back" onclick={() => takeBack(c)}>{$t("k.rv.takeBack")}</Button>
                  </span>
                {/if}
              </div>
            </div>
          {/each}
        </section>
      {/each}
    {/if}
  </div>
</div>

<style>
  .rv {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rv-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.875rem;
    padding: 0.6rem 1.1rem 0.5rem;
  }
  .note {
    flex: 1 1 100%;
  }
  /* What saying yes does, once, where the answers are given. */
  .rv-what {
    flex: 1 1 18rem;
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .rv-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.25rem 1.1rem 1.25rem;
  }
  .rv-empty {
    margin: 0.75rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  .rv-group {
    padding: 0.75rem 0 0.5rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .rv-group:last-child {
    border-bottom: none;
  }
  /* Name, the detail that tells it apart, where it is known from. The same
     three columns on the entry and on every candidate under it, so the eye
     compares down a column. */
  .rv-entry,
  .rv-line {
    display: grid;
    grid-template-columns: 15px minmax(0, 1fr) minmax(0, 1.1fr) 9rem;
    align-items: center;
    column-gap: 0.5rem;
    padding: 0 0.375rem;
  }
  .rv-entry {
    padding-bottom: 0.25rem;
    color: var(--color-fg-primary);
  }
  /* A candidate keeps the icon column empty, so its three cells sit exactly
     under the entry's. */
  .rv-line::before {
    content: "";
  }
  .rv-cand {
    padding: 0.4rem 0;
  }
  .rv-ask {
    margin-inline-start: calc(15px + 0.5rem);
    padding: 0 0.375rem;
  }
  .rv-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rv-detail,
  .rv-source {
    min-width: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rv-source {
    justify-self: end;
  }
  .rv-ask {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.375rem 0.75rem;
    margin-top: 0.3rem;
  }
  .rv-why {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .rv-verbs {
    display: flex;
    gap: 0.375rem;
  }
  /* An answered row steps back; its way back stays at full strength. */
  .rv-answered .rv-line {
    opacity: 0.6;
  }
</style>
