/// Frontend state for active BlueZ Agent1 pairing requests.
///
/// The Rust agent (`bluetooth_agent.rs`) emits Tauri events when
/// BlueZ calls one of the eight `Agent1` methods; this store turns
/// those events into a single reactive `current` slot that the
/// `BluetoothPairingDialog` component renders.
///
/// Architecture: see `docs/architecture/bluetooth-pairing.md`.
///
/// Concurrency: only one request is shown at a time. If a second
/// blocking request arrives while one is pending, the backend
/// rejects it with `org.bluez.Error.Rejected`; this store does not
/// queue or stack requests visually.

import { writable, type Readable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// Discriminated union for every kind of pairing request the user
/// might see. The `id` is the backend's request id and is what the
/// `respond` call references.
export type PairRequest =
  | {
      kind: "confirmation";
      id: number;
      deviceName: string;
      deviceAddress: string;
      passkey: number;
    }
  | {
      kind: "pinCodeInput";
      id: number;
      deviceName: string;
      deviceAddress: string;
    }
  | {
      kind: "passkeyInput";
      id: number;
      deviceName: string;
      deviceAddress: string;
    }
  | {
      kind: "displayPinCode";
      id: number;
      deviceName: string;
      deviceAddress: string;
      pinCode: string;
    }
  | {
      kind: "displayPasskey";
      id: number;
      deviceName: string;
      deviceAddress: string;
      passkey: number;
      entered: number;
    }
  | {
      kind: "authorization";
      id: number;
      deviceName: string;
      deviceAddress: string;
    }
  | {
      kind: "authorizeService";
      id: number;
      deviceName: string;
      deviceAddress: string;
      uuid: string;
      uuidLabel: string;
    };

/// Response payloads, mirrored from `PairResponseDto` in Rust. Each
/// `respond()` call must match the kind of the active request: e.g.
/// `pinCodeInput` expects `{ kind: "pinCode", value }`. The backend
/// rejects mismatched responses defensively.
export type PairResponse =
  | { kind: "confirm" }
  | { kind: "reject" }
  | { kind: "pinCode"; value: string }
  | { kind: "passkey"; value: number };

/// Update payload for `displayPasskey` while the user types on the
/// remote device. Backend re-emits with monotonically increasing
/// `entered` count.
type DisplayUpdate = {
  id: number;
  entered: number;
};

const inner = writable<PairRequest | null>(null);

/// Subscribe-only handle. Mutations go through the exported helpers
/// so external components can't desync the backend pending-map.
export const current: Readable<PairRequest | null> = {
  subscribe: inner.subscribe,
};

let unlistenRequest: UnlistenFn | null = null;
let unlistenUpdate: UnlistenFn | null = null;
let unlistenCancel: UnlistenFn | null = null;
let initialised = false;

// `?btmock=<kind>` (DEV only) renders one pairing state without a BlueZ
// event, so the screenshot loop can see this dialog too. A real boot never
// hits this branch.
const BT_MOCKS: Record<string, PairRequest> = {
  confirmation: { kind: "confirmation", id: 1, deviceName: "WH-1000XM5", deviceAddress: "F8:4E:17:2A:9C:41", passkey: 748291 },
  pinCodeInput: { kind: "pinCodeInput", id: 2, deviceName: "Magic Keyboard", deviceAddress: "60:F4:45:11:B0:8E" },
  passkeyInput: { kind: "passkeyInput", id: 3, deviceName: "ThinkPad Keyboard", deviceAddress: "10:3D:1C:77:52:F0" },
  displayPinCode: { kind: "displayPinCode", id: 4, deviceName: "Car Audio", deviceAddress: "00:1B:DC:0F:AA:23", pinCode: "4207" },
  displayPasskey: { kind: "displayPasskey", id: 5, deviceName: "Magic Keyboard", deviceAddress: "60:F4:45:11:B0:8E", passkey: 351904, entered: 2 },
  authorization: { kind: "authorization", id: 6, deviceName: "WH-1000XM5", deviceAddress: "F8:4E:17:2A:9C:41" },
  authorizeService: { kind: "authorizeService", id: 7, deviceName: "WH-1000XM5", deviceAddress: "F8:4E:17:2A:9C:41", uuid: "0000110b-0000-1000-8000-00805f9b34fb", uuidLabel: "Audio sink" },
};

/// Wire up the three Tauri event listeners and restore any in-
/// flight pending request from the backend. Idempotent — calling
/// twice is safe; the second call short-circuits.
export async function init(): Promise<void> {
  if (initialised) return;
  initialised = true;

  if (import.meta.env.DEV && typeof location !== "undefined") {
    const mock = BT_MOCKS[new URLSearchParams(location.search).get("btmock") ?? ""];
    if (mock) inner.set(mock);
  }

  unlistenRequest = await listen<PairRequest>(
    "bluetooth-pair-request",
    ({ payload }) => {
      inner.set(payload);
    },
  );

  unlistenUpdate = await listen<DisplayUpdate>(
    "bluetooth-pair-display-update",
    ({ payload }) => {
      inner.update((cur) => {
        if (!cur || cur.id !== payload.id || cur.kind !== "displayPasskey") {
          return cur;
        }
        return { ...cur, entered: payload.entered };
      });
    },
  );

  unlistenCancel = await listen<{ id?: number }>(
    "bluetooth-pair-cancel",
    ({ payload }) => {
      inner.update((cur) => {
        if (!cur) return cur;
        if (payload?.id !== undefined && cur.id !== payload.id) return cur;
        return null;
      });
    },
  );

  // Restore on mount — if the frontend was reloaded while the
  // backend had a pending request, re-render its dialog.
  try {
    const pending = await invoke<PairRequest[]>(
      "bluetooth_pair_pending_requests",
    );
    if (pending.length > 0) {
      inner.set(pending[0]!);
    }
  } catch (err) {
    console.warn("bluetooth_pair_pending_requests failed:", err);
  }
}

/// Tear down on app unmount. Tests use this; in production the
/// listeners live for the process lifetime.
export function dispose(): void {
  unlistenRequest?.();
  unlistenUpdate?.();
  unlistenCancel?.();
  unlistenRequest = null;
  unlistenUpdate = null;
  unlistenCancel = null;
  initialised = false;
}

/// Send a response to the active request. Pre-emptively clears the
/// store so the dialog disappears immediately, before the Tauri
/// command round-trip completes — the 60s backend timeout still
/// bounds any error case.
export async function respond(
  id: number,
  response: PairResponse,
): Promise<void> {
  inner.set(null);
  try {
    await invoke("bluetooth_pair_respond", { reqId: id, response });
  } catch (err) {
    console.warn("bluetooth_pair_respond failed:", err);
  }
}

/// Convenience: cancel/reject the active request.
export function cancel(id: number): Promise<void> {
  return respond(id, { kind: "reject" });
}
