<script lang="ts">
  /// The stream's two empty states: the session list could not be read, and the
  /// list read fine but holds nothing. Both offer one button, and that is why
  /// this is a component rather than markup inside the route.
  ///
  /// A person here has no terminal and one thing to press. If the press changes
  /// nothing on screen the app is indistinguishable from a dead one, so the
  /// refusal has to be visible - and a state that only exists inline in a route
  /// cannot be mounted, driven and photographed. Extracting it is what makes the
  /// refused case verifiable at all; the route renders exactly this.
  import { newSessionFailed } from "$lib/stores/sessions";
  import { t } from "$lib/i18n/messages";

  let {
    kind,
    onretry,
  }: {
    /// `unreachable` when the list read failed, `none` when it read empty.
    kind: "unreachable" | "none";
    /// Runs the button's action: reload the list, or open a shell.
    onretry: () => void;
  } = $props();
</script>

<div class="stream-empty">
  <span class="stream-empty-title">
    {kind === "unreachable" ? $t("term.err.unreachable") : $t("term.err.noSession")}
  </span>
  <span class="stream-empty-hint">
    {kind === "unreachable" ? $t("term.err.unreachableHint") : $t("term.err.noSessionHint")}
  </span>
  <button class="stream-empty-btn" onclick={() => onretry()}>
    {kind === "unreachable" ? $t("term.err.tryAgain") : $t("term.sidebar.newSession")}
  </button>
  <!-- Only under the open-a-shell button, and deliberately NOT a second sentence
       about having failed - the title above already says that. It carries the one
       thing the static copy cannot: a likely cause. Its appearing at all is what
       acknowledges the press, which is the part that was missing; a panel that
       looks identical after a click cannot be told from a dead button. -->
  {#if kind === "none" && $newSessionFailed}
    <span class="stream-empty-failed" role="alert">{$t("term.err.newSessionFailed")}</span>
  {/if}
</div>

<style>
  .stream-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 100%;
    padding: 32px;
    text-align: center;
  }
  .stream-empty-title {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--foreground);
  }
  .stream-empty-hint {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .stream-empty-failed {
    margin-top: 8px;
    max-width: 34ch;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-error, #f87171) 85%, var(--foreground));
  }
  .stream-empty-btn {
    margin-top: 8px;
    height: var(--height-control, 28px);
    padding: 0 12px;
    border-radius: var(--radius-input);
    border: 1px solid var(--control-border);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: var(--text-xs);
  }
  .stream-empty-btn:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
</style>
