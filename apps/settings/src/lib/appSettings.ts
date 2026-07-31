/// TypeScript mirrors of the per-app settings wire shapes
/// (`contracts/forage-recipe/src/settings.rs` + the Tauri command payloads in
/// `src-tauri/src/commands/app_settings.rs`). Field names match serde exactly:
/// the schema is snake_case with `type` renamed, the page/answer are camelCase.

/// The value type of one setting (closed on purpose; irregular values use `raw`,
/// un-describable surfaces use `handoff`).
export type SettingType =
  | "bool"
  | "int"
  | "float"
  | "string"
  | "enum"
  | "string_list"
  | "path"
  | "color"
  | "keybind"
  | "duration"
  | "secret_ref"
  | "handoff"
  | "raw";

/// Which layer a key may legally be written to (the broker enforces it).
export type SettingScope = "user" | "machine" | "defaults_only";

/// One option of an `enum` setting; the description is mandatory by contract.
export interface SettingOption {
  value: string;
  label: string;
  description: string;
}

/// Where an enum's choices come from when the package cannot know them.
export type ValueSource = "audio_outputs" | "audio_inputs" | "installed_themes" | "locales" | "browsers";

/// Conditional visibility over another key of the SAME app.
export interface VisibleWhen {
  key: string;
  equals?: string | null;
  in_?: string[] | null;
}

/// A handoff row's target window name (never a command).
export interface HandoffTarget {
  window: string;
}

/// One declared setting.
export interface SettingsItem {
  key: string;
  type: SettingType;
  label: string;
  description?: string | null;
  default?: unknown;
  min?: number | null;
  max?: number | null;
  unit?: string | null;
  options?: SettingOption[];
  options_from?: ValueSource | null;
  order?: number | null;
  keywords?: string[];
  scope?: SettingScope;
  handoff?: HandoffTarget | null;
  tags?: string[];
  included?: boolean | null;
  deprecated_message?: string | null;
  replaced_by?: string | null;
  renamed_from?: string[];
  since?: number | null;
  removed_in?: number | null;
  visible_when?: VisibleWhen | null;
}

/// One group of settings items.
export interface SettingsSection {
  label: string;
  description?: string | null;
  order?: number | null;
  items: SettingsItem[];
}

/// The whole declared schema.
export interface SettingsSchema {
  version: number;
  sections: SettingsSection[];
}

/// Everything the page needs to render one app's settings
/// (`app_settings_page`, camelCase).
export interface AppSettingsPage {
  appId: string;
  schema: SettingsSchema;
  /// The value in force per declared key, as JSON.
  values: Record<string, unknown>;
  /// Keys the USER chose (vs the shipped default) - "reset to default" is only
  /// honest on these.
  userSet: string[];
  /// Dynamic sources that could not be resolved, key -> plain-words reason
  /// ("no devices" and "could not ask" must not look the same).
  unavailable: Record<string, string>;
}

/// What came back from a write (`app_settings_write`, camelCase).
export interface WriteAnswer {
  ok: boolean;
  changed: string[];
  refusedKey: string;
  message: string;
}

/// Sections in render order (`order` first, declaration order after).
export function orderedSections(schema: SettingsSchema): SettingsSection[] {
  return [...schema.sections].sort((a, b) => (a.order ?? Number.MAX_SAFE_INTEGER) - (b.order ?? Number.MAX_SAFE_INTEGER));
}

/// Items in render order, dropping `included: false`-style hidden ones.
export function orderedItems(section: SettingsSection): SettingsItem[] {
  return section.items
    .filter((i) => i.included !== false)
    .sort((a, b) => (a.order ?? Number.MAX_SAFE_INTEGER) - (b.order ?? Number.MAX_SAFE_INTEGER));
}

/// Whether an item is visible given the current values (the `visible_when`
/// rule; a missing referenced value hides the dependent item).
export function isVisible(item: SettingsItem, values: Record<string, unknown>): boolean {
  const rule = item.visible_when;
  if (!rule) return true;
  const v = values[rule.key];
  const s = v === undefined || v === null ? "" : String(v);
  if (rule.equals !== undefined && rule.equals !== null) return s === rule.equals;
  if (rule.in_ !== undefined && rule.in_ !== null) return rule.in_.includes(s);
  return true;
}

/// The keys present in the stored values but absent from the schema - settings
/// from an older version of the app, surfaced instead of silently dropped.
export function orphanKeys(page: AppSettingsPage): string[] {
  const declared = new Set(page.schema.sections.flatMap((s) => s.items.map((i) => i.key)));
  return Object.keys(page.values).filter((k) => !declared.has(k));
}
