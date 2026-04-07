/// Barrel export for the module system frontend.

export { default as ModuleHost } from "./ModuleHost.svelte";
export type {
  ShellToModuleMessage,
  ModuleToShellMessage,
  SearchResult,
  SearchResultsMessage,
  ActionMessage,
  IndicatorUpdateMessage,
  ReadyMessage,
  ErrorMessage,
} from "./protocol.js";
export { isModuleMessage } from "./protocol.js";
