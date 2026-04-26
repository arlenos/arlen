/// Barrel export for the Tier 2 (iframe) module system frontend.

export { default as SandboxedModuleHost } from "./SandboxedModuleHost.svelte";
export type {
  HostToModule,
  ModuleToHost,
  HostCall,
  HostReply,
  SearchResult,
  SearchAction,
  Capabilities,
  ThemeTokens,
  ProjectInfo,
  ErrorCode,
} from "./postmsg.js";
export { isModuleToHost } from "./postmsg.js";
export {
  refreshFromDaemon,
  searchModules,
  installListener,
  moduleWorkers,
} from "./moduleSearchStore.js";
