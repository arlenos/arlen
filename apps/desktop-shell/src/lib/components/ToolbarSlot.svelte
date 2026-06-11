<!--
  Renders the focused app's `slot-toolbar` content. Mutually-
  exclusive: Quick Actions, Breadcrumb, or Progress. State
  comes from `toolbarStore.focusedToolbar`. Auto-clears (via
  the derived store) when focus moves to an app with no state.

  Action dispatch: clicking a Quick Action or Breadcrumb item
  invokes `dispatch_app_action` with the action string and the
  focused app's id. Backend routes to the app's window via a
  per-window Tauri event.
-->
<script lang="ts">
  import { focusedToolbar, focusedToolbarKey } from "$lib/stores/toolbarStore";
  import { invoke } from "@tauri-apps/api/core";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import * as Icons from "lucide-svelte";

  /** Resolve a Lucide icon name (kebab-case) to its component. */
  function lookupIcon(name: string): typeof Icons.Circle {
    const pascal = name
      .split("-")
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join("");
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const candidate = (Icons as Record<string, any>)[pascal];
    return candidate ?? Icons.Circle;
  }

  async function dispatch(action: string) {
    const key = $focusedToolbarKey;
    if (!key) return;
    try {
      // window_id is the cosmic-toplevel id of the focused
      // window. Source-app's tauri-plugin-shell consumer maps
      // this back to its WebviewWindow via the same id —
      // window_id is end-to-end opaque on the shell side.
      await invoke("dispatch_app_action", {
        appId: key.appId,
        windowId: key.windowId,
        action,
      });
    } catch (e) {
      console.warn("dispatch_app_action failed:", e);
    }
  }
</script>

{#if $focusedToolbar.kind === "quick-actions" && $focusedToolbar.actions.length > 0}
  <div class="toolbar-quick-actions" data-tauri-drag-region={false}>
    {#each $focusedToolbar.actions as action (action.action)}
      {@const Icon = lookupIcon(action.icon)}
      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              class="qa-btn"
              class:toggle={action.toggle}
              class:active={action.toggle && action.active}
              aria-label={action.tooltip}
              onclick={() => dispatch(action.action)}
            >
              <Icon size={14} />
            </button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.TooltipContent side="bottom">
          {action.tooltip}
        </Tooltip.TooltipContent>
      </Tooltip.Root>
    {/each}
  </div>
{:else if $focusedToolbar.kind === "breadcrumb"}
  <nav class="toolbar-breadcrumb" data-tauri-drag-region={false}>
    {#each $focusedToolbar.items as item, i (item.action + i)}
      {#if i > 0}
        <span class="bc-sep">/</span>
      {/if}
      <button class="bc-seg" onclick={() => dispatch(item.action)}>
        {item.label}
      </button>
    {/each}
  </nav>
{:else if $focusedToolbar.kind === "progress"}
  <div
    class="toolbar-progress"
    role="progressbar"
    aria-label={$focusedToolbar.progress.label ?? "Progress"}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(
      Math.max(0, Math.min(1, $focusedToolbar.progress.value)) * 100,
    )}
  >
    <div
      class="tp-fill"
      style="width: {Math.max(0, Math.min(1, $focusedToolbar.progress.value)) * 100}%"
    ></div>
    {#if $focusedToolbar.progress.label}
      <span class="tp-label">{$focusedToolbar.progress.label}</span>
    {/if}
  </div>
{/if}

<style>
  .toolbar-quick-actions {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .qa-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    background: transparent;
    color: var(--color-fg);
    border-radius: var(--radius-chip);
    transition: background var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .qa-btn:hover {
    background: color-mix(in srgb, var(--color-fg) 10%, transparent);
  }
  .qa-btn.toggle.active {
    background: color-mix(in srgb, var(--color-accent) 25%, transparent);
    color: var(--color-accent);
  }

  .toolbar-breadcrumb {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--color-fg-muted);
  }
  .bc-seg {
    border: none;
    background: transparent;
    color: var(--color-fg-muted);
    padding: 2px 4px;
    border-radius: var(--radius-chip);
  }
  .bc-seg:hover {
    background: color-mix(in srgb, var(--color-fg) 10%, transparent);
    color: var(--color-fg);
  }
  .bc-sep {
    color: var(--color-fg-muted);
    opacity: 0.5;
  }

  .toolbar-progress {
    position: relative;
    width: 200px;
    height: 4px;
    background: color-mix(in srgb, var(--color-fg) 10%, transparent);
    border-radius: var(--radius-full);
    overflow: hidden;
  }
  .tp-fill {
    height: 100%;
    background: var(--color-accent);
    transition: width var(--duration-fast, 150ms) ease;
  }
  .tp-label {
    position: absolute;
    left: 50%;
    top: -16px;
    transform: translateX(-50%);
    font-size: 11px;
    color: var(--color-fg-muted);
    white-space: nowrap;
  }
</style>
