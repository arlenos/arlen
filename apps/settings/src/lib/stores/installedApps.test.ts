import { describe, expect, it } from "vitest";
import { mergeAppRows, type AppRow, type GrantedPrincipal } from "./installedApps";

const row = (app_id: string, name: string): AppRow => ({ app_id, name, version: null, publisher: null });
const grant = (appId: string, label: string, identityVerified = true): GrantedPrincipal => ({
  appId,
  label,
  identityVerified,
});

describe("the installed-apps row source", () => {
  it("keeps an installed app that holds no grant", () => {
    // The defect this replaced: the list was the grant ledger, so an app with a
    // settings page and no grant had no way in at all.
    const out = mergeAppRows([row("dev.arlen.clock", "Clock")], []);
    expect(out.map((a) => a.appId)).toEqual(["dev.arlen.clock"]);
    expect(out[0].source).toBe("installed");
  });

  it("keeps a granted app that ships no desktop entry", () => {
    // The mirror risk of the fix: a straight swap would have removed a row that
    // is reachable today.
    const out = mergeAppRows([], [grant("dev.arlen.bridge", "Bridge")]);
    expect(out.map((a) => a.appId)).toEqual(["dev.arlen.bridge"]);
    expect(out[0].source).toBe("granted");
  });

  it("lists an app once when both know it", () => {
    const out = mergeAppRows([row("dev.arlen.mail", "Mail")], [grant("dev.arlen.mail", "mail")]);
    expect(out).toHaveLength(1);
    expect(out[0].source).toBe("both");
  });

  it("prefers the name the app gives itself over the one derived from its id", () => {
    const out = mergeAppRows([row("dev.arlen.text-editor", "Text Editor")], [grant("dev.arlen.text-editor", "text-editor")]);
    expect(out[0].label).toBe("Text Editor");
  });

  it("carries an unverified identity through from the grant", () => {
    const out = mergeAppRows([row("x.y", "Y")], [grant("x.y", "Y", false)]);
    expect(out[0].identityVerified).toBe(false);
  });

  it("does not call an ungranted app unverified", () => {
    // Nothing has claimed anything about it. That is not the same as a claim
    // that failed, and the row must not carry the warning that says it is.
    const out = mergeAppRows([row("x.y", "Y")], []);
    expect(out[0].identityVerified).toBe(true);
  });

  it("orders by the label a reader sees, not by id", () => {
    const out = mergeAppRows([row("z.id", "Alpha"), row("a.id", "Zulu")], []);
    expect(out.map((a) => a.label)).toEqual(["Alpha", "Zulu"]);
  });

  it("is empty when both sources are", () => {
    expect(mergeAppRows([], [])).toEqual([]);
  });
});
