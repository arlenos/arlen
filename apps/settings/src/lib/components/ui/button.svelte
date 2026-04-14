<script lang="ts" module>
  import { cn } from "$lib/utils";
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  export type ButtonVariant =
    | "default"
    | "outline"
    | "secondary"
    | "ghost"
    | "destructive";
  export type ButtonSize = "default" | "sm" | "icon" | "icon-sm";

  const variants: Record<ButtonVariant, string> = {
    default:
      "bg-primary text-primary-foreground hover:bg-primary/90",
    outline:
      "border border-border bg-transparent hover:bg-muted",
    secondary:
      "bg-secondary text-secondary-foreground hover:bg-secondary/80",
    ghost: "hover:bg-muted hover:text-foreground",
    destructive:
      "bg-destructive/10 text-destructive hover:bg-destructive/20",
  };

  const sizes: Record<ButtonSize, string> = {
    default: "h-9 px-4 gap-2",
    sm: "h-8 px-3 gap-1.5 text-xs",
    icon: "size-9",
    "icon-sm": "size-8",
  };
</script>

<script lang="ts">
  let {
    class: className,
    variant = "default",
    size = "default",
    type = "button",
    children,
    ...rest
  }: HTMLButtonAttributes & {
    variant?: ButtonVariant;
    size?: ButtonSize;
    children?: Snippet;
  } = $props();
</script>

<button
  {type}
  class={cn(
    "inline-flex items-center justify-center rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
    variants[variant],
    sizes[size],
    className
  )}
  {...rest}
>
  {@render children?.()}
</button>
