/// Toast renderer configuration store.
///
/// Reads `~/.config/lunaris/shell.toml [toast]` via the `get_shell_config`
/// Tauri command and re-reads it whenever the backend emits
/// `lunaris://shell-config-changed` (which fires on any external write
/// from the Settings app).

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ToastPosition =
  | "top-right"
  | "top-left"
  | "top-center"
  | "bottom-right"
  | "bottom-left"
  | "bottom-center";

export type ToastAnimation = "slide" | "fade" | "none";

export interface ToastConfig {
  position: ToastPosition;
  width: number;
  animation: ToastAnimation;
}

export const DEFAULT_TOAST_CONFIG: ToastConfig = {
  position: "top-right",
  width: 380,
  animation: "slide",
};

interface ShellConfigShape {
  toast?: Partial<ToastConfig>;
}

export const toastConfig = writable<ToastConfig>(DEFAULT_TOAST_CONFIG);

async function load(): Promise<void> {
  try {
    const cfg = await invoke<ShellConfigShape>("get_shell_config");
    const merged: ToastConfig = {
      ...DEFAULT_TOAST_CONFIG,
      ...(cfg.toast ?? {}),
    };
    toastConfig.set(merged);
  } catch {
    // Keep defaults on error.
  }
}

let started = false;
let teardown: (() => void) | null = null;

export function initToastConfig(): () => void {
  if (started && teardown) return teardown;
  started = true;

  load();

  const unlistenPromise: Promise<UnlistenFn> = listen(
    "lunaris://shell-config-changed",
    () => {
      load();
    },
  );

  teardown = () => {
    unlistenPromise.then((fn) => fn()).catch(() => {});
    started = false;
    teardown = null;
  };
  return teardown;
}
