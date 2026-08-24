import { describe, expect, it } from "vitest";
import { parseQuick } from "./quickparse";

// A Monday, mid-August, mid-morning - weekday arithmetic stays legible.
const NOW = new Date(2026, 7, 24, 10, 0);

describe("parseQuick", () => {
  it("reads an English line with weekday, time and place token", () => {
    const p = parseQuick("Lunch with Anna tuesday 13:00 @Cafe X", NOW);
    expect(p.title).toBe("Lunch with Anna");
    expect(p.date).toBe("2026-08-25");
    expect(p.time).toBe("13:00");
    expect(p.location).toBe("Cafe X");
  });

  it("reads a German line with Uhr and a duration", () => {
    const p = parseQuick("Probe morgen 19 Uhr für 2h", NOW);
    expect(p.title).toBe("Probe");
    expect(p.date).toBe("2026-08-25");
    expect(p.time).toBe("19:00");
    expect(p.endTime).toBe("21:00");
  });

  it("reads a span and a calendar token", () => {
    const p = parseQuick("Review 13-14:30 /work", NOW);
    expect(p.title).toBe("Review");
    expect(p.time).toBe("13:00");
    expect(p.endTime).toBe("14:30");
    expect(p.calendar).toBe("work");
  });

  it("keeps a bare number in the title rather than inventing a time", () => {
    const p = parseQuick("Buy 3 melons", NOW);
    expect(p.title).toBe("Buy 3 melons");
    expect(p.time).toBeNull();
  });

  it("keeps mid-sentence in as language, takes trailing in as a place", () => {
    const mid = parseQuick("Check in with Mara friday", NOW);
    expect(mid.title).toBe("Check in with Mara");
    expect(mid.date).toBe("2026-08-28");
    const tail = parseQuick("Dinner friday 19:00 in Stadthalle", NOW);
    expect(tail.location).toBe("Stadthalle");
    expect(tail.title).toBe("Dinner");
  });

  it("rolls a bare past dotted date into next year", () => {
    const p = parseQuick("Party 3.1.", NOW);
    expect(p.date).toBe("2027-01-03");
  });

  it("keeps pm hours honest", () => {
    const p = parseQuick("Call 1pm-2:30pm", NOW);
    expect(p.time).toBe("13:00");
    expect(p.endTime).toBe("14:30");
  });

  it("today is today, even at the edge of the word", () => {
    const p = parseQuick("Standup heute 9:00", NOW);
    expect(p.date).toBe("2026-08-24");
    expect(p.time).toBe("09:00");
  });
});
