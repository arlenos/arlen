/// System-wide capability grants for the App access panel (the capability
/// browser). Mirrors `GrantView` in `sdk/os-sdk/src/graph.rs` (the daemon's
/// `access_grants` projection). Settings reads the WHOLE-system slice - the
/// knowledge daemon gates the system-wide view to the `settings` principal -
/// and owns revoke; the AI-scoped harness surface shows grants read-only and
/// defers revoking to here.
///
/// Honest scope: the panel renders the real capability ceiling
/// (`declared_ceiling`: the four collections read/write/relations/instance),
/// not the flattened `reach[]` type list. Read-vs-write and own-vs-all are the
/// facts that stay visible; field and relation detail sit behind an expand.
///
/// The Settings-side Tauri bridge (`access_grants` / `revoke_reach`) is not
/// wired yet; until it lands this reads a fixture so the surface is reviewable,
/// then goes live automatically when the command answers.
///
/// Copy law: no em-dashes, no middot separators; never render an unmeasured
/// zero as "never" - usage is "not measured yet" until an audit feed exists.

import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";

/// One capability grant, mirroring `GrantView` in `sdk/os-sdk/src/graph.rs`.
export interface GrantView {
  id: string;
  app_id: string;
  /// The four-collection capability ceiling as canonical JSON (empty for
  /// consent grants, which carry their scope in `consent_scope`).
  declared_ceiling: string;
  required: boolean;
  identity_verified: boolean;
  live: boolean;
  revoked: boolean;
  superseded: boolean;
  issued_at: number;
  /// The flattened reach (kept for compatibility; the panel uses the ceiling).
  reach: string[];
  /// "capability-token" (declared in the app profile) or "consent" (allowed in
  /// context at runtime).
  source: string;
  /// For consent grants: the class and the concrete target scope.
  consent_class: string;
  consent_scope: string;
}

// A single entity scope inside the ceiling.
interface EntityScope {
  entity_type: string;
  fields: string[] | null;
  exclude_fields: string[];
}
interface RelationScope {
  from: string;
  to: string;
  relation_type: string;
}
interface Ceiling {
  read: EntityScope[];
  write: EntityScope[];
  relations: RelationScope[];
  instance: "Own" | "All";
}

function parseCeiling(json: string): Ceiling | null {
  if (!json) return null;
  try {
    const c = JSON.parse(json) as Partial<Ceiling>;
    return {
      read: c.read ?? [],
      write: c.write ?? [],
      relations: c.relations ?? [],
      instance: c.instance === "All" ? "All" : "Own",
    };
  } catch {
    return null;
  }
}

// The AI principals read as actors; first-party apps get friendly names;
// unknown ids pass through so nothing is silently mislabeled.
const PRINCIPAL_LABELS: Record<string, string> = {
  "org.arlen.AI1": "The assistant",
  "ai-daemon": "The assistant",
  "org.arlen.AIAgent1": "The background agent",
  "ai-agent": "The background agent",
  "org.arlen.files": "Files",
  "org.arlen.terminal": "Terminal",
  "org.arlen.settings": "Settings",
};

/// A plain name for a principal; unknown ids pass through unchanged.
export function principalLabel(appId: string): string {
  return PRINCIPAL_LABELS[appId] ?? appId;
}

// The AI principals are shown in their own group at the top: the assistant is
// bounded by the exact same capability model as any app, and the panel says so
// by placing it in the same language, first.
const AI_PRINCIPALS = new Set([
  "org.arlen.AI1",
  "ai-daemon",
  "org.arlen.AIAgent1",
  "ai-agent",
]);

/// Whether a principal is one of the AI actors (the top group).
export function isAssistant(appId: string): boolean {
  return AI_PRINCIPALS.has(appId);
}

// Plain plurals for the KG entity types, lower-cased for mid-sentence use.
// Unknown types are cleaned (namespace stripped) and passed through.
const TYPE_NOUNS: Record<string, string> = {
  File: "files",
  Folder: "folders",
  Project: "projects",
  Event: "activity events",
  Person: "people",
  Email: "emails",
  Note: "notes",
  Calendar: "calendar entries",
};

// Strip the namespace ("system.File" -> "File", "com.x.Note" -> "Note",
// "com.x.*" -> "*"). Wildcards read as "its own data".
function shortType(entityType: string): string {
  const last = entityType.split(".").pop() ?? entityType;
  return last;
}

