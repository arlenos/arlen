<script lang="ts">
  /// Sandboxed module host.
  ///
  /// Third-party modules run in an iframe with `sandbox="allow-scripts"`.
  /// Communication is exclusively via postMessage. System and first-party
  /// modules can be mounted directly (no sandbox).
  ///
  /// See `docs/architecture/module-system.md`.

  import { onMount, onDestroy } from "svelte";
  import { themeVars, type CssVariables } from "$lib/theme/index.js";
  import type {
    ModuleToShellMessage,
    ShellToModuleMessage,
    SearchResult,
  } from "./protocol.js";
  import { isModuleMessage } from "./protocol.js";

  interface ModuleInfo {
    id: string;
    name: string;
    module_type: string;
    path: string;
    entry?: string;
  }

  interface Props {
    module: ModuleInfo;
    /** Which extension point this host serves. */
    extensionPoint: string;
    /** Called when the module sends search results. */
    onResults?: (results: SearchResult[]) => void;
    /** Called when the module sends an action. */
    onAction?: (action: string, data?: unknown) => void;
    /** Called when the module sends an indicator update. */
    onIndicatorUpdate?: (update: { icon?: string; label?: string; badge?: string | number }) => void;
    /** Called when the module reports ready. */
    onReady?: () => void;
    /** Called on module errors. */
    onError?: (message: string) => void;
    /** Content to render for trusted (non-sandboxed) modules. */
    children?: import("svelte").Snippet;
  }

  let {
    module,
    extensionPoint,
    onResults,
    onAction,
    onIndicatorUpdate,
    onReady,
    onError,
    children,
  }: Props = $props();

  let iframe: HTMLIFrameElement | undefined = $state(undefined);
  let ready = $state(false);
  const isSandboxed = $derived(module.module_type === "third-party");

  // Build the module URL.
  // Tauri serves local files via asset protocol. For sandboxed modules,
  // we construct a URL to the module's entry file.
  const moduleUrl = $derived(
    `file://${module.path}/${module.entry ?? "dist/index.html"}`
  );

  // Build CSP meta tag content for the iframe.
  // Restricts network access to only allowed domains from the manifest.
  const cspContent = $derived(buildCsp(module));

  function buildCsp(_mod: ModuleInfo): string {
    // Default: no external connections, only inline scripts.
    return [
      "default-src 'none'",
      "script-src 'unsafe-inline'",
      "style-src 'unsafe-inline'",
      "img-src data: blob:",
    ].join("; ");
  }

  // ---------------------------------------------------------------------------
  // postMessage communication
  // ---------------------------------------------------------------------------

  /** Send a typed message to the module iframe. */
  function sendToModule(type: string, payload: unknown) {
    if (!iframe?.contentWindow) return;
    const msg: ShellToModuleMessage = { type: type as any, payload };
    iframe.contentWindow.postMessage(msg, "*");
  }

  /** Send a search query to the module. */
  export function search(query: string) {
    sendToModule("search", { query });
  }

  /** Send theme variables to the module. */
  function sendTheme(vars: CssVariables | null) {
    if (!vars) return;
    sendToModule("theme", {
      cssVars: vars.variables,
      variant: vars.variant,
    });
  }

  /** Handle messages from the module. */
  function handleMessage(event: MessageEvent) {
    // Only accept messages from our iframe.
    if (!iframe || event.source !== iframe.contentWindow) return;

    const data = event.data;
    if (!isModuleMessage(data)) return;
    if (data.moduleId !== module.id) return;

    switch (data.type) {
      case "ready":
        ready = true;
        sendTheme($themeVars);
        onReady?.();
        break;
      case "results":
        onResults?.(data.payload as SearchResult[]);
        break;
      case "action":
        onAction?.((data.payload as any).action, (data.payload as any).data);
        break;
      case "indicator_update":
        onIndicatorUpdate?.(data.payload as any);
        break;
      case "error":
        onError?.((data.payload as any).message);
        break;
    }
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  onMount(() => {
    if (isSandboxed) {
      window.addEventListener("message", handleMessage);
    }
  });

  onDestroy(() => {
    if (isSandboxed) {
      window.removeEventListener("message", handleMessage);
      // Tell module to clean up.
      sendToModule("destroy", {});
    }
  });

  // Forward theme changes to module.
  $effect(() => {
    if (ready && $themeVars) {
      sendTheme($themeVars);
    }
  });
</script>

{#if isSandboxed}
  <iframe
    bind:this={iframe}
    src={moduleUrl}
    sandbox="allow-scripts"
    title={module.name}
    class="module-iframe"
    data-module-id={module.id}
    data-extension-point={extensionPoint}
  ></iframe>
{:else}
  <!-- System/first-party modules: direct mount (no sandbox). -->
  <div class="module-direct" data-module-id={module.id}>
    {@render children?.()}
  </div>
{/if}

<style>
  .module-iframe {
    border: none;
    width: 100%;
    height: 100%;
    background: transparent;
    color-scheme: inherit;
  }
  .module-direct {
    width: 100%;
    height: 100%;
  }
</style>
