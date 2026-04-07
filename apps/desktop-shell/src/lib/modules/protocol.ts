/// postMessage protocol between Shell and sandboxed modules.
///
/// See `docs/architecture/module-system.md` (Module Isolation section).

// ---------------------------------------------------------------------------
// Shell -> Module messages
// ---------------------------------------------------------------------------

export interface ShellToModuleMessage {
  type: ShellMessageType;
  payload: unknown;
}

export type ShellMessageType =
  | "search"
  | "config"
  | "theme"
  | "action_request"
  | "destroy";

export interface SearchMessage {
  type: "search";
  payload: { query: string };
}

export interface ConfigMessage {
  type: "config";
  payload: Record<string, unknown>;
}

export interface ThemeMessage {
  type: "theme";
  payload: { cssVars: Record<string, string>; variant: "dark" | "light" };
}

// ---------------------------------------------------------------------------
// Module -> Shell messages
// ---------------------------------------------------------------------------

export interface ModuleToShellMessage {
  type: ModuleMessageType;
  moduleId: string;
  payload: unknown;
}

export type ModuleMessageType =
  | "ready"
  | "results"
  | "action"
  | "indicator_update"
  | "error";

export interface ReadyMessage {
  type: "ready";
  moduleId: string;
  payload: { version: string };
}

export interface SearchResultsMessage {
  type: "results";
  moduleId: string;
  payload: SearchResult[];
}

export interface SearchResult {
  id: string;
  title: string;
  description?: string;
  icon?: string;
  action: string;
  data?: unknown;
}

export interface ActionMessage {
  type: "action";
  moduleId: string;
  payload: { action: string; data?: unknown };
}

export interface IndicatorUpdateMessage {
  type: "indicator_update";
  moduleId: string;
  payload: { icon?: string; label?: string; badge?: string | number };
}

export interface ErrorMessage {
  type: "error";
  moduleId: string;
  payload: { message: string };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Type guard for messages from modules. */
export function isModuleMessage(data: unknown): data is ModuleToShellMessage {
  return (
    typeof data === "object" &&
    data !== null &&
    "type" in data &&
    "moduleId" in data
  );
}
