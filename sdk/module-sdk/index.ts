/// `@lunaris/module-sdk` — Tier 2 (iframe) module SDK.
///
/// Re-exports the typed postMessage protocol, the capability-gated
/// host calls, and the `createModule` entry point. Tier 1 (WASM)
/// modules compile against the Rust crate of the same name; this
/// file is for Tier 2 (TypeScript-in-iframe) modules.

export * from "./postmsg";
export { HostError, graph, network, events, bridgeInit } from "./host";
export type { ModuleConfig, ModuleHandle } from "./module";
export { createModule } from "./module";
