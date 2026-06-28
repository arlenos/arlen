<script lang="ts">
  /// An agent action that needs confirmation (high-impact / irreversible /
  /// external), inline in the transcript (harness-redesign-plan.md "the gate").
  /// More than Approve/Deny: "Always allow" opens a scope submenu that creates a
  /// granular, revocable capability grant (action-type x scope) - not a session
  /// toggle. A broad grant raises a standing-permission warning; a narrow one
  /// (this project) applies directly. Pull-not-push: nothing runs until chosen.
  import { TriangleAlert, ChevronDown } from "@lucide/svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";

  let {
    title,
    detail,
    onapprove,
    ondeny,
    onalways,
  }: {
    /// The proposed action, one line ("write FILE_PART_OF (3 files → Arlen)").
    title: string;
    /// What + why (predict-before-act), shown under the title.
    detail?: string;
    onapprove?: () => void;
    ondeny?: () => void;
    /// Create a grant at the chosen scope ("project" narrow, "type" broad).
    onalways?: (scope: "project" | "type") => void;
  } = $props();
</script>

<div class="gate" role="group" aria-label="Action needs confirmation">
  <div class="gate-head">
    <TriangleAlert size={15} strokeWidth={2} />
    <span class="gate-title">{title}</span>
  </div>
  {#if detail}
    <p class="gate-detail">{detail}</p>
  {/if}
  <div class="gate-actions">
    <Button variant="default" size="sm" onclick={() => onapprove?.()}>Approve</Button>
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button variant="outline" size="sm" {...props}>
            Always allow
            <ChevronDown size={13} strokeWidth={2} />
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content class="w-56">
        <DropdownMenu.Item onclick={() => onalways?.("project")}>
          Only in this project
        </DropdownMenu.Item>
        <DropdownMenu.Item onclick={() => onalways?.("type")}>
          This action type generally
        </DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
    <Button variant="ghost" size="sm" onclick={() => ondeny?.()}>Deny</Button>
  </div>
</div>

<style>
  .gate {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem var(--space-card, 1rem);
    border: 1px solid color-mix(in srgb, var(--color-warning, #d4b483) 40%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-warning, #d4b483) 8%, transparent);
  }
  .gate-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--color-warning, #d4b483);
  }
  .gate-head :global(svg) {
    flex-shrink: 0;
  }
  .gate-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .gate-detail {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .gate-actions {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    margin-top: 0.125rem;
  }
</style>
