// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// Context key for the id the palette's list wears and its input points at.
///
/// `role="combobox"` requires `aria-controls`, and bits-ui sets the role, the
/// expanded state and the autocomplete hint but not the pointer to the popup -
/// which axe reports as a critical `aria-required-attr` and which costs a
/// screen-reader user the link between the field they type in and the list that
/// answers it.
export const COMMAND_LIST_ID = Symbol("arlen-command-list-id");
