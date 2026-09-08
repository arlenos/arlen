/// Focus Mode's own settings, read from `shell.toml [focus_settings]`.
///
/// **The switch that writes this had no reader.** Settings has offered "Show
/// project name when active - pin the active project name to the top bar while
/// Focus Mode is on" since Sprint C, and the top bar pinned the name whether or
/// not it was on. A switch that changes nothing is worse than a missing one: it
/// reads as a preference the system took.
///
/// Same shape as `toastConfig`, which reads the same file the same way: one
/// command, plus a re-read on `arlen://shell-config-changed` so a change in
/// Settings lands without a restart.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// Default TRUE, matching the Settings page's own default: a person who has
/// never touched the switch sees the name, which is what they saw before it
/// worked.
export const showProjectName = writable(true);

interface ShellConfigShape {
  focus_settings?: { show_project_name?: boolean };
}

async function load(): Promise<void> {
  try {
    const cfg = await invoke<ShellConfigShape>("get_shell_config");
    showProjectName.set(cfg.focus_settings?.show_project_name ?? true);
  } catch {
    // Keep the default. An unreadable config showing the name matches what the
    // surface said before this store existed, and hiding it on a failed read
    // would be the shell deciding something nobody asked for.
  }
}

let started = false;
let teardown: (() => void) | null = null;

export function initFocusSettings(): () => void {
  if (started && teardown) return teardown;
  started = true;
  void load();
  const pending: Array<Promise<UnlistenFn>> = [
    listen("arlen://shell-config-changed", () => void load()),
  ];
  teardown = () => {
    pending.forEach((p) => p.then((fn) => fn()).catch(() => {}));
    started = false;
    teardown = null;
  };
  return teardown;
}
