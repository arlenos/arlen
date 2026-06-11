/// The Console archetype primitives (design-system.md §5.3): the
/// command block and the reserved cell-grid render area. Net-new for
/// the console the way the chat primitives were for the harness;
/// consumed by apps/terminal and, later, embedded hosts like the file
/// manager's terminal pane.
export { default as ConsoleBlock } from "./ConsoleBlock.svelte";
export { default as GridRegion } from "./GridRegion.svelte";
