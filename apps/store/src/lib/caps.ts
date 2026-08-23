/// Capability identifiers → plain-language rows. The backend sends identifiers
/// (`network`, `filesystem`, `read:File`), never prose - this app is translated
/// and the daemon is not. The negatives ("Cannot reach the network") carry the
/// weight of the least-privilege story, so they are derived here from what a
/// variant does NOT ask for, not shipped as data.
import type { Translate } from "@arlen/ui-kit/i18n";

/// One rendered capability row.
export interface CapRow {
  text: string;
  /// A negative row states what the app cannot do.
  negative: boolean;
}

const KNOWN = new Set(["network", "filesystem", "notifications", "clipboard", "audio", "system"]);

/// One identifier as a sentence. An identifier the catalogue does not know is
/// shown de-mangled rather than hidden - an unknown grant is still a grant.
export function capText(t: Translate, cap: string): string {
  if (KNOWN.has(cap)) return t(`st.cap.${cap}`);
  if (cap.startsWith("read:")) return t("st.cap.graphRead", { what: cap.slice(5) });
  if (cap.startsWith("write:")) return t("st.cap.graphWrite", { what: cap.slice(6) });
  return cap.replace(/[._-]/g, " ");
}

/// The full "What it can reach" panel for one capability set: every declared
/// identifier as a sentence, then the negatives the set implies. An empty set
/// is its own sentence, not an empty panel.
export function reachRows(t: Translate, caps: string[]): CapRow[] {
  const rows: CapRow[] = caps.map((c) => ({ text: capText(t, c), negative: false }));
  if (!caps.includes("network")) rows.push({ text: t("st.cap.noNetwork"), negative: true });
  if (!caps.some((c) => c.startsWith("read:") || c.startsWith("write:"))) {
    rows.push({ text: t("st.cap.noGraph"), negative: true });
  }
  if (caps.length === 0) rows.unshift({ text: t("st.cap.none"), negative: false });
  return rows;
}
