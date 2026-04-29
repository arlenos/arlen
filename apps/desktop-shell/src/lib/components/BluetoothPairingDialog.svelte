<script lang="ts">
  /// Modal dialog driven by the `bluetoothPairing` store.
  ///
  /// Mounted once globally in `+layout.svelte`. The component is
  /// inert (`open={false}`) when no pairing request is active, so
  /// it has zero visual cost in the common case.
  ///
  /// One layout per request kind. The discriminated union is
  /// flattened with `{#if request.kind === "..."}` so each branch
  /// can read its own typed fields without casts.
  ///
  /// User actions:
  /// - Cancel button / Escape / backdrop click → respond with
  ///   `reject`, dialog closes immediately.
  /// - Confirm / Pair / Allow button → respond with the appropriate
  ///   typed payload. Inputs are validated client-side before send.

  import { onMount, onDestroy } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
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

  // Reset inputs whenever the active request changes so a leftover
  // value from a prior dialog can't get sent into a new one.
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
        return `Pair with ${req.deviceName}?`;
      case "pinCodeInput":
        return `Enter PIN for ${req.deviceName}`;
      case "passkeyInput":
        return `Enter passkey for ${req.deviceName}`;
      case "displayPinCode":
        return `Pair with ${req.deviceName}`;
      case "displayPasskey":
        return `Pair with ${req.deviceName}`;
      case "authorization":
        return `Pair with ${req.deviceName}?`;
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

  function showsConfirmButton(req: PairRequest): boolean {
    // Display variants are informational — the user types on the
    // peer device, no confirmation here. Only Cancel is meaningful.
    return req.kind !== "displayPinCode" && req.kind !== "displayPasskey";
  }

  function isConfirmDisabled(req: PairRequest): boolean {
    if (req.kind === "pinCodeInput") return !pinInRange();
    if (req.kind === "passkeyInput") return !passkeyInRange();
    return false;
  }
</script>

{#if $current}
  {@const request = $current}
  <Dialog.Root
    open={true}
    onOpenChange={(v) => {
      if (!v) onCancel();
    }}
  >
    <Dialog.Content>
      <Dialog.Header>
        <Dialog.Title>{titleFor(request)}</Dialog.Title>
        <Dialog.Description>{descriptionFor(request)}</Dialog.Description>
      </Dialog.Header>

      {#if request.kind === "confirmation"}
        <div class="code-display" aria-label="Pairing code">
          {pad6(request.passkey)}
        </div>
      {:else if request.kind === "displayPinCode"}
        <div class="code-display" aria-label="PIN code">
          {request.pinCode}
        </div>
      {:else if request.kind === "displayPasskey"}
        <div class="code-display" aria-label="Passkey">
          {pad6(request.passkey)}
        </div>
        <div class="entered-progress" aria-label="Digits entered on device">
          {#each Array.from({ length: 6 }) as _, i}
            <span
              class="entered-slot"
              class:filled={i < request.entered}
            ></span>
          {/each}
        </div>
      {:else if request.kind === "pinCodeInput"}
        <div class="input-row">
          <Input
            bind:value={pinInput}
            maxlength={16}
            autofocus
            placeholder="PIN"
            aria-label="PIN code"
          />
        </div>
      {:else if request.kind === "passkeyInput"}
        <div class="input-row">
          <Input
            type="number"
            bind:value={passkeyInput}
            min={0}
            max={999999}
            autofocus
            placeholder="000000"
            aria-label="Passkey"
          />
        </div>
      {:else if request.kind === "authorizeService"}
        <div class="meta">
          {request.deviceAddress} · {request.uuidLabel}
        </div>
      {/if}

      <Dialog.Footer>
        <Button variant="outline" onclick={onCancel}>Cancel</Button>
        {#if showsConfirmButton(request)}
          <Button
            disabled={isConfirmDisabled(request)}
            onclick={onConfirm}
          >
            {confirmLabel(request)}
          </Button>
        {/if}
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .code-display {
    margin: 4px 0 8px;
    padding: 14px 16px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 1.6rem;
    font-weight: 600;
    letter-spacing: 0.18em;
    text-align: center;
    color: var(--color-fg-shell);
  }

  .entered-progress {
    display: flex;
    justify-content: center;
    gap: 6px;
    margin: 0 0 4px;
  }

  .entered-slot {
    width: 22px;
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    transition: background 0.15s ease;
  }

  .entered-slot.filled {
    background: var(--color-accent);
  }

  .input-row {
    margin: 4px 0 8px;
  }

  .meta {
    margin: 4px 0 8px;
    font-size: 0.85rem;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }
</style>