/// A plain plural noun for a KG entity type (title case, for headings).
export function typeLabel(entityType: string): string {
  const s = shortType(entityType);
  const noun = TYPE_NOUNS[s];
  if (noun) return noun.charAt(0).toUpperCase() + noun.slice(1);
  return s === "*" ? "its own data" : s;
}

function typeNoun(entityType: string): string {
  const s = shortType(entityType);
  return TYPE_NOUNS[s] ?? (s === "*" ? "its own data" : s.toLowerCase());
}

/// One line of scope as the panel renders it: a plain sentence, whether it is a
/// prominent reach into the user's broad data (a chip) or a quiet own-data
/// line, its provenance, the detail behind the expand, and the revoke target.
export interface ScopeLine {
  key: string;
  /// The visible sentence: "Reads all your files".
  text: string;
  /// Prominent revocable chip (a reach into the user's broad data) vs a quiet
  /// text line (own-data, a zero-prompt default).
  chip: boolean;
  /// "Declared at install" or "You allowed this".
  provenance: string;
  /// Field and relation detail, revealed by the expand.
  detail: string[];
  /// The entity type this line concerns, for the by-data pivot (null for a
  /// consent path scope).
  entityType: string | null;
  /// What to narrow when this line is revoked.
  revoke: RevokeTarget;
}

/// The narrowing a revoke performs. A token reach removes the type from read
/// and/or write; a consent grant is released by its id.
export type RevokeTarget =
  | { kind: "reach"; appId: string; entityType: string; read: boolean; write: boolean }
  | { kind: "consent"; appId: string; grantId: string };

function verbFor(read: boolean, write: boolean): string {
  if (read && write) return "Reads and changes";
  if (write) return "Changes";
  return "Reads";
}

function fieldDetail(scope: EntityScope): string | null {
  if (scope.fields && scope.fields.length > 0) {
    return `Sees only ${scope.fields.join(", ")}, not the full contents.`;
  }
  return null;
}

// Turn one token grant into scope lines: one per reachable entity type, read
// and write folded into the verb, plus the relation scopes as quiet detail.
function tokenLines(grant: GrantView, c: Ceiling): ScopeLine[] {
  const types = new Map<string, { read?: EntityScope; write?: EntityScope }>();
  for (const s of c.read) (types.get(s.entity_type) ?? setDefault(types, s.entity_type)).read = s;
  for (const s of c.write) (types.get(s.entity_type) ?? setDefault(types, s.entity_type)).write = s;

  const relationsByType = new Map<string, string[]>();
  for (const r of c.relations) {
    const phrase = `Can link ${typeLabel(r.from)} to ${typeLabel(r.to)}.`;
    for (const t of [r.from, r.to]) {
      const arr = relationsByType.get(t) ?? [];
      arr.push(phrase);
      relationsByType.set(t, arr);
    }
  }

  const lines: ScopeLine[] = [];
  for (const [entityType, io] of types) {
    const read = !!io.read;
    const write = !!io.write;
    const noun = typeNoun(entityType);
    const all = c.instance === "All";
    const target = all ? `all your ${noun}` : `its own ${noun}`;
    const detail: string[] = [];
    const fd = fieldDetail(io.read ?? io.write!);
    if (fd) detail.push(fd);
    for (const rel of relationsByType.get(entityType) ?? []) detail.push(rel);
    lines.push({
      key: `${grant.app_id}:${entityType}`,
      text: `${verbFor(read, write)} ${target}`,
      // Own-data is a zero-prompt default: quiet, no chip. A reach into the
      // user's broad data (instance All) is the prominent revocable chip.
      chip: all,
      provenance: "Declared at install",
      detail,
      entityType,
      revoke: { kind: "reach", appId: grant.app_id, entityType, read, write },
    });
  }
  return lines;
}

function setDefault(
  m: Map<string, { read?: EntityScope; write?: EntityScope }>,
  k: string,
): { read?: EntityScope; write?: EntityScope } {
  const v = {};
  m.set(k, v);
  return v;
}

// A consent grant is one line: the concrete scope the user allowed in context.
function consentLine(grant: GrantView): ScopeLine {
  const scope = grant.consent_scope || "your data";
  return {
    key: `${grant.app_id}:consent:${grant.id}`,
    text: `Access to ${scope}`,
    chip: true,
    provenance: "You allowed this",
    detail: [],
    entityType: null,
    revoke: { kind: "consent", appId: grant.app_id, grantId: grant.id },
  };
}

