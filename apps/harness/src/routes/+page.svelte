<script lang="ts">
  /// Chat (ai-app.md §2.1), the heart of the harness: a flat document
  /// thread, one composer container, and one quiet capability line, all on
  /// the shared thread column. The pieces live in `$lib/components/chat`;
  /// this route only loads the capability context and lays out the column.
  /// A contextual right pane (the artifact panel) mounts beside the column
  /// once its data path exists.
  import { onMount } from "svelte";
  import ChatThread from "$lib/components/chat/ChatThread.svelte";
  import Composer from "$lib/components/chat/Composer.svelte";
  import CapabilityBar from "$lib/components/chat/CapabilityBar.svelte";
  import ArtifactPanel from "$lib/components/ArtifactPanel.svelte";
  import { readCapability, type Capability } from "$lib/capability";
  import { messages } from "$lib/stores/conversation";
  import { openArtifact, closePane } from "$lib/stores/artifact";

  let capability = $state<Capability | null>(null);
  let capLoaded = $state(false);

  async function loadCapability() {
    capability = await readCapability();
    capLoaded = true;
  }
  onMount(loadCapability);

  // The three capability states the surface designs for: usable, switched
  // off, and unreachable (the read failed, distinct from off).
  const aiReady = $derived(capability?.enabled === true);
  const aiOff = $derived(capLoaded && capability !== null && !capability.enabled);
  const unreachable = $derived(capLoaded && capability === null);

  const emptyVariant = $derived(aiOff ? "off" : unreachable ? "unreachable" : "ready");
  const composerDisabled = $derived(aiOff || unreachable);
  const placeholder = $derived(
    aiOff
      ? "AI is off"
      : unreachable
        ? "Not available right now"
        : "Ask about your files, projects, or activity",
  );

  let composer = $state<Composer | null>(null);
</script>

<div class="layout">
  <div class="chat">
    <ChatThread
      {emptyVariant}
      showEmpty={capLoaded}
      {aiReady}
      onstarter={(text) => composer?.setText(text)}
      onretry={loadCapability}
    />
    <div class="foot">
      <Composer bind:this={composer} disabled={composerDisabled} {placeholder} />
      <!-- The steady-state posture lives in the composer foot now; this line is
           warning-only (AI off or unreachable). It shows only once there are
           messages: on an empty chat the centred empty state already carries
           the same off / unreachable notice, so this would double it. -->
      {#if !aiReady && $messages.length > 0}
        <CapabilityBar {capability} loaded={capLoaded} onretry={loadCapability} />
      {/if}
    </div>
  </div>
  {#if $openArtifact}
    <ArtifactPanel artifact={$openArtifact} onclose={closePane} />
  {/if}
</div>

<style>
  /* Fill the shell's content area (not 100vh, which overflows the shell and
     adds a second outer scrollbar). The chat scrolls inside; the pane sits
     beside it at full height. */
  .layout {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .chat {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .foot {
    flex-shrink: 0;
    padding-bottom: var(--space-row, 0.75rem);
  }
</style>
