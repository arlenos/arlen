<script lang="ts">
  /// The meeting note: one merged document (your lines full-strength, the AI's
  /// enhancements inline in the AI tint), the action items with confirmable
  /// owners, and the transcript rail as the verification source - speaker labels
  /// click-to-rename there. Back always returns to the list.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { SquareArrowOutUpRight } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import MeetingShell from "$lib/components/MeetingShell.svelte";
  import NotesMerged from "$lib/components/NotesMerged.svelte";
  import ActionItems from "$lib/components/ActionItems.svelte";
  import TranscriptPanel from "$lib/components/TranscriptPanel.svelte";
  import { t, dir } from "$lib/i18n/messages";
  import {
    meeting,
    currentId,
    openMeeting,
    saveNotes,
    updateItem,
    openInEditor,
    editFailed,
  } from "$lib/stores/meeting";

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
        <!-- The inset header names the note; this head carries only the meta
             and the actions, so the title is said once. -->
        <div class="note-head">
          <p class="meta">{m.note.participants.join(", ")}</p>
          <Button variant="ghost" size="sm" class="text-muted-foreground" id="open-editor" onclick={openInEditor}>
            <SquareArrowOutUpRight size={14} strokeWidth={1.75} />
            {$t("mt.open")}
          </Button>
        </div>
        {#if m.mocked}
          <p class="sample">{$t("mt.sample")}</p>
          <!-- The notes and items are the user's own words; if an edit did not
               persist the text on screen goes back rather than reading as saved. -->
          {#if $editFailed}
            <p class="sample" role="alert">{$t("mt.editFailed")}</p>
          {/if}
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
  .meta {
    margin: 0;
    flex: 1;
    min-width: 0;
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