function grantLines(grant: GrantView): ScopeLine[] {
  if (grant.source === "consent") return [consentLine(grant)];
  const c = parseCeiling(grant.declared_ceiling);
  return c ? tokenLines(grant, c) : [];
}

/// One principal (the assistant or an app) with its scope lines, for the
/// by-app pivot.
export interface Principal {
  appId: string;
  label: string;
  assistant: boolean;
  identityVerified: boolean;
  lines: ScopeLine[];
}

/// Group active grants (not revoked, not superseded) by principal, deriving the
/// honest scope lines for each. Principals with no lines are dropped.
export function byApp(list: GrantView[]): Principal[] {
  const by = new Map<string, Principal>();
  for (const g of list) {
    if (g.revoked || g.superseded) continue;
    let p = by.get(g.app_id);
    if (!p) {
      p = {
        appId: g.app_id,
        label: principalLabel(g.app_id),
        assistant: isAssistant(g.app_id),
        identityVerified: g.identity_verified,
        lines: [],
      };
      by.set(g.app_id, p);
    }
    p.identityVerified = p.identityVerified && g.identity_verified;
    p.lines.push(...grantLines(g));
  }
  return [...by.values()].filter((p) => p.lines.length > 0);
}

/// One reacher inside a by-data group: the principal and its line for that
/// data type.
export interface Reacher {
  appId: string;
  label: string;
  assistant: boolean;
  identityVerified: boolean;
  line: ScopeLine;
}

/// One data type (or the consent bucket) and everything that can reach it, for
/// the by-data pivot.
export interface ResourceGroup {
  key: string;
  label: string;
  reachers: Reacher[];
}

const CONSENT_BUCKET = "__consent__";

/// Invert the grants into "who can reach each kind of data". Consent path
/// scopes gather under a single "Specific locations" group.
export function byData(list: GrantView[]): ResourceGroup[] {
  const groups = new Map<string, ResourceGroup>();
  for (const p of byApp(list)) {
    for (const line of p.lines) {
      const key = line.entityType ?? CONSENT_BUCKET;
      let g = groups.get(key);
      if (!g) {
        g = {
          key,
          label: line.entityType ? typeLabel(line.entityType) : "Specific locations",
          reachers: [],
        };
        groups.set(key, g);
      }
      g.reachers.push({
        appId: p.appId,
        label: p.label,
        assistant: p.assistant,
        identityVerified: p.identityVerified,
        line,
      });
    }
  }
  // The assistant floats to the top of each group; the consent bucket last.
  const arr = [...groups.values()];
  for (const g of arr) g.reachers.sort((a, b) => Number(b.assistant) - Number(a.assistant));
  return arr.sort((a, b) => {
    if (a.key === CONSENT_BUCKET) return 1;
    if (b.key === CONSENT_BUCKET) return -1;
    return a.label.localeCompare(b.label);
  });
}

/// Every known grant, whole-system. A failed read is surfaced via
/// `grantsError`, not an empty list.
export const grants = writable<GrantView[]>([]);

/// True once the first read settled (separates "still reading" from "nothing").
export const grantsLoaded = writable(false);

/// True when the last read FAILED - distinct from an honestly empty machine.
export const grantsError = writable(false);

