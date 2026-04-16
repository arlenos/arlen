/// In-app search index and JSON export for Waypointer.
///
/// The index is built once from `SETTINGS_REGISTRY` at app start and
/// rebuilt whenever a new panel is registered (future: dynamic modules).
/// The exported JSON file lives at
///   `~/.local/share/lunaris/settings-index.json`
/// and is read by Waypointer at query time without having to start the
/// Settings app.

import { SETTINGS_REGISTRY, type SettingDefinition } from "./settings-registry";
import { invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

export interface SearchResult {
  setting: SettingDefinition;
  score: number;
}

/// Case-insensitive, all-terms-must-match search over the registry.
/// Scoring: title match +10, section +5, description +3, keyword +2.
export function search(query: string, limit = 10): SearchResult[] {
  const terms = query
    .toLowerCase()
    .split(/\s+/)
    .filter((t) => t.length > 0);
  if (terms.length === 0) return [];

  const results: SearchResult[] = [];

  for (const setting of SETTINGS_REGISTRY) {
    const titleLower = setting.title.toLowerCase();
    const sectionLower = setting.section.toLowerCase();
    const descLower = setting.description.toLowerCase();
    const haystack = [
      titleLower,
      sectionLower,
      descLower,
      ...setting.keywords,
    ].join(" ");

    if (!terms.every((t) => haystack.includes(t))) continue;

    let score = 0;
    for (const term of terms) {
      if (titleLower.includes(term)) score += 10;
      if (sectionLower.includes(term)) score += 5;
      if (descLower.includes(term)) score += 3;
      if (setting.keywords.some((k) => k.includes(term))) score += 2;
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
  title: string;
  description: string;
  keywords: string[];
  panel: string;
  section: string;
  deepLink: string;
  inlineAction?: {
    type: string;
    configFile: string;
    configKey: string;
    options?: { value: string; label: string }[];
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
  };
}

interface SettingsIndex {
  version: number;
  generatedAt: string;
  settings: ExportedSetting[];
}

function buildExportPayload(): SettingsIndex {
  return {
    version: 1,
    generatedAt: new Date().toISOString(),
    settings: SETTINGS_REGISTRY.map((s) => ({
      id: s.id,
      title: s.title,
      description: s.description,
      keywords: s.keywords,
      panel: s.panel,
      section: s.section,
      deepLink: `lunaris-settings://${s.panel}#${s.anchor}`,
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

/// Write the settings index to disk via Tauri command. Called once at
/// app startup so Waypointer always has an up-to-date copy.
export async function exportSettingsIndex(): Promise<void> {
  const payload = buildExportPayload();
  try {
    await invoke("export_settings_index", {
      json: JSON.stringify(payload, null, 2),
    });
  } catch (e) {
    console.error("[search] failed to export settings index:", e);
  }
}
