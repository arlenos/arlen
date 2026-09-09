// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// What the window says when an open does not open.
//
// The property worth pinning is the one a drive's screenshot caught: a person is
// never handed the host's own words when the host could have named the state. A
// launch that reaches the shell and is refused already had that; a launch that
// never reached it did not, and showed `launch socket i/o: No such file or
// directory (os error 2)` under a half-translated sentence.

import { describe, expect, it } from "vitest";
import { launchProblem } from "./openFailure";

describe("launchProblem", () => {
  it("names each outcome the shell can answer with", () => {
    expect(launchProblem('{"outcome":"no_handler","mime":"application/pdf"}')).toEqual({
      key: "f.open.noHandler",
      what: "application/pdf",
    });
    expect(launchProblem('{"outcome":"unknown_application","app_id":"dev.arlen.pdf"}').key).toBe(
      "f.open.notInstalled",
    );
    expect(launchProblem('{"outcome":"malformed_entry","app_id":"x"}').key).toBe(
      "f.open.packagedWrong",
    );
    expect(launchProblem('{"outcome":"did_not_start","app_id":"x"}').key).toBe(
      "f.open.didNotStart",
    );
    expect(launchProblem('{"outcome":"refused"}').key).toBe("f.open.refused");
  });

  /// Not an outcome: nothing was reached to have one. It arrives in the same
  /// shape so it can be said in the reader's language.
  it("names an unreachable shell rather than passing on an errno", () => {
    const p = launchProblem('{"outcome":"service_unavailable"}');
    expect(p).toEqual({ key: "f.open.noService", what: "" });
  });

  /// Tauri wraps a command error in its own message, so the JSON arrives with
  /// prose in front of it. Both halves have to survive that.
  it("finds the answer inside whatever Tauri wrapped it in", () => {
    expect(launchProblem('Error: {"outcome":"service_unavailable"}').key).toBe("f.open.noService");
  });

  /// The floor, and it stays: an unrecognised failure is SHOWN. Dropping it puts
  /// the window back where it was before it said anything at all.
  it("shows what it cannot name", () => {
    for (const raw of ["boom", "", '{"outcome":"something_new"}', "{not json"]) {
      const p = launchProblem(raw);
      expect(p.key).toBe("f.open.failed");
      expect(p.what).toBe(raw);
    }
  });
});
