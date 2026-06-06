import { writable } from "svelte/store";

export type NetworkState = "wifi" | "ethernet" | "disconnected";
export type VolumeState = "muted" | "low" | "medium" | "high";
export interface BatteryState {
    charging: boolean;
    /** 0-100 */
    level: number;
}

// Stub values. Wire to D-Bus / event-bus integration in Phase 4.
export const networkState = writable<NetworkState>("wifi");
export const volumeState = writable<VolumeState>("medium");
export const batteryState = writable<BatteryState>({ charging: false, level: 72 });

// notificationCount removed - use unreadCount from notifications.ts instead.
