/// The bulk-rename preview transform, mirroring `apps/files/core/src/bulk_rename.rs`
/// field for field (serde lowercase enums) so the dialog can show a live preview
/// without a round-trip per keystroke. The contract changes in the core crate
/// first, never here; the actual rename is applied by the backend over the same
/// core, so this is the preview/UX layer only.

/// A letter-case transform applied to the whole name.
export type CaseTransform = "lower" | "upper" | "title";

/// Sequential numbering via a template with `{n}` (the padded sequence number)
/// and `{name}` (the name after find/replace + case). A template without `{n}`
/// has the number appended, so the rows stay distinct.
export interface Numbering {
  template: string;
  start: number;
  step: number;
  pad: number;
}

/// A bulk-rename rule. Empty/absent fields are no-ops.
export interface RenameRule {
  find?: string;
  replace: string;
  find_case_insensitive: boolean;
  case?: CaseTransform;
  numbering?: Numbering;
}

/// Why a planned name cannot be applied. Precedence: invalid > duplicate >
/// unchanged > none.
export type ConflictKind = "none" | "unchanged" | "invalid" | "duplicate";

/// One planned rename: the original name, the proposed new name and any conflict.
export interface RenamePreview {
  old: string;
  new: string;
  conflict: ConflictKind;
}

/// Literal replace of every occurrence (matches Rust `str::replace`).
function replaceAll(haystack: string, find: string, replace: string): string {
  if (find === "") return haystack;
  return haystack.split(find).join(replace);
}

/// Case-insensitive literal replace of every occurrence. The match advances by
/// `find.length`; near-perfect for the preview (the backend apply is exact).
function replaceCi(haystack: string, find: string, replace: string): string {
  if (find === "") return haystack;
  const lower = find.toLowerCase();
  let out = "";
  let rest = haystack;
  while (rest.length > 0) {
    if (rest.toLowerCase().startsWith(lower)) {
      out += replace;
      rest = rest.slice(find.length);
    } else {
      out += rest[0];
      rest = rest.slice(1);
    }
  }
  return out;
}

/// Apply a case transform to a whole name.
function applyCase(name: string, kind: CaseTransform): string {
  switch (kind) {
    case "lower":
      return name.toLowerCase();
    case "upper":
      return name.toUpperCase();
    case "title":
      return name
        .split(" ")
        .map((w) => (w.length > 0 ? w[0].toUpperCase() + w.slice(1) : ""))
        .join(" ");
  }
}

/// Apply numbering to a name using its sequence number.
function applyNumbering(name: string, num: Numbering, seq: number): string {
  const number = String(seq).padStart(num.pad, "0");
  if (num.template.includes("{n}")) {
    return num.template.replaceAll("{name}", name).replaceAll("{n}", number);
  }
  // No `{n}`: append the number so the rows stay distinct.
  return num.template.replaceAll("{name}", name) + number;
}

/// Whether a produced name is a usable filename.
function isValidName(name: string): boolean {
  return (
    name.length > 0 &&
    name !== "." &&
    name !== ".." &&
    !name.includes("/") &&
    !name.includes("\0")
  );
}

/// Plan a bulk rename: apply `rule` to each name in order (the order also drives
/// the numbering sequence) and return a per-row preview with conflict detection.
export function planRename(
  names: string[],
  rule: RenameRule,
): RenamePreview[] {
  const news = names.map((name, i) => {
    let s = name;
    if (rule.find && rule.find.length > 0) {
      s = rule.find_case_insensitive
        ? replaceCi(s, rule.find, rule.replace)
        : replaceAll(s, rule.find, rule.replace);
    }
    if (rule.case) {
      s = applyCase(s, rule.case);
    }
    if (rule.numbering) {
      const seq = rule.numbering.start + i * rule.numbering.step;
      s = applyNumbering(s, rule.numbering, seq);
    }
    return s;
  });

  const counts = new Map<string, number>();
  for (const n of news) counts.set(n, (counts.get(n) ?? 0) + 1);

  return names.map((old, i) => {
    const next = news[i];
    let conflict: ConflictKind;
    if (!isValidName(next)) conflict = "invalid";
    else if ((counts.get(next) ?? 0) > 1) conflict = "duplicate";
    else if (next === old) conflict = "unchanged";
    else conflict = "none";
    return { old, new: next, conflict };
  });
}
