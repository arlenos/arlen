/// The keys the review actions hand to the catalogue.
///
/// `driveHunk` takes a message KEY, so a typo shows the key on screen and no
/// check sees it: `check-message-keys` reads literals inside `$t(...)`, and these
/// are literals at a call site.
import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import { t } from "$lib/i18n/messages";

describe("review failure keys", () => {
  it("every hunk action's refusal resolves to a sentence", () => {
    for (const key of ["te.review.notApplied", "te.review.notRejected", "te.review.notUndone"]) {
      const sentence = get(t)(key);
      expect(sentence).not.toBe(key);
      expect(sentence.length).toBeGreaterThan(10);
    }
  });
});
