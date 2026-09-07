import { describe, expect, it } from "vitest";
import { byApp, hasExpired, type GrantView } from "./grants";

/// The App-access page renders through `byApp`, so these run the same path the
/// three surfaces do (/privacy both pivots, /apps, /apps/[id]).
const t = ((k: string) => k) as unknown as Parameters<typeof byApp>[0];

/// A consent grant reaching one folder, which is the shape the consent broker
/// mints with a lifetime.
const grant = (over: Partial<GrantView> = {}): GrantView => ({
  id: "0192-0001",
  app_id: "dev.arlen.notes",
  declared_ceiling: "",
  required: false,
  identity_verified: true,
  live: true,
  revoked: false,
  superseded: false,
  issued_at: 1_780_000_000_000_000,
  expires_at: 0,
  reach: [],
  source: "consent",
  consent_class: "files.read",
  consent_scope: "/home/u/Documents",
  ...over,
});

const NOW = 1_790_000_000_000_000;

describe("the expiry of a time-boxed grant", () => {
  it("treats a zero expiry as lasting until revoked", () => {
    // The stored "no expiry" is 0, not an expiry at the epoch, so a grant that
    // never expires must not read as long over.
    expect(hasExpired(grant({ expires_at: 0 }), NOW)).toBe(false);
  });

  it("reads a window that has closed as over", () => {
    expect(hasExpired(grant({ expires_at: NOW - 1 }), NOW)).toBe(true);
  });

  it("reads the closing instant itself as over", () => {
    // Matches the daemon's own `expires_at <= now`, so the two sides agree on
    // the boundary rather than one of them granting an extra microsecond.
    expect(hasExpired(grant({ expires_at: NOW }), NOW)).toBe(true);
  });

  it("keeps a window that is still open", () => {
    expect(hasExpired(grant({ expires_at: NOW + 1 }), NOW)).toBe(false);
  });
});

describe("the App-access page's active-grant filter", () => {
  it("drops an app whose only grant has expired", () => {
    // The defect: the page filtered on revoked and superseded alone, so the
    // five minutes you allowed a folder for still rendered as current reach
    // long after they had passed.
    const out = byApp(t, "en", [grant({ expires_at: 1 })]);
    expect(out.map((p) => p.appId)).toEqual([]);
  });

  it("keeps a grant that lasts until revoked", () => {
    const out = byApp(t, "en", [grant()]);
    expect(out.map((p) => p.appId)).toEqual(["dev.arlen.notes"]);
  });

  it("keeps a dormant capability token whose process has exited", () => {
    // The mirror risk: `live` also goes false when the minting process dies.
    // Filtering on it would say an installed app reaches nothing, when it
    // reaches its declared scope again the moment it starts.
    const out = byApp(t, "en", [
      grant({
        source: "capability-token",
        consent_class: "",
        consent_scope: "",
        live: false,
        declared_ceiling: JSON.stringify({
          read: [{ entity_type: "system.File", fields: null, exclude_fields: [] }],
          write: [],
          relations: [],
          instance: "All",
        }),
      }),
    ]);
    expect(out.map((p) => p.appId)).toEqual(["dev.arlen.notes"]);
  });
});
