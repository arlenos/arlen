<script lang="ts">
  /// Sandboxed Tier 2 module host (ds#77 path).
  ///
  /// Mounts the module as an iframe served over `module://`. The
  /// scheme handler in `module_scheme.rs` proxies the asset request
  /// through `lunaris-modulesd` for nonce + CSP enforcement; this
  /// component only needs to know the URL the daemon mints.
  ///
  /// Capability-gated host calls (`graph.query`, `network.fetch`,
  /// `events.emit`) flow shell → modulesd via `module_host_call`
  /// Tauri command. The iframe never sees direct daemon access.

  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { themeVars } from "$lib/theme/index.js";
  import {
    isModuleToHost,
    type HostCall,
    type HostToModule,
    type SearchResult,
    type ThemeTokens,
  } from "./postmsg.js";

  interface ModuleInfo {
    id: string;
    name: string;
    enabled: boolean;
    failed: boolean;
  }

  interface Props {
    module: ModuleInfo;
    /** Slot identifier passed to `mint_iframe`; daemon uses it for telemetry. */
    slot: string;
    /** Called when the module sends search results. */
    onResults?: (results: SearchResult[]) => void;
    /** Called when the module pushes an indicator update. */
    onIndicatorUpdate?: (update: { icon?: string; label?: string; badge?: string | number }) => void;
    /** Called when the module reports ready. */
    onReady?: () => void;
    /** Called on module-side errors. */
    onError?: (message: string) => void;
  }

  let {
    module,
    slot,
    onResults,
    onIndicatorUpdate,
    onReady,
    onError,
  }: Props = $props();

  let iframe: HTMLIFrameElement | undefined = $state(undefined);
  let iframeUrl = $state<string | null>(null);
  let nonce = $state<string | null>(null);
  let mountError = $state<string | null>(null);

  /// Send a typed message to the iframe.
  function sendToIframe(msg: HostToModule) {
    if (!iframe?.contentWindow) return;
    iframe.contentWindow.postMessage(msg, "*");
  }

  /// Build a ThemeTokens payload from the current cssVars.
  function buildTheme(): ThemeTokens | null {
    const v = $themeVars;
    if (!v) return null;
    return {
      cssVars: v.variables,
      variant: v.variant,
    };
  }

  /// Forward a `host.call` from the iframe through modulesd.
  async function handleHostCall(requestId: string, call: HostCall) {
    if (!nonce) return;
    try {
      const reply = await invoke<{ type: string; [k: string]: unknown }>(
        "module_host_call",
        { nonce, callPayload: hostCallToProtoVariant(call) },
      );
      sendToIframe({
        type: "host.reply",
        requestId,
        reply: replyFromUiShape(reply),
      });
    } catch (err) {
      sendToIframe({
        type: "host.reply",
        requestId,
        reply: {
          type: "error",
          code: "internal",
          message: String(err),
        },
      });
    }
  }

  /// The Rust HostCall variant uses snake_case tag values
  /// (graph_query, network_fetch, ...) per `modulesd_proto`.
  function hostCallToProtoVariant(call: HostCall): unknown {
    switch (call.type) {
      case "graph.query":
        return { type: "graph_query", cypher: call.cypher };
      case "graph.write":
        return { type: "graph_write", cypher: call.cypher };
      case "network.fetch":
        return {
          type: "network_fetch",
          url: call.url,
          headers: call.headers,
        };
      case "events.emit":
        return {
          type: "events_emit",
          event_type: call.eventType,
          payload_b64: call.payloadB64,
        };
    }
  }

  /// modulesd UiHostReply uses camelCase field names; map to the
  /// HostReply shape the iframe SDK expects.
  function replyFromUiShape(r: { type: string; [k: string]: unknown }) {
    switch (r.type) {
      case "graphResult":
        return { type: "graph.result" as const, rows: r.rows as string };
      case "networkBody":
        return {
          type: "network.body" as const,
          status: r.status as number,
          bodyB64: r.bodyB64 as string,
        };
      case "acked":
        return { type: "acked" as const };
      case "error":
      default:
        return {
          type: "error" as const,
          code: (r.code as
            | "not_found"
            | "permission_denied"
            | "module_failed"
            | "timeout"
            | "invalid_request"
            | "internal") ?? "internal",
          message: (r.message as string) ?? "unknown",
        };
    }
  }

  /// Demux postMessages from the iframe.
  function handleMessage(event: MessageEvent) {
    if (!iframe || event.source !== iframe.contentWindow) return;
    const data = event.data;
    if (!isModuleToHost(data)) return;

    switch (data.type) {
      case "ready": {
        const theme = buildTheme();
        if (theme) {
          sendToIframe({
            type: "init",
            capabilities: {},
            theme,
          });
        }
        onReady?.();
        break;
      }
      case "search.results":
        onResults?.(data.results);
        break;
      case "indicator.update":
        onIndicatorUpdate?.({
          icon: data.icon,
          label: data.label,
          badge: data.badge,
        });
        break;
      case "module.error":
        onError?.(data.message);
        break;
      case "host.call":
        handleHostCall(data.requestId, data.call);
        break;
    }
  }

  /// Push a search query to the iframe. Caller correlates results
  /// via `requestId` in the response.
  export function search(requestId: string, query: string) {
    sendToIframe({ type: "search", requestId, query });
  }

  // ---------------------------------------------------------------
  // Lifecycle: mint, mount, unmount.
  // ---------------------------------------------------------------

  onMount(async () => {
    if (!module.enabled || module.failed) return;
    try {
      const issued = await invoke<{ url: string; nonce: string }>(
        "mint_iframe",
        { moduleId: module.id, slot },
      );
      nonce = issued.nonce;
      iframeUrl = issued.url;
    } catch (err) {
      mountError = String(err);
    }
    window.addEventListener("message", handleMessage);
  });

  onDestroy(() => {
    window.removeEventListener("message", handleMessage);
    sendToIframe({ type: "destroy" });
    // Nonce revocation happens daemon-side when the module is
    // disabled. For a transient unmount (e.g. component re-render)
    // we leave the nonce live; the next mount mints a fresh one.
  });

  // Theme propagation: any change in $themeVars pushes a typed
  // theme.changed message to the iframe. Without this the module
  // renders against stale tokens after the user switches themes.
  $effect(() => {
    const theme = buildTheme();
    if (!theme) return;
    sendToIframe({ type: "theme.changed", theme });
  });
</script>

{#if mountError}
  <div class="mod-failed" title={mountError}>
    <span class="mod-failed-glyph">!</span>
  </div>
{:else if module.failed}
  <button
    class="mod-failed"
    type="button"
    title="Module failed; click to retry"
    onclick={async () => {
      try {
        await invoke("retry_module", { moduleId: module.id });
      } catch {}
    }}
  >
    <span class="mod-failed-glyph">↻</span>
  </button>
{:else if iframeUrl}
  <iframe
    bind:this={iframe}
    src={iframeUrl}
    sandbox="allow-scripts"
    title={module.name}
    class="module-iframe"
    data-module-id={module.id}
  ></iframe>
{/if}

<style>
  .module-iframe {
    border: none;
    width: 100%;
    height: 100%;
    background: transparent;
    color-scheme: inherit;
  }
  .mod-failed {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--color-fg-muted, #a1a1aa);
  }
  .mod-failed-glyph {
    font-size: 12px;
    font-weight: 600;
  }
</style>
