/// In-app search index and JSON export for Waypointer.
///
/// The index is built once from `SETTINGS_REGISTRY` at app start and
/// rebuilt whenever a new panel is registered (future: dynamic modules).
/// The exported JSON file lives at
///   `~/.local/share/arlen/settings-index.json`
/// and is read by Waypointer at query time without having to start the
/// Settings app.

import { get } from "svelte/store";

import { SETTINGS_REGISTRY, type SettingDefinition } from "./settings-registry";
import { invoke } from "@tauri-apps/api/core";
import { CATALOGS, SOURCE_LOCALE, t } from "$lib/i18n/messages";

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

export interface SearchResult {
  setting: SettingDefinition;
  score: number;
}

/// The source-language text for a message, whatever the UI is showing.
///
/// Somebody who learned a setting as "night light" in English goes looking for it
/// by that name in a German UI, so a query has to reach the setting through either
/// language. Only the id form can do this: a prose snapshot has already thrown the
/// other string away.
function sourceText(id: string): string {
  // A direct catalog read, not the translator: the translator is bound to the
  // live locale and would hand back whatever the UI is currently showing, which
  // is the one thing this must not do.
  return CATALOGS[SOURCE_LOCALE]?.[id] ?? "";
}

/// Case-insensitive, all-terms-must-match search over the registry.
/// Scoring: title match +10, section +5, description +3, keyword +2.
export function search(query: string, limit = 10): SearchResult[] {
  const terms = query
    .toLowerCase()
    .split(/\s+/)
    .filter((t) => t.length > 0);
  if (terms.length === 0) return [];

  const tr = get(t);
  const results: SearchResult[] = [];

  for (const setting of SETTINGS_REGISTRY) {
    const titleLower = tr(setting.titleKey).toLowerCase();
    const sectionLower = tr(setting.sectionKey).toLowerCase();
    const descLower = tr(setting.descKey).toLowerCase();
    const keywords = tr(setting.keywordsKey).toLowerCase();
    // Both languages, so an English name finds a German setting and the reverse.
    const source = [setting.titleKey, setting.sectionKey, setting.descKey, setting.keywordsKey]
      .map((id) => sourceText(id).toLowerCase())
      .join(" ");
    const haystack = [titleLower, sectionLower, descLower, keywords, source].join(" ");

    if (!terms.every((t) => haystack.includes(t))) continue;

    let score = 0;
    for (const term of terms) {
      if (titleLower.includes(term)) score += 10;
      if (sectionLower.includes(term)) score += 5;
      if (descLower.includes(term)) score += 3;
      if (keywords.includes(term)) score += 2;
    }
    results.push({ setting, score });
  }

  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}

// ---------------------------------------------------------------------------
// Export to JSON
// ---------------------------------------------------------------------------

interface ExportedSetting {
  id: string;
  titleKey: string;
  descKey: string;
  keywordsKey: string;
  panel: string;
  sectionKey: string;
  deepLink: string;
  inlineAction?: {
    type: string;
    configFile: string;
    configKey: string;
    options?: { value: string; labelKey: string }[];
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
  };
}

interface SettingsIndex {
  version: number;
  generatedAt: string;
  /// The catalog the ids below resolve against.
  catalog: string;
  settings: ExportedSetting[];
}

/// A message the index can carry: plain text, no inputs.
///
/// The reader substring-matches these, and a selector is a pattern rather than a
/// string - `.match $n one {{...}} * {{...}}` has no single text to search. Titles
/// and sections have no placeholders today, so requiring it now costs nothing and
/// stops the first one arriving unnoticed and being indexed as its own source.
function assertPlain(id: string): void {
  const src = CATALOGS[SOURCE_LOCALE]?.[id];
  if (src === undefined) throw new Error(`settings index: no message for ${id}`);
  if (src.includes("{$") || src.includes(".match") || src.includes(".input")) {
    throw new Error(
      `settings index: ${id} takes inputs, so it cannot be searched as text. ` +
        `Split the indexed part into a plain message.`,
    );
  }
}

function buildExportPayload(): SettingsIndex {
  for (const s of SETTINGS_REGISTRY) {
    for (const id of [s.titleKey, s.descKey, s.keywordsKey, s.sectionKey]) assertPlain(id);
  }
  return {
    version: 2,
    generatedAt: new Date().toISOString(),
    /// Which catalog resolves the ids below. A reader loads it for its own locale.
    catalog: "settings",
    settings: SETTINGS_REGISTRY.map((s) => ({
      id: s.id,
      // Ids, never prose: the reader resolves them against `catalog` in its own
      // locale. See `SettingDefinition` for why a snapshot is the wrong shape.
      titleKey: s.titleKey,
      descKey: s.descKey,
      keywordsKey: s.keywordsKey,
      panel: s.panel,
      sectionKey: s.sectionKey,
      deepLink: `arlen-settings://${s.panel}#${s.anchor}`,
      inlineAction: s.inlineAction
        ? {
            type: s.inlineAction.type,
            configFile: s.inlineAction.configFile,
            configKey: s.inlineAction.configKey,
            options: s.inlineAction.options,
            min: s.inlineAction.min,
            max: s.inlineAction.max,
            step: s.inlineAction.step,
            unit: s.inlineAction.unit,
          }
        : undefined,
    })),
  };
}

/// The catalogs the index's ids resolve against, one per locale we hold.
///
/// Shipped with the index because the index carries ids, so a reader that has
/// only the index has nothing to show. Emitted verbatim from the same module the
/// Settings UI itself resolves against, so the two cannot say different things -
/// unlike a snapshot of resolved prose, which is a second rendering of the same
/// strings and drifts the moment one side is edited.
function catalogPayload(): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {};
  for (const [locale, messages] of Object.entries(CATALOGS)) {
    out[locale] = { ...messages };
  }
  return out;
}

/// Write the settings index to disk via Tauri command. Called once at
/// app startup so Waypointer always has an up-to-date copy.
export async function exportSettingsIndex(): Promise<void> {
  const payload = buildExportPayload();
  try {
    // One call, so the index and the catalogs it points at are written in one
    // pass. The Rust side writes catalogs first and the index last, so a reader
    // that sees a new index can always resolve it.
    await invoke("export_settings_index", {
      json: JSON.stringify(payload, null, 2),
      catalogs: JSON.stringify(catalogPayload()),
    });
  } catch (e) {
    console.error("[search] failed to export settings index:", e);
  }
}
