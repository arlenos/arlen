/**
 * Project system store: tracks detected projects and Focus Mode state.
 *
 * Subscribes to Tauri events from the projects module and provides
 * reactive state for the TopBar project indicator and Waypointer scoping.
 */

import { writable, derived, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types ────────────────────────────────────────────────────────────────

export interface Project {
  id: string;
  name: string;
  description: string | null;
  rootPath: string;
  accentColor: string | null;
  icon: string | null;
  status: "active" | "archived";
  createdAt: number;
  lastAccessed: number | null;
  inferred: boolean;
  confidence: number;
  promoted: boolean;
}

export interface FocusState {
  projectId: string | null;
  projectName: string | null;
  rootPath: string | null;
  accentColor: string | null;
  activatedAt: number | null;
}

const EMPTY_FOCUS: FocusState = {
  projectId: null,
  projectName: null,
  rootPath: null,
  accentColor: null,
  activatedAt: null,
};

// ── Stores ───────────────────────────────────────────────────────────────

/** All known projects. */
export const projects = writable<Project[]>([]);

/** Current Focus Mode state. */
export const focusState = writable<FocusState>({ ...EMPTY_FOCUS });

/** Whether Focus Mode is active. */
export const isFocused = derived(focusState, ($f) => $f.projectId !== null);

/** Active (non-archived) projects. */
export const activeProjects = derived(projects, ($p) =>
  $p.filter((p) => p.status === "active")
);

/** Promoted projects (visible in Waypointer / Focus picker). */
export const promotedProjects = derived(projects, ($p) =>
  $p.filter((p) => p.promoted && p.status === "active")
);

/** The currently focused project (full object), or null. */
export const focusedProject = derived(
  [focusState, projects],
  ([$f, $p]) => {
    if (!$f.projectId) return null;
    return $p.find((p) => p.id === $f.projectId) ?? null;
  }
);

// ── Actions ──────────────────────────────────────────────────────────────

export async function loadProjects(): Promise<void> {
  try {
    const list = await invoke<Project[]>("list_projects");
    console.log("[projects] loaded:", list.length, "projects", list.map(p => `${p.name}(promoted=${p.promoted})`));
    projects.set(list);
  } catch (e) {
    console.error("[projects] load failed:", e);
  }
}

export async function activateFocus(project: Project): Promise<boolean> {
  // Optimistic: set state immediately so UI reacts before invoke returns.
  focusState.set({
    projectId: project.id,
    projectName: project.name,
    rootPath: project.rootPath,
    accentColor: project.accentColor ?? null,
    activatedAt: Date.now(),
  });
  applyFocusAccent(project.accentColor ?? null);
  try {
    await invoke("activate_focus", {
      projectId: project.id,
      projectName: project.name,
      rootPath: project.rootPath,
      accentColor: project.accentColor ?? null,
    });
    return true;
  } catch (e) {
    // Rollback on failure.
    focusState.set({ ...EMPTY_FOCUS });
    removeFocusAccent();
    console.error("[projects] activate focus failed:", e);
    return false;
  }
}

export async function deactivateFocus(): Promise<boolean> {
  focusState.set({ ...EMPTY_FOCUS });
  removeFocusAccent();
  try {
    await invoke("deactivate_focus");
    return true;
  } catch (e) {
    console.error("[projects] deactivate focus failed:", e);
    return false;
  }
}

// ── Accent Color ─────────────────────────────────────────────────────────

function applyFocusAccent(color: string | null): void {
  if (!color) return;
  document.documentElement.style.setProperty("--color-accent", color);
}

function removeFocusAccent(): void {
  document.documentElement.style.removeProperty("--color-accent");
}

// ── Initialization ───────────────────────────────────────────────────────

let projectsStarted = false;
let projectsTeardown: (() => void) | null = null;

/** Initialize stores and event listeners. Call once from +layout.svelte.
 *  Returns a disposer that removes all 5 listeners. Idempotent. */
export function initProjects(): () => void {
  if (projectsStarted && projectsTeardown) return projectsTeardown;
  projectsStarted = true;

  loadProjects();

  // Try restoring persisted focus state.
  invoke<FocusState | null>("get_focus_state")
    .then((state) => {
      if (state?.projectId) {
        focusState.set(state);
        applyFocusAccent(state.accentColor);
      }
    })
    .catch(() => {});

  const pending: Promise<UnlistenFn>[] = [
    // Project lifecycle events.
    listen<Project>("project:created", ({ payload }) => {
      console.log("[projects] created event:", payload.name, "promoted:", payload.promoted);
      projects.update((list) => [...list, payload]);
    }),
    listen<Project>("project:updated", ({ payload }) => {
      projects.update((list) =>
        list.map((p) => (p.id === payload.id ? payload : p))
      );
    }),
    listen<{ projectId: string }>("project:archived", ({ payload }) => {
      projects.update((list) =>
        list.map((p) =>
          p.id === payload.projectId
            ? { ...p, status: "archived" as const }
            : p
        )
      );
    }),

    // Focus Mode events.
    listen<FocusState>("focus:activated", ({ payload }) => {
      console.log("[projects] focus:activated event:", payload);
      focusState.set(payload);
      applyFocusAccent(payload.accentColor);
    }),
    listen("focus:deactivated", () => {
      focusState.set({ ...EMPTY_FOCUS });
      removeFocusAccent();
    }),
  ];

  projectsTeardown = () => {
    pending.forEach((p) => p.then((fn) => fn()).catch(() => {}));
    projectsStarted = false;
    projectsTeardown = null;
  };
  return projectsTeardown;
}
