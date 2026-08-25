/// The keys the job actions hand to the catalogue.
///
/// `driveJob` takes a message KEY as its `failure` argument, so a typo there is
/// invisible: `check-message-keys` scans literals inside `$t(...)` calls, and
/// these are literals at a call site. The surface would show the key itself. This
/// is the guard for that, and it enumerates rather than spot-checks - a key
/// falling through would look exactly like the code working.
import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import { t } from "$lib/i18n/messages";

describe("job failure keys", () => {
  it("every action's refusal resolves to a sentence", () => {
    for (const key of ["sh.job.notCancelled", "sh.job.notPaused", "sh.job.notResumed"]) {
      const sentence = get(t)(key);
      expect(sentence).not.toBe(key);
      expect(sentence.length).toBeGreaterThan(10);
    }
  });
});
