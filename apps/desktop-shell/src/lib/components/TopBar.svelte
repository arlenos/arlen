<script lang="ts">
  import { activeAppName } from "$lib/stores/windows";
  import { timeString, dateString } from "$lib/stores/status";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Wifi, Volume2, Battery, Settings } from "lucide-svelte";
</script>

<!--
  Top Bar: persistent narrow bar anchored to the top of the screen.
  Design: macOS/GNOME inspired single shared bar.

  Left:   Active app name + app menu (menu coming in Phase 3)
  Center: Active project name when Focus Mode active (Phase 3)
  Right:  System status indicators + clock
-->
<div class="topbar shell-surface" data-tauri-drag-region>

  <!-- Left: App name -->
  <div class="topbar-left" data-tauri-drag-region>
    <span class="app-name">
      {$activeAppName || "Lunaris"}
    </span>
  </div>

  <!-- Center: Focus Mode project name (Phase 3) -->
  <div class="topbar-center" data-tauri-drag-region>
    <!-- placeholder -->
  </div>

  <!-- Right: Status indicators + clock -->
  <div class="topbar-right">
    <Button variant="ghost" size="icon" class="status-btn" aria-label="Network">
      <Wifi size={14} strokeWidth={1.5} />
    </Button>

    <Button variant="ghost" size="icon" class="status-btn" aria-label="Volume">
      <Volume2 size={14} strokeWidth={1.5} />
    </Button>

    <Button variant="ghost" size="icon" class="status-btn" aria-label="Battery">
      <Battery size={14} strokeWidth={1.5} />
    </Button>

    <Separator orientation="vertical" class="h-4 mx-1 opacity-30" />

    <button class="clock" aria-label="Date and time">
      <span class="time">{$timeString}</span>
      <span class="date">{$dateString}</span>
    </button>

    <Button variant="ghost" size="icon" class="status-btn" aria-label="Quick Settings">
      <Settings size={14} strokeWidth={1.5} />
    </Button>
  </div>

</div>

<style>
  .topbar {
    display: flex;
    align-items: center;
    height: 28px;
    width: 100%;
    padding: 0 4px;
    position: relative;
    user-select: none;
    flex-shrink: 0;
    background: var(--background);
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }

  .app-name {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding-left: 8px;
  }

  .topbar-center {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    justify-content: flex-end;
    padding-right: 4px;
  }

  .topbar-right :global(.status-btn) {
    width: 26px;
    height: 22px;
    padding: 0;
    opacity: 1;
		color: var(--foreground);
  }

  .topbar-right :global(.status-btn:hover) {
    opacity: 1;
  }

  .clock {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 0 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    border-radius: 4px;
    transition: background 0.1s;
  }

  .clock:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .time {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
    line-height: 1.2;
  }

  .date {
    font-size: 0.625rem;
    color: var(--muted-foreground);
    line-height: 1.2;
  }
</style>
