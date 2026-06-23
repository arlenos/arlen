// Arlen's thin pi extensions (pi-agent-adoption.md). The package's default
// export is the Arlen extension factory (gate + audit shims), so pi can load
// this package directly; the contract client and individual shim factories are
// also exported for tests and advanced use.
export * from "./contract.js";
export { makeGate } from "./gate.js";
export { makeAudit } from "./audit.js";
export { installArlenShims, type ArlenExtensionAPI } from "./extension.js";
export { default } from "./extension.js";
