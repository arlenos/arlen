// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The mapping, checked in both directions: a refusal this window models must
// reach a message id, and one it does not must reach the reader ANYWAY. The
// second half is the one worth a test - swallowing an unrecognised answer is
// how a person ends up with a window that says nothing happened.

import { describe, expect, it } from "vitest";
import { restoreProblem, trashProblem } from "./trashProblem";

// Tauri stringifies a command error, so the JSON arrives inside a message.
const wrapped = (body: string) => `invoke error: ${body}`;

describe("trashProblem", () => {
  it("names each refusal the host models", () => {
    expect(trashProblem(wrapped('{"problem":"cross-device"}')).key).toBe("v.trash.crossDevice");
    expect(trashProblem(wrapped('{"problem":"not-found"}')).key).toBe("v.trash.notFound");
    expect(trashProblem(wrapped('{"problem":"no-slot"}')).key).toBe("v.trash.noSlot");
    expect(trashProblem(wrapped('{"problem":"unsupported"}')).key).toBe("v.trash.unsupported");
    expect(trashProblem(wrapped('{"problem":"non-canonical"}')).key).toBe("v.trash.nonCanonical");
  });

  it("keeps the detail on the two cases that carry one", () => {
    const noTrash = trashProblem(wrapped('{"problem":"no-trash-here","why":"read-only mount"}'));
    expect(noTrash).toEqual({ key: "v.trash.noTrashHere", detail: "read-only mount" });
    const io = trashProblem(wrapped('{"problem":"io","message":"Permission denied"}'));
    expect(io).toEqual({ key: "v.trash.io", detail: "Permission denied" });
  });

  it("shows an answer it does not model rather than dropping it", () => {
    const raw = "the daemon went away";
    expect(trashProblem(raw)).toEqual({ key: "v.couldNotDelete", detail: raw });
    // Valid JSON, unknown word: still the reader's problem to see.
    const odd = wrapped('{"problem":"asteroid"}');
    expect(trashProblem(odd).key).toBe("v.couldNotDelete");
    expect(trashProblem(odd).detail).toBe(odd);
  });

  it("does not choke on a half-written answer", () => {
    expect(trashProblem("{not json").key).toBe("v.couldNotDelete");
  });
});

describe("restoreProblem", () => {
  it("says the name is taken rather than printing DestinationExists", () => {
    expect(restoreProblem(wrapped('{"problem":"destination-exists"}')).key)
      .toBe("v.restore.nameTaken");
  });

  it("carries the message of an unmodelled rename failure", () => {
    const other = restoreProblem(wrapped('{"problem":"other","message":"EACCES"}'));
    expect(other).toEqual({ key: "v.couldNotRestore", detail: "EACCES" });
  });

  it("shows an unrecognised answer", () => {
    expect(restoreProblem("boom")).toEqual({ key: "v.couldNotRestore", detail: "boom" });
  });
});
