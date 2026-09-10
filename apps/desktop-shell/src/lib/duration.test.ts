// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Both of these decide a SHAPE, and the shape is what a person reads on the
// battery tooltip and every undo row. The boundaries are the whole content:
// nothing to say at zero, minutes until an hour, a rounded hour after that -
// and the "now" band, which exists so a row does not flicker "1m" a second
// after the thing happened.
import { describe, it, expect } from "vitest";
import { durationText, agoText } from "./duration.js";

//: A translator that reveals which key was chosen and with what numbers, since
//: that IS the decision under test - the German catalogue writes these
//: differently and this module deliberately does not know how.
const t = ((key: string, params?: Record<string, unknown>) =>
  params ? `${key}(${Object.entries(params).map(([k, v]) => `${k}=${v}`).join(",")})` : key) as never;

describe("durationText", () => {
  it("says nothing at all rather than a zero", () => {
    expect(durationText(t, 0)).toBe("");
    expect(durationText(t, null)).toBe("");
    expect(durationText(t, -5)).toBe("");
  });

  it("uses the minutes form under an hour", () => {
    expect(durationText(t, 1)).toBe("sh.dur.m(m=1)");
    expect(durationText(t, 59)).toBe("sh.dur.m(m=59)");
  });

  it("uses the hours-and-minutes form from an hour up", () => {
    expect(durationText(t, 60)).toBe("sh.dur.hm(h=1,m=0)");
    expect(durationText(t, 135)).toBe("sh.dur.hm(h=2,m=15)");
  });
});

describe("agoText", () => {
  it("calls the first ninety seconds now", () => {
    expect(agoText(t, 0)).toBe("sh.dur.now");
    expect(agoText(t, 89)).toBe("sh.dur.now");
  });

  it("counts whole minutes after that", () => {
    expect(agoText(t, 90)).toBe("sh.dur.agoM(n=1)");
    expect(agoText(t, 3599)).toBe("sh.dur.agoM(n=59)");
  });

  it("rounds down to whole hours from an hour up", () => {
    expect(agoText(t, 3600)).toBe("sh.dur.agoH(n=1)");
    expect(agoText(t, 7199)).toBe("sh.dur.agoH(n=1)");
  });

  it("treats a clock that ran backwards as now", () => {
    // A receipt stamped a moment in the future - a clock correction, a suspend -
    // must not read as "in -3 minutes".
    expect(agoText(t, -10)).toBe("sh.dur.now");
  });
});
