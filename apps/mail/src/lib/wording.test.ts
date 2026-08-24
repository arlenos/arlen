// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The two rules this window applies to what it was handed. Both used to live
// inside the component, where the only way to exercise them was to render it.

import { describe, expect, it } from "vitest";
import { formatSent, invitationWords, threadKey } from "./wording";

// Returns the id it was asked for, so a test can assert WHICH sentence was
// chosen without depending on any wording.
const echo = (key: string, values?: Record<string, unknown>) =>
  values ? `${key}:${JSON.stringify(values)}` : key;

describe("invitationWords", () => {
  it("gives each method its own sentence", () => {
    expect(invitationWords("request", echo)).toBe("ml.invitation.request");
    expect(invitationWords("cancel", echo)).toBe("ml.invitation.cancel");
    expect(invitationWords("reply", echo)).toBe("ml.invitation.reply");
    expect(invitationWords("publish", echo)).toBe("ml.invitation.publish");
  });

  it("says a part with no method is a calendar part and nothing more", () => {
    expect(invitationWords(null, echo)).toBe("ml.invitation.unmarked");
  });

  it("shows a method nobody has heard of, as written", () => {
    expect(invitationWords("counter", echo)).toBe(
      'ml.invitation.other:{"method":"counter"}',
    );
  });
});

describe("formatSent", () => {
  it("writes the date the way the reader's language writes one", () => {
    const out = formatSent("Fri, 21 Aug 2026 09:15:00 +0000", "en-GB");
    expect(out).toContain("2026");
    expect(out).not.toContain("Fri, 21 Aug 2026 09:15:00 +0000");
  });

  it("is the same instant in two languages, said differently", () => {
    const en = formatSent("Fri, 21 Aug 2026 09:15:00 +0000", "en-GB");
    const de = formatSent("Fri, 21 Aug 2026 09:15:00 +0000", "de-DE");
    expect(en).not.toBe(de);
    expect(de).toContain("2026");
  });

  it("returns a header it cannot read VERBATIM rather than inventing one", () => {
    // The raw line is what the sender wrote, and it is the only thing anybody
    // has for a malformed date. An empty field or today's date are both worse.
    expect(formatSent("sometime next week", "en-GB")).toBe("sometime next week");
  });
});

describe("threadKey", () => {
  it("folds reply and forward prefixes into one conversation", () => {
    expect(threadKey("Re: Re: AW: Plans")).toBe("plans");
    expect(threadKey("Fwd: plans")).toBe("plans");
    expect(threadKey("Plans")).toBe("plans");
  });
  it("keeps a prefix-looking word inside the subject", () => {
    expect(threadKey("Rethinking the plan")).toBe("rethinking the plan");
  });
  it("names the empty subject rather than keying everything together", () => {
    expect(threadKey(null)).toBe("(no subject)");
    expect(threadKey("Re:")).toBe("(no subject)");
  });
});
