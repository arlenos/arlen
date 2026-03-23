<script lang="ts">
  import { applyTokens, PANDA_TOKENS } from "$lib/theme";
  import { windows } from "$lib/stores/windows";
  import "../app.css";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";

  // Apply tokens immediately before render
  applyTokens(PANDA_TOKENS);

  interface WindowPayload {
    event_type: string;
    app_id: string;
    title: string;
  }

  onMount(async () => {
    // Subscribe to window events from the Event Bus via Rust backend
    const unlistenFocused = await listen<WindowPayload>(
      "lunaris://window-focused",
      ({ payload }) => {
        windows.update((ws) => {
          // Mark all unfocused, then mark the new focused one
          const updated = ws.map((w) => ({ ...w, focused: false }));
          const existing = updated.find((w) => w.app_id === payload.app_id);
          if (existing) {
            existing.focused = true;
            existing.title = payload.title;
          } else {
            updated.push({
              id: payload.app_id,
              app_id: payload.app_id,
              title: payload.title,
              focused: true,
            });
          }
          return updated;
        });
      }
    );

    const unlistenOpened = await listen<WindowPayload>(
      "lunaris://window-opened",
      ({ payload }) => {
        windows.update((ws) => {
          if (!ws.find((w) => w.app_id === payload.app_id)) {
            ws.push({
              id: payload.app_id,
              app_id: payload.app_id,
              title: payload.title,
              focused: false,
            });
          }
          return ws;
        });
      }
    );

    const unlistenClosed = await listen<WindowPayload>(
      "lunaris://window-closed",
      ({ payload }) => {
        windows.update((ws) =>
          ws.filter((w) => w.app_id !== payload.app_id)
        );
      }
    );

    return () => {
      unlistenFocused();
      unlistenOpened();
      unlistenClosed();
    };
  });
</script>

<slot />
