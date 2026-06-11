/// Cross-component UI signals of the FM shell.

import { writable } from "svelte/store";

/// True while the breadcrumb shows the editable path field (Ctrl+L).
export const pathEditing = writable(false);