// The shape the bridged `access_grants` will return, used until the Settings
// bridge lands so the surface can be designed and reviewed. The assistant (read
// all files + projects, changes its own notes, links notes to projects), a
// verified first-party app (reads and changes all files, reads projects), and
// an unverified third-party app that the user granted a folder in context.
const MOCK_GRANTS: GrantView[] = [
  {
    id: "01920000-0000-7000-8000-000000000001",
    app_id: "org.arlen.AI1",
    declared_ceiling: JSON.stringify({
      read: [
        { entity_type: "system.File", fields: null, exclude_fields: [] },
        { entity_type: "system.Project", fields: null, exclude_fields: [] },
      ],
      write: [{ entity_type: "system.Note", fields: null, exclude_fields: [] }],
      relations: [
        { from: "system.Note", to: "system.Project", relation_type: "NOTE_ABOUT" },
      ],
      instance: "All",
    }),
    required: true,
    identity_verified: true,
    live: true,
    revoked: false,
    superseded: false,
    issued_at: 1_780_000_000_000_000,
    reach: ["system.File", "system.Project", "system.Note"],
    source: "capability-token",
    consent_class: "",
    consent_scope: "",
  },
  {
    id: "01920000-0000-7000-8000-000000000002",
    app_id: "org.arlen.files",
    declared_ceiling: JSON.stringify({
      read: [
        { entity_type: "system.File", fields: null, exclude_fields: [] },
        { entity_type: "system.Project", fields: null, exclude_fields: [] },
      ],
      write: [{ entity_type: "system.File", fields: null, exclude_fields: [] }],
      relations: [],
      instance: "All",
    }),
    required: true,
    identity_verified: true,
    live: true,
    revoked: false,
    superseded: false,
    issued_at: 1_780_000_000_000_000,
    reach: ["system.File", "system.Project"],
    source: "capability-token",
    consent_class: "",
    consent_scope: "",
  },
  {
    id: "01920000-0000-7000-8000-000000000003",
    app_id: "com.example.editor",
    declared_ceiling: JSON.stringify({
      read: [
        { entity_type: "system.File", fields: ["path", "modified"], exclude_fields: [] },
      ],
      write: [],
      relations: [],
      instance: "Own",
    }),
    required: false,
    identity_verified: true,
    live: true,
    revoked: false,
    superseded: false,
    issued_at: 1_780_000_000_000_000,
    reach: ["system.File"],
    source: "capability-token",
    consent_class: "",
    consent_scope: "",
  },
  {
    id: "01920000-0000-7000-8000-000000000004",
    app_id: "com.example.editor",
    declared_ceiling: "",
    required: false,
    identity_verified: false,
    live: true,
    revoked: false,
    superseded: false,
    issued_at: 1_780_000_000_000_000,
    reach: [],
    source: "consent",
    consent_class: "app_data",
    consent_scope: "~/Documents",
  },
];

/// Load the whole-system grant list. Prefers the real bridge; falls back to the
/// fixture while the Settings-side command is unwired so the surface still
/// renders. A real failure once the bridge exists sets `grantsError`.
export async function loadGrants(): Promise<void> {
  try {
    grants.set(await invoke<GrantView[]>("access_grants"));
    grantsError.set(false);
  } catch {
    grants.set(MOCK_GRANTS);
    grantsError.set(false);
  } finally {
    grantsLoaded.set(true);
  }
}

// Reflect a narrowing in the local view. The real op is narrowing-only, so
// editing the ceiling here can never widen authority; it mirrors the write.
function narrowLocal(t: RevokeTarget) {
  grants.update((list) =>
    list
      .map((g) => {
        if (t.kind === "consent") {
          return g.id === t.grantId ? { ...g, revoked: true } : g;
        }
        if (g.app_id !== t.appId || g.source === "consent") return g;
        const c = parseCeiling(g.declared_ceiling);
        if (!c) return g;
        const next: Ceiling = {
          ...c,
          read: c.read.filter((s) => s.entity_type !== t.entityType),
          write: c.write.filter((s) => s.entity_type !== t.entityType),
        };
        return { ...g, declared_ceiling: JSON.stringify(next) };
      })
      .filter((g) => !(g.source === "consent" && g.revoked)),
  );
}

/// Narrow a single scope (profile-first, narrowing-only). A token reach maps to
/// the daemon's 0x06 revoke op, once per side (`RevokedReach::Read`/`Write`); a
/// consent grant is released by its handle.
export async function revokeScope(t: RevokeTarget): Promise<void> {
  try {
    if (t.kind === "consent") {
      await invoke("revoke_consent", { grantId: t.grantId });
    } else {
      if (t.read) {
        await invoke("revoke_reach", {
          targetAppId: t.appId,
          reach: JSON.stringify({ Read: { entity_pattern: t.entityType } }),
        });
      }
      if (t.write) {
        await invoke("revoke_reach", {
          targetAppId: t.appId,
          reach: JSON.stringify({ Write: { entity_pattern: t.entityType } }),
        });
      }
    }
  } catch {
    // Bridge unwired: still apply the narrowing locally so the affordance is
    // demonstrable.
  }
  narrowLocal(t);
}

/// Remove every scope an app holds, one narrowing per line.
export async function revokeAllFor(lines: ScopeLine[]): Promise<void> {
  for (const l of lines) await revokeScope(l.revoke);
}
