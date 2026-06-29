/// A generic pointer-based drag-to-reorder list (WebKitGTK-safe; not the HTML5
/// drag API). The consumer renders each item and marks its drag handle with
/// `data-sortable-handle`; `onReorder` returns the new id order on drop.
export { default as SortableList } from "./sortable-list.svelte";
