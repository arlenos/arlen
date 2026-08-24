/// Natural-language quick entry, the Fantastical pattern: as the title is
/// typed, recognised pieces (a weekday, a time, a span, a duration, a
/// /calendar token, an @place) assemble the event live while the words stay
/// visible. CONSERVATIVE by design - anything not certainly a token stays
/// title text, because eating a word someone meant is worse than leaving one.
///
/// English and German, matching the app's two catalogues. Pure and tested;
/// the form consumes the result and keeps every field editable.

export interface QuickParse {
  /// The title with consumed tokens removed (folded whitespace).
  title: string;
  /// YYYY-MM-DD when a date was recognised.
  date: string | null;
  /// HH:MM when a start time was recognised.
  time: string | null;
  /// HH:MM when an end followed (a span or a duration).
  endTime: string | null;
  /// A location after "@" or a trailing "in <place>".
  location: string | null;
  /// A calendar id after "/", verbatim (the caller matches it to its list).
  calendar: string | null;
}

const WEEKDAYS: Record<string, number> = {
  monday: 1, tuesday: 2, wednesday: 3, thursday: 4, friday: 5, saturday: 6, sunday: 0,
  montag: 1, dienstag: 2, mittwoch: 3, donnerstag: 4, freitag: 5, samstag: 6, sonntag: 0,
  mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, sat: 6, sun: 0,
  mo: 1, di: 2, mi: 3, don: 4, fr: 5, sa: 6, so: 0,
};

function ymdOf(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/// The next occurrence of a weekday, today included.
function nextWeekday(from: Date, dow: number): string {
  const d = new Date(from);
  const shift = (dow - d.getDay() + 7) % 7;
  d.setDate(d.getDate() + shift);
  return ymdOf(d);
}

function two(n: number): string {
  return String(n).padStart(2, "0");
}

/// Parse one quick-entry line. `now` is a parameter so midnight and month
/// turns are testable without waiting for them.
export function parseQuick(input: string, now: Date): QuickParse {
  let text = ` ${input} `;
  const out: QuickParse = { title: "", date: null, time: null, endTime: null, location: null, calendar: null };

  // /calendar token: one word after a slash.
  text = text.replace(/\s\/([\p{L}\p{N}_-]+)/u, (_, cal: string) => {
    out.calendar = cal.toLowerCase();
    return " ";
  });

  // @place, to end of line or the next recognised token start.
  text = text.replace(/\s@([^/@]+?)(?=\s*$)/u, (_, loc: string) => {
    out.location = loc.trim();
    return " ";
  });

  // A time span "13-14:30" / "13:00-14:30" / "1pm-2:30pm".
  text = text.replace(
    /\s(\d{1,2})(?::(\d{2}))?\s*(am|pm)?\s*[-–]\s*(\d{1,2})(?::(\d{2}))?\s*(am|pm)?(?=[\s.,]|$)/i,
    (_, h1: string, m1: string | undefined, ap1: string | undefined, h2: string, m2: string | undefined, ap2: string | undefined) => {
      let a = Number(h1);
      let b = Number(h2);
      if (ap1?.toLowerCase() === "pm" && a < 12) a += 12;
      if (ap2?.toLowerCase() === "pm" && b < 12) b += 12;
      if (a > 23 || b > 23) return _;
      out.time = `${two(a)}:${m1 ?? "00"}`;
      out.endTime = `${two(b)}:${m2 ?? "00"}`;
      return " ";
    },
  );

  // A single time "13:00" / "9:30" / "1pm" / "13 Uhr". A bare hour without a
  // marker stays a word - "3" in a title is not an appointment.
  if (!out.time) {
    text = text.replace(
      /\s(?:um\s+|at\s+)?(\d{1,2})(?::(\d{2}))\s*(am|pm)?(?=[\s.,]|$)|\s(?:um\s+|at\s+)?(\d{1,2})\s*(uhr|am|pm)(?=[\s.,]|$)/i,
      (whole, h1: string | undefined, m1: string | undefined, ap1: string | undefined, h2: string | undefined, marker: string | undefined) => {
        let h = Number(h1 ?? h2);
        const mk = (ap1 ?? marker ?? "").toLowerCase();
        if (mk === "pm" && h < 12) h += 12;
        if (h > 23) return whole;
        out.time = `${two(h)}:${m1 ?? "00"}`;
        return " ";
      },
    );
  }

  // A duration "for 2h" / "für 90min" / "for 1.5h" - needs a start time.
  text = text.replace(
    /\s(?:for|für)\s+(\d+(?:[.,]\d+)?)\s*(h|std|stunden?|m|min|minuten?)(?=[\s.,]|$)/i,
    (whole, num: string, unit: string) => {
      if (!out.time) return whole;
      const n = Number(num.replace(",", "."));
      const minutes = /^h|^std/i.test(unit) ? Math.round(n * 60) : Math.round(n);
      const [sh, sm] = out.time.split(":").map(Number);
      const end = Math.min(24 * 60, sh * 60 + sm + minutes);
      out.endTime = `${two(Math.floor(end / 60))}:${two(end % 60)}`;
      return " ";
    },
  );

  // Relative days and weekdays.
  text = text.replace(/\s(today|heute)(?=[\s.,]|$)/i, () => {
    out.date = ymdOf(now);
    return " ";
  });
  text = text.replace(/\s(tomorrow|morgen)(?=[\s.,]|$)/i, () => {
    const d = new Date(now);
    d.setDate(d.getDate() + 1);
    out.date = ymdOf(d);
    return " ";
  });
  if (!out.date) {
    text = text.replace(/\s(?:am\s+|on\s+)?(\p{L}+)(?=[\s.,]|$)/giu, (whole, word: string) => {
      if (out.date) return whole;
      const dow = WEEKDAYS[word.toLowerCase()];
      if (dow === undefined) return whole;
      out.date = nextWeekday(now, dow);
      return " ";
    });
  }

  // A numeric date "24.12." / "24.12.2026" / "2026-12-24".
  if (!out.date) {
    text = text.replace(/\s(\d{4})-(\d{2})-(\d{2})(?=[\s.,]|$)/, (_, y: string, m: string, d: string) => {
      out.date = `${y}-${m}-${d}`;
      return " ";
    });
  }
  if (!out.date) {
    text = text.replace(/\s(\d{1,2})\.(\d{1,2})\.(\d{4})?(?=[\s.,]|$)/, (_, d: string, m: string, y: string | undefined) => {
      const year = y ? Number(y) : now.getFullYear();
      const guess = new Date(year, Number(m) - 1, Number(d));
      // A date-looking token in the past (no year given) means next year.
      if (!y && guess < now && guess.toDateString() !== now.toDateString()) guess.setFullYear(year + 1);
      out.date = ymdOf(guess);
      return " ";
    });
  }

  // Trailing "in <place>" only when a time made this an appointment, the
  // candidate starts like a name (capital or digit), and nothing else claimed
  // a location - "check in with Mara" is language, not an address.
  if (!out.location && out.time) {
    text = text.replace(/\s(?:in|im)\s+([\p{Lu}\p{N}][\p{L}\p{N} .'-]{1,}?)\s*$/u, (_, loc: string) => {
      out.location = loc.trim();
      return " ";
    });
  }

  out.title = text.replace(/\s+/g, " ").trim();
  return out;
}
