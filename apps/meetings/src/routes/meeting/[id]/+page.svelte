<script lang="ts">
  /// The meeting note: one merged document (your lines full-strength, the AI's
  /// enhancements inline in the AI tint), the action items with confirmable
  /// owners, and the transcript rail as the verification source - speaker labels
  /// click-to-rename there. Back always returns to the list.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { ArrowLeft, SquareArrowOutUpRight } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import MeetingShell from "$lib/components/MeetingShell.svelte";
  import NotesMerged from "$lib/components/NotesMerged.svelte";
  import ActionItems from "$lib/components/ActionItems.svelte";
  import TranscriptPanel from "$lib/components/TranscriptPanel.svelte";
  import { t, dir } from "$lib/i18n/messages";
  import { meeting, currentId, openMeeting, saveNotes, updateItem, openInEditor } from "$lib/stores/meeting";

  const id = $derived($page.params.id);
  let activeStart = $state<number | null>(null);

  onMount(async () => {
    // "live" is the just-captured note already in the store; anything else loads
    // by id. A cold "live" hit (reload) falls back to the list.
    const target = id;
    if (!target || target === "live") {
      if (!$meeting) await goto("/");
      return;
    }
    if ($currentId !== target || !$meeting) await openMeeting(target);
  });
</script>

<div class="page" dir={$dir}>
  {#if $meeting}
    {@const m = $meeting}
    <MeetingShell>
      {#snippet head()}
        <div class="note-head">
          <button type="button" class="back" id="back" onclick={() => goto("/")}>
            <ArrowLeft size={15} strokeWidth={2} />
            {$t("mt.back")}
          </button>
          <div class="head-text">
            <h1 class="title">{m.note.title}</h1>
            <p class="meta">{m.note.participants.join(", ")}</p>
          </div>
          <Button variant="ghost" size="sm" class="text-muted-foreground" id="open-editor" onclick={openInEditor}>
            <SquareArrowOutUpRight size={14} strokeWidth={1.75} />
            {$t("mt.open")}
          </Button>
        </div>
        {#if m.mocked}
          <p class="sample">{$t("mt.sample")}</p>
        {/if}
      {/snippet}
      {#snippet content()}
        <div class="note-body">
          <NotesMerged
            notes={m.humanNotes}
            claims={m.note.summary_claims ?? []}
            transcript={m.note.transcript}
            onjump={(s) => (activeStart = s)}
            onsave={(text) => void saveNotes(text)}
          />
          <ActionItems
            items={m.note.action_items}
            transcript={m.note.transcript}
            onjump={(s) => (activeStart = s)}
            onupdate={(i, patch) => void updateItem(i, patch)}
          />
        </div>
      {/snippet}
      {#snippet rail()}
        <TranscriptPanel
          transcript={m.note.transcript}
          {activeStart}
          onseek={(s) => (activeStart = s)}
          renamable
        />
      {/snippet}
    </MeetingShell>
  {/if}
</div>

<style>
  .page {
    height: 100%;
    min-height: 0;
  }
  .note-head {
    display: flex;
    align-items: center;
    gap: 1rem;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    flex-shrink: 0;
    padding: 0.3rem 0.6rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .back:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
    color: var(--color-fg-primary);
  }
  .head-text {
    flex: 1;
    min-width: 0;
  }
  .title {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    margin: 0.1rem 0 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .sample {
    margin: 0.5rem 0 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .note-body {
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
  }
</style>
