// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The same three properties the viewer's twin holds: the modelled words reach a
// message id, the detail survives where it is the useful part, and an answer
// this does not model still reaches the reader.

import { describe, expect, it } from "vitest";
import { printProblem } from "./printProblem";

const wrapped = (body: string) => `invoke error: ${body}`;

describe("printProblem", () => {
  it("says a machine has no printing rather than showing a bus error", () => {
    expect(printProblem(wrapped('{"problem":"no-portal","message":"no such name"}'))).toEqual({
      key: "te.print.noPortal",
      detail: "",
    });
  });

  it("keeps the detail when the machine can print and this attempt did not", () => {
    expect(printProblem(wrapped('{"problem":"portal-refused","message":"queue full"}'))).toEqual({
      key: "te.print.failed",
      detail: "queue full",
    });
  });

  it("shows an answer it does not model", () => {
    expect(printProblem("the plugin is missing")).toEqual({
      key: "te.print.failed",
      detail: "the plugin is missing",
    });
  });
});
