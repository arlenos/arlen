// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// Re-exported from the kit, which is where this predicate now lives.
///
/// This file used to hold the third copy of `readsAsInternal`, with a note
/// saying the shared home was `@arlen/ui-kit` and the move was another lane's to
/// make. The move happened on 16 September. The re-export stays so the five call
/// sites here keep importing `$lib/errors`, and so an app-level decision about
/// its own failure text still has a file to live in.
export { readsAsInternal } from "@arlen/ui-kit/errors";
