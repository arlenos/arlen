<script lang="ts">
  import { cn } from "$lib/utils";
  import { ChevronDown } from "lucide-svelte";

  interface Option {
    value: string;
    label: string;
  }

  let {
    class: className,
    value = $bindable(""),
    options,
    placeholder = "Select...",
    onchange,
  }: {
    class?: string;
    value?: string;
    options: Option[];
    placeholder?: string;
    onchange?: (value: string) => void;
  } = $props();

  function handleChange(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    value = v;
    onchange?.(v);
  }
</script>

<div class={cn("relative", className)}>
  <select
    {value}
    onchange={handleChange}
    class="h-control w-full appearance-none rounded-md border border-border bg-input px-3 pr-8 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
  >
    {#if !value}
      <option value="" disabled>{placeholder}</option>
    {/if}
    {#each options as opt}
      <option value={opt.value}>{opt.label}</option>
    {/each}
  </select>
  <ChevronDown
    size={14}
    class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 opacity-50"
  />
</div>
