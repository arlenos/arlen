// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The relation a graph step carries decides which sentence the window writes,
// and until 26 August the backend sent prose ("Last opened by") where this file
// compares against a token ("lastOpenedBy"). Every graph step therefore fell
// through to the part-of wording: right actor, wrong relation, and fluent in
// both languages, which is the kind of wrong nobody reports as a bug.

import { describe, expect, it } from "vitest";

import { stepLine, type ProvenanceStep } from "./provenance";

// Returns the key, so a test asserts WHICH sentence was chosen rather than how
// it happens to be worded today.
const t = (id: string) => id;

function graphStep(relation?: string): ProvenanceStep {
    return {
        origin: "graph",
        actor: "Atlas",
        when_ms: 1_700_000_000_000,
        fidelity: "resolved",
        relation: relation as ProvenanceStep["relation"],
    };
}

describe("stepLine", () => {
    it("words a last-open step as a last-open, not a membership", () => {
        expect(stepLine(t, graphStep("lastOpenedBy"), "en")).toBe("f.prov.lastOpenedBy");
    });

    it("words a co-tenant step as its own sentence", () => {
        expect(stepLine(t, graphStep("alsoOpenedBy"), "en")).toBe("f.prov.alsoOpenedBy");
    });

    it("words a membership step as a membership", () => {
        expect(stepLine(t, graphStep("partOf"), "en")).toBe("f.prov.partOf");
    });

    it("falls to the membership wording for a relation it does not know", () => {
        // Conservative rather than silent: an unrecognised relation still gets a
        // sentence, and the one that claims the least.
        expect(stepLine(t, graphStep("Last opened by"), "en")).toBe("f.prov.partOf");
    });
});
