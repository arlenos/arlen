<script lang="ts">
  import { cn } from "$lib/utils";

  let {
    class: className,
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    disabled = false,
    onchange,
  }: {
    class?: string;
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    disabled?: boolean;
    onchange?: (value: number) => void;
  } = $props();

  const percent = $derived(((value - min) / (max - min)) * 100);

  function handleInput(e: Event) {
    const v = parseFloat((e.currentTarget as HTMLInputElement).value);
    value = v;
    onchange?.(v);
  }
</script>

<div
  class={cn("relative flex h-5 w-full items-center", className)}
  style="--value: {percent}%"
>
  <div class="absolute inset-x-0 h-1 rounded-full bg-muted"></div>
  <div
    class="absolute h-1 rounded-full bg-primary"
    style="width: var(--value)"
  ></div>
  <div
    class="absolute size-4 rounded-full border-2 border-primary bg-background shadow-sm"
    style="left: var(--value); transform: translateX(-50%)"
  ></div>
  <input
    type="range"
    {min}
    {max}
    {step}
    {value}
    {disabled}
    oninput={handleInput}
    class="absolute inset-0 w-full cursor-pointer appearance-none bg-transparent opacity-0"
  />
</div>
