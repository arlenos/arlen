<script lang="ts">
  /// The session picker, kept deliberately quiet: most boxes have only the
  /// Arlen session, so a single session renders as a plain label, not a
  /// control. More than one falls back to the kit PopoverSelect.
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import type { Session } from "$lib/greeter";

  let {
    sessions,
    value,
    onchange,
  }: {
    sessions: Session[];
    value: string;
    onchange: (id: string) => void;
  } = $props();

  const options = $derived(sessions.map((s) => ({ value: s.id, label: s.name })));
  const single = $derived(sessions.length <= 1);
  const onlyLabel = $derived(sessions[0]?.name ?? "Arlen");
</script>

{#if single}
  <span class="label" id="greeter-session">{onlyLabel}</span>
{:else}
  <PopoverSelect
    {value}
    {options}
    {onchange}
    ariaLabel="Choose a session"
    width="11rem"
  />
{/if}

<style>
  .label {
    font-size: calc(0.8125rem * var(--greeter-scale, 1));
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  :global([data-contrast="high"]) .label {
    color: #ffffff;
  }
</style>
