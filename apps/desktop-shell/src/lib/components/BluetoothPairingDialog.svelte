<script lang="ts">
  /// BlueZ Agent1 pairing requests, rendered through the shared `ConsentCard`
  /// so the whole permission cluster reads as one dialog (system-dialog-plan.md).
  /// The requester is the DEVICE (name + address, not a system-attested app id,
  /// so `attested={false}`), toned caution because it is a foreign device.
  ///
  /// Mounted once globally in `+layout.svelte`. Inert when no request is active.
  /// The allow/deny kinds (confirmation / authorization / authorizeService) are
  /// a yes-no consent; the display + input kinds (pin/passkey) are a code-exchange
  /// mechanic - they keep their own body + affordances (Cancel, or Cancel/OK), the
  /// same way the xdg-portal file-PICKER stays its own interaction, but wear the
  /// shared chrome for one consistent look.
  ///
  /// User actions: Cancel / Escape / backdrop → reject, closes immediately.
  /// Confirm / Pair / Allow / OK → the appropriate typed payload; inputs are
  /// validated client-side before send.
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import ConsentCard from "$lib/components/ConsentCard.svelte";
  import {
    current,
    init,
    dispose,
    respond,
    type PairRequest,
  } from "$lib/stores/bluetoothPairing";

  let pinInput = $state("");
  let passkeyInput = $state("");

  onMount(() => {
    init();
  });

  onDestroy(() => {
    dispose();
  });

  // Reset inputs whenever the active request changes so a leftover value from a
  // prior dialog can't get sent into a new one.
  let lastId: number | null = null;
  $effect(() => {
    const cur = $current;
    if (cur && cur.id !== lastId) {
      lastId = cur.id;
      pinInput = "";
      passkeyInput = "";
    } else if (!cur) {
      lastId = null;
    }
  });

  function titleFor(req: PairRequest): string {
    switch (req.kind) {
      case "confirmation":
      case "authorization":
        return `Pair with ${req.deviceName}?`;
      case "pinCodeInput":
        return `Enter PIN for ${req.deviceName}`;
      case "passkeyInput":
        return `Enter passkey for ${req.deviceName}`;
      case "displayPinCode":
      case "displayPasskey":
        return `Pair with ${req.deviceName}`;
      case "authorizeService":
        return `Allow ${req.deviceName} to use ${req.uuidLabel}?`;
    }
  }

  function descriptionFor(req: PairRequest): string {
    switch (req.kind) {
      case "confirmation":
        return "Confirm that the same code is shown on the other device.";
      case "pinCodeInput":
        return "1 to 16 characters. The PIN is set on the device itself.";
      case "passkeyInput":
        return "Numeric code, 0 to 999999.";
      case "displayPinCode":
        return "Type this PIN on the device.";
      case "displayPasskey":
        return "Type this code on the device.";
      case "authorization":
        return "An incoming pairing request without security verification.";
      case "authorizeService":
        return "Allow this service for as long as the device stays paired.";
    }
  }

  function pad6(n: number): string {
    return String(n).padStart(6, "0");
  }

  function passkeyInRange(): boolean {
    if (passkeyInput === "") return false;
    const n = Number(passkeyInput);
    return Number.isInteger(n) && n >= 0 && n <= 999999;
  }

  function pinInRange(): boolean {
    return pinInput.length >= 1 && pinInput.length <= 16;
  }

  async function onCancel() {
    if (!$current) return;
    await respond($current.id, { kind: "reject" });
  }

  // A pending request must always be cancellable by Escape. `open` is controlled
  // (static true; the request clears via the store), which does not reliably fire
  // the primitive's escape-close, so reject explicitly here.
  function onWindowKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape" && get(current)) void onCancel();
  }

  async function onConfirm() {
    const cur = $current;
    if (!cur) return;
    if (cur.kind === "confirmation" || cur.kind === "authorization" || cur.kind === "authorizeService") {
      await respond(cur.id, { kind: "confirm" });
    } else if (cur.kind === "pinCodeInput") {
      if (!pinInRange()) return;
      await respond(cur.id, { kind: "pinCode", value: pinInput });
    } else if (cur.kind === "passkeyInput") {
      if (!passkeyInRange()) return;
      await respond(cur.id, { kind: "passkey", value: Number(passkeyInput) });
    }
  }

  function confirmLabel(req: PairRequest): string {
    switch (req.kind) {
      case "confirmation":
      case "displayPinCode":
      case "displayPasskey":
        return "Pair";
      case "pinCodeInput":
      case "passkeyInput":
        return "OK";
      case "authorization":
      case "authorizeService":
        return "Allow";
    }
  }

  // The allow/deny kinds are rejecting a request (Deny); the input kinds are
  // cancelling your own entry (Cancel).
  function rejectLabel(req: PairRequest): string {
    return req.kind === "confirmation" || req.kind === "authorization" || req.kind === "authorizeService"
      ? "Deny"
      : "Cancel";
  }

  function showsConfirmButton(req: PairRequest): boolean {
    // Display variants are informational — the user types on the peer device,
    // no confirmation here. Only the reject button is meaningful.
    return req.kind !== "displayPinCode" && req.kind !== "displayPasskey";
  }

  function isConfirmDisabled(req: PairRequest): boolean {
    if (req.kind === "pinCodeInput") return !pinInRange();
    if (req.kind === "passkeyInput") return !passkeyInRange();
    return false;
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if $current}
  {@const request = $current}

  {#snippet body()}
    {#if request.kind === "confirmation"}
      <div class="code-display">{pad6(request.passkey)}</div>
    {:else if request.kind === "displayPinCode"}
      <div class="code-display">{request.pinCode}</div>
    {:else if request.kind === "displayPasskey"}
      <div class="code-display">{pad6(request.passkey)}</div>
      <div
        class="entered-progress"
        role="progressbar"
        aria-label="Digits entered on device"
        aria-valuemin={0}
        aria-valuemax={6}
        aria-valuenow={request.entered}
      >
        {#each Array.from({ length: 6 }) as _, i}
          <span class="entered-slot" class:filled={i < request.entered}></span>
        {/each}
      </div>
    {:else if request.kind === "pinCodeInput"}
      <div class="input-row">
        <Input bind:value={pinInput} maxlength={16} autofocus placeholder="PIN" aria-label="PIN code" />
      </div>
    {:else if request.kind === "passkeyInput"}
      <div class="input-row">
        <Input
          bind:value={passkeyInput}
          inputmode="numeric"
          maxlength={6}
          autofocus
          placeholder="000000"
          aria-label="Passkey"
          oninput={() => {
            passkeyInput = passkeyInput.replace(/[^0-9]/g, "");
          }}
        />
      </div>
    {:else if request.kind === "authorizeService"}
      <div class="meta">
        <span>{request.deviceAddress}</span>
        <span>{request.uuidLabel}</span>
      </div>
    {/if}
    <p class="bt-note">{descriptionFor(request)}</p>
  {/snippet}

  {#snippet footer()}
    <Button variant="outline" onclick={onCancel}>{rejectLabel(request)}</Button>
    {#if showsConfirmButton(request)}
      <span style="flex:1"></span>
      <Button disabled={isConfirmDisabled(request)} onclick={onConfirm}>
        {confirmLabel(request)}
      </Button>
    {/if}
  {/snippet}

  <Dialog.Root
    open={true}
    onOpenChange={(v) => {
      if (!v) onCancel();
    }}
  >
    <Dialog.Content>
      <ConsentCard
        requesterName={request.deviceName}
        requesterId={request.deviceAddress}
        attested={false}
        tone="caution"
        title={titleFor(request)}
        {body}
        {footer}
      />
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .code-display {
    margin: 2px 0;
    padding: 14px 16px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.6rem;
    font-weight: 600;
    letter-spacing: 0.18em;
    text-align: center;
    color: var(--foreground);
  }

  .entered-progress {
    display: flex;
    justify-content: center;
    gap: 6px;
  }

  .entered-slot {
    width: 22px;
    height: 4px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    transition: background var(--duration-fast, 150ms) ease;
  }

  .entered-slot.filled {
    background: var(--color-accent);
  }

  .input-row {
    margin: 2px 0;
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--text-xs);
    font-family: var(--font-mono, ui-monospace, monospace);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .bt-note {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
