/// The change-diff primitive: a presentational unified-diff renderer plus the
/// model + parser it consumes. Any surface showing a proposed or applied file
/// change (the harness gate/receipt, a future reviewer) composes these.

export { default as DiffView } from "./DiffView.svelte";
export {
  parseUnifiedDiff,
  diffTotals,
  type DiffFile,
  type DiffHunk,
  type DiffLine,
  type FileStatus,
} from "./diff";
