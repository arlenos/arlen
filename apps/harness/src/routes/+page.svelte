<script lang="ts">
  /// Conversation surface (ai-app.md §2.1) — the full GUI door for
  /// human-initiated, multi-turn conversation against the ai-daemon
  /// query path, with streaming, visible tool calls, and graph-data
  /// citations.
  ///
  /// A1 is the skeleton: the conversation layout (a scrollable message
  /// area + a composer) with an honest empty state. The daemon wiring,
  /// streaming, and message/tool-call rendering land in A2/A3 — the
  /// composer is intentionally inert here rather than faking replies.
  import { Input } from "@lunaris/ui-kit/components/ui/input";
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { MessageSquare, ArrowUp } from "@lucide/svelte";
</script>

<div class="conversation">
  <div class="messages">
    <div class="empty-state">
      <MessageSquare size={28} strokeWidth={1.5} />
      <p class="empty-title">Ask the assistant</p>
      <p class="empty-sub">
        Multi-turn conversation against the on-device AI, with streaming
        replies and visible tool calls. Connecting to the AI daemon lands
        in the next build step.
      </p>
    </div>
  </div>

  <div class="composer">
    <Input
      placeholder="Conversation backend lands in A2"
      disabled
      aria-label="Message"
    />
    <Button size="icon" variant="default" disabled aria-label="Send">
      <ArrowUp size={16} strokeWidth={2} />
    </Button>
  </div>
</div>

<style>
  .conversation {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 0.5rem;
    max-width: 26rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .empty-title {
    margin: 0.25rem 0 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .empty-sub {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
  }
  .composer {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid var(--color-border);
  }
  .composer :global(input) {
    flex: 1;
  }
</style>
