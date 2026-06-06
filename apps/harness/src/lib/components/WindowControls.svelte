<script lang="ts">
  /// Client-side window decorations: minimize / maximize / close.
  /// Rendered in the titlebar because the app runs with
  /// `decorations: false` (Lunaris CSD). App-local chrome by design —
  /// the SDK `WindowControls` is the shell-overlay variant and is not
  /// imported by individual apps. Token-styled so it matches the canon.
  import { Minus, Square, X } from "@lucide/svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  async function minimize() {
    await getCurrentWindow().minimize();
  }
  async function toggleMax() {
    const w = getCurrentWindow();
    (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
  }
  async function close() {
    await getCurrentWindow().close();
  }
</script>

<div class="flex items-center gap-0.5">
  <button type="button" class="wc-btn" onclick={minimize} aria-label="Minimize" title="Minimize">
    <Minus size={12} strokeWidth={2} />
  </button>
  <button type="button" class="wc-btn" onclick={toggleMax} aria-label="Maximize" title="Maximize">
    <Square size={10} strokeWidth={2} />
  </button>
  <button type="button" class="wc-btn wc-close" onclick={close} aria-label="Close" title="Close">
    <X size={12} strokeWidth={2} />
  </button>
</div>

<style>
  .wc-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--color-fg-secondary);
    border-radius: var(--radius-input);
    cursor: pointer;
    transition: background-color 150ms ease, color 150ms ease;
  }
  .wc-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }
  .wc-close:hover {
    background: #ef4444;
    color: white;
  }
</style>
