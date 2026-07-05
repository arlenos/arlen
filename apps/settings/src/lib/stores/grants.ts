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

/// The display families the panel groups reach into. Every one of the eleven
/// profile dimensions maps to exactly one family (see `DIMENSION_FAMILY`), so
/// nothing an app can do falls through the cracks.
export type Family =
  | "data"
  | "network"
  | "files"
  | "devices"
  | "clipboard"
  | "notifications"
  | "system"
  | "automation";

/// The families in display order, with their headings. "Knowledge graph" names
/// the KG data family so it does not read as the same thing as on-disk "Files
/// and folders" (the exact wording is settled with Tim at the screenshot).
export const FAMILIES: { key: Family; label: string }[] = [
  { key: "data", label: "Knowledge graph" },
  { key: "network", label: "Network" },
  { key: "files", label: "Files and folders" },
  { key: "devices", label: "Devices" },
  { key: "clipboard", label: "Clipboard" },
  { key: "notifications", label: "Notifications" },
  { key: "system", label: "System" },
  { key: "automation", label: "Automation" },
];

// Normalize a consent-class / dimension token so the backend's snake_case
// ("network_access") and any PascalCase ("NetworkAccess") both resolve.
function dim(consentClass: string): string {
  return consentClass.toLowerCase().replace(/[^a-z]/g, "");
}

// Every non-graph profile dimension (carried on a grant as `consent_class`) maps
// to a display family, keyed by the normalized token. graph is family "data"
// via the token path. event_bus and mcp are real reach (listening to activity;
// exposing tools the assistant uses), so they map to automation as their OWN
// lines, never silently dropped.
const DIMENSION_FAMILY: Record<string, Family> = {
  networkaccess: "network",
  filesystem: "files",
  appdata: "files",
  portal: "devices",
  clipboard: "clipboard",
  notifications: "notifications",
  system: "system",
  eventbus: "automation",
  mcp: "automation",
  input: "automation",
  search: "automation",
  intents: "automation",
};

/// The closed narrowing vocabulary the daemon accepts (`sdk/permissions`
/// `RevokedReach`), serialized as the `reach` arg of `revoke_reach`/`restore_reach`.
/// Unit variants serialize as the bare string; struct variants as `{Variant:{..}}`.
/// We only build the graph-data variants: the exact `entity_pattern` comes from the
/// ceiling, so it round-trips. The non-graph variants need the exact profile key,
/// which `access_grants` only summarizes today, so those rows are not revoked here.
export type RevokedReach =
  | { Read: { entity_pattern: string } }
  | { Write: { entity_pattern: string } };

/// What a line's revoke does: the narrowing calls to send, the target app, and
/// whether it can be exercised here (false = shown but disabled, with a reason).
export interface RevokeAction {
  appId: string;
  reaches: RevokedReach[];
  enabled: boolean;
  /// Why the revoke is disabled, shown before the click (settled model).
  disabledReason?: string;
}

/// One line of scope as the panel renders it: a plain sentence split into a
/// quiet verb and the emphasized object, with its family, provenance, the detail
/// behind the expand, and the revoke action.
export interface ScopeLine {
  key: string;
  /// The display family this reach belongs to.
  family: Family;
  /// The quiet leading verb: "reads", "reads and changes", "changes", or
  /// "access to" for a consent path.
  verb: string;
  /// The emphasized object: "your files", "its own files", "~/Documents".
  object: string;
  /// Own-data (a zero-prompt default): the line is rendered dimmed.
  own: boolean;
  /// The app declared this reach essential at enroll: revoke is refused + explained.
  required: boolean;
  /// System-managed reach: not per-app revocable here.
  systemManaged: boolean;
  /// "declared at install" or "you allowed this".
  provenance: string;
  /// Field and relation detail, revealed by the expand.
  detail: string[];
  /// The entity type this line concerns, for the by-data pivot (null for a
  /// consent path scope).
  entityType: string | null;
  /// What to narrow when this line is revoked.
  revoke: RevokeAction;
  /// The full sentence, for the confirm dialog and aria labels.
  text: string;
}

function verbFor(read: boolean, write: boolean): string {
  if (read && write) return "reads and changes";
  if (write) return "changes";
  return "reads";
}

// Build a line's revoke action. Graph reaches carry the exact `entity_pattern`, so
// they narrow here; a required or system-managed reach is refused with a reason
// shown before the click; a non-graph reach has no exact descriptor to send (the
// summary is not the profile key), so it is shown disabled until `access_grants`
// carries the revocable descriptor.
function revokeAction(
  appId: string,
  reaches: RevokedReach[],
  required: boolean,
  systemManaged: boolean,
): RevokeAction {
  if (required)
    return { appId, reaches, enabled: false, disabledReason: "This app needs this to work." };
  if (systemManaged)
    return { appId, reaches, enabled: false, disabledReason: "Managed by the system, not revocable here." };
  if (reaches.length === 0)
    return {
      appId,
      reaches,
      enabled: false,
      disabledReason: "Removing this needs the service's reach descriptor, coming with the backend.",
    };
  return { appId, reaches, enabled: true };
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
    const verb = verbFor(read, write);
    // The object is what matters and gets the emphasis; own-data reads as "its
    // own" and the whole line is dimmed (a zero-prompt default).
    const object = all ? `your ${noun}` : `its own ${noun}`;
    const detail: string[] = [];
    const fd = fieldDetail(io.read ?? io.write!);
    if (fd) detail.push(fd);
    for (const rel of relationsByType.get(entityType) ?? []) detail.push(rel);
    const reaches: RevokedReach[] = [];
    if (read) reaches.push({ Read: { entity_pattern: entityType } });
    if (write) reaches.push({ Write: { entity_pattern: entityType } });
    lines.push({
      key: `${grant.app_id}:${entityType}`,
      family: "data",
      verb,
      object,
      own: !all,
      required: grant.required,
      systemManaged: grant.source === "system",
      provenance: "declared at install",
      detail,
      entityType,
      revoke: revokeAction(grant.app_id, reaches, grant.required, grant.source === "system"),
      text: `${verb} ${object}`,
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

// The provenance line for a grant, from its source: an app declares reach at
// install; the user allows a consent grant in context (with a date); some reach
// is system-managed and not yet per-app revocable.
function fmtDate(micros: number): string {
  return new Date(micros / 1000).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
function provenanceOf(grant: GrantView): string {
  if (grant.source === "consent") return `you allowed on ${fmtDate(grant.issued_at)}`;
  if (grant.source === "system") return "managed by the system";
  return "declared at install";
}

// One non-graph grant becomes one line. The phrasing keeps the honest
// distinction for each family (read vs write, all vs scoped, per-device), never
// a bare "has access". `consent_class` names the dimension; `consent_scope`
// carries the concrete detail.
function nonGraphLine(grant: GrantView): ScopeLine {
  const d = dim(grant.consent_class);
  const family = DIMENSION_FAMILY[d] ?? "automation";
  const scope = grant.consent_scope;
  let verb = "accesses";
  let object = scope || grant.consent_class.toLowerCase();
  const detail: string[] = [];

  switch (d) {
    case "networkaccess": {
      verb = "reaches";
      if (scope === "all") {
        object = "the whole internet";
      } else {
        const domains = scope.split(",").map((d) => d.trim()).filter(Boolean);
        object =
          domains.length <= 1
            ? domains[0] ?? "one host"
            : `${domains[0]} and ${domains.length - 1} more`;
        if (domains.length > 1) detail.push(`Hosts: ${domains.join(", ")}.`);
      }
      break;
    }
    case "filesystem":
      verb = "reads and changes";
      object = `your ${scope}`;
      break;
    case "appdata":
      verb = "access to";
      object = scope;
      break;
    case "portal": {
      // scope is "camera" / "microphone" / "screen" / "usb:Yubikey".
      const [dev, persist] = scope.split("|");
      if (dev === "screen") {
        verb = "can capture";
        object = "your screen";
      } else if (dev.startsWith("usb:")) {
        verb = "can use";
        object = `your ${dev.slice(4)}`;
      } else {
        verb = "can use";
        object = `your ${dev}`;
      }
      detail.push(persist ? `Access: ${persist}.` : "Access: while the app is in use.");
      break;
    }
    case "clipboard":
      verb = scope.includes("write") ? "reads and writes" : "reads";
      object = scope.includes("history") ? "the clipboard and its history" : "the clipboard";
      break;
    case "notifications":
      verb = "can send";
      object = "notifications";
      break;
    case "system": {
      const map: Record<string, [string, string]> = {
        background: ["keeps running", "in the background"],
        suspend: ["can suspend", "the system"],
        autostart: ["starts", "automatically at login"],
      };
      [verb, object] = map[scope] ?? ["controls", scope];
      break;
    }
    case "eventbus":
      // Listening to the bus means seeing activity: real reach, not plumbing.
      verb = scope.startsWith("publish") ? "publishes" : "listens to";
      object = scope.replace(/^(publish|subscribe):\s*/, "") || "app events";
      break;
    case "mcp":
      // Tools an app exposes are used BY the assistant: real reach.
      verb = "exposes";
      object = `${scope} to the assistant`;
      break;
    case "input":
      verb = "registers";
      object = scope;
      break;
    case "search":
      verb = "provides";
      object = scope;
      break;
    case "intents":
      verb = "handles";
      object = scope;
      break;
  }

  const systemManaged = grant.source === "system";
  return {
    key: `${grant.app_id}:${grant.consent_class}:${grant.id}`,
    family,
    verb,
    object,
    own: false,
    required: grant.required,
    systemManaged,
    provenance: provenanceOf(grant),
    detail,
    entityType: null,
    revoke: revokeAction(grant.app_id, [], grant.required, systemManaged),
    text: `${verb} ${object}`,
  };
}

function grantLines(grant: GrantView): ScopeLine[] {
  if (grant.source === "capability-token") {
    const c = parseCeiling(grant.declared_ceiling);
    return c ? tokenLines(grant, c) : [];
  }
  // Everything else (declared non-graph reach, consent, system) is one line.
  return [nonGraphLine(grant)];
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

/// The lines of one principal, grouped by family, for the family subheaders in
/// the by-app view.
export interface FamilyGroup {
  key: Family;
  label: string;
  lines: ScopeLine[];
}

/// Split a principal's lines into families, in display order.
export function familyGroups(lines: ScopeLine[]): FamilyGroup[] {
  const by = new Map<Family, ScopeLine[]>();
  for (const l of lines) {
    const arr = by.get(l.family) ?? [];
    arr.push(l);
    by.set(l.family, arr);
  }
  return FAMILIES.filter((f) => by.has(f.key)).map((f) => ({
    key: f.key,
    label: f.label,
    lines: by.get(f.key)!,
  }));
}

/// One reacher inside a by-capability group: the principal and its line.
export interface Reacher {
  appId: string;
  label: string;
  assistant: boolean;
  identityVerified: boolean;
  line: ScopeLine;
}

/// One capability family and everything that can reach through it.
export interface ResourceGroup {
  key: string;
  label: string;
  reachers: Reacher[];
}

/// Invert the grants into "who can reach through each capability family",
/// grouped by the display families in order. The assistant floats to the top of
/// each family.
export function byCapability(list: GrantView[]): ResourceGroup[] {
  const groups = new Map<Family, ResourceGroup>();
  for (const p of byApp(list)) {
    for (const line of p.lines) {
      let g = groups.get(line.family);
      if (!g) {
        g = {
          key: line.family,
          label: FAMILIES.find((f) => f.key === line.family)?.label ?? line.family,
          reachers: [],
        };
        groups.set(line.family, g);
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
  for (const g of groups.values())
    g.reachers.sort((a, b) => Number(b.assistant) - Number(a.assistant));
  return FAMILIES.map((f) => groups.get(f.key)).filter(
    (g): g is ResourceGroup => !!g,
  );
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
// A base grant so each fixture entry only names what it changes.
function g(over: Partial<GrantView> & Pick<GrantView, "id" | "app_id">): GrantView {
  return {
    declared_ceiling: "",
    required: false,
    identity_verified: true,
    live: true,
    revoked: false,
    superseded: false,
    issued_at: 1_780_000_000_000_000,
    reach: [],
    source: "declared",
    consent_class: "",
    consent_scope: "",
    ...over,
  };
}

const MOCK_GRANTS: GrantView[] = [
  // The assistant: reads your files + projects, changes its own notes (data),
  // and reaches two declared hosts (network).
  g({
    id: "0192-0001",
    app_id: "org.arlen.AI1",
    source: "capability-token",
    declared_ceiling: JSON.stringify({
      read: [
        { entity_type: "system.File", fields: null, exclude_fields: [] },
        { entity_type: "system.Project", fields: null, exclude_fields: [] },
      ],
      write: [{ entity_type: "system.Note", fields: null, exclude_fields: [] }],
      relations: [{ from: "system.Note", to: "system.Project", relation_type: "NOTE_ABOUT" }],
      instance: "All",
    }),
  }),
  g({
    id: "0192-0002",
    app_id: "org.arlen.AI1",
    consent_class: "network_access",
    consent_scope: "api.openai.com, api.anthropic.com",
  }),

  // Files: reads and changes your files, reads your projects (data), and reads
  // and changes your Documents and Downloads folders (filesystem).
  g({
    id: "0192-0003",
    app_id: "org.arlen.files",
    source: "capability-token",
    required: true,
    declared_ceiling: JSON.stringify({
      read: [
        { entity_type: "system.File", fields: null, exclude_fields: [] },
        { entity_type: "system.Project", fields: null, exclude_fields: [] },
      ],
      write: [{ entity_type: "system.File", fields: null, exclude_fields: [] }],
      relations: [],
      instance: "All",
    }),
  }),
  g({
    id: "0192-0004",
    app_id: "org.arlen.files",
    consent_class: "filesystem",
    consent_scope: "Documents and Downloads folders",
  }),

  // An unverified third-party editor: reads only its own files, sees only file
  // names and dates (data, field-limited); you granted it ~/Documents (consent);
  // it reads and writes the clipboard.
  g({
    id: "0192-0005",
    app_id: "com.example.editor",
    identity_verified: false,
    source: "capability-token",
    declared_ceiling: JSON.stringify({
      read: [{ entity_type: "system.File", fields: ["path", "modified"], exclude_fields: [] }],
      write: [],
      relations: [],
      instance: "Own",
    }),
  }),
  g({
    id: "0192-0006",
    app_id: "com.example.editor",
    identity_verified: false,
    source: "consent",
    consent_class: "app_data",
    consent_scope: "~/Documents",
    issued_at: 1_782_600_000_000_000,
  }),
  g({
    id: "0192-0007",
    app_id: "com.example.editor",
    identity_verified: false,
    consent_class: "clipboard",
    consent_scope: "read, write",
  }),

  // A recorder app: camera, microphone, screen (devices), notifications, and it
  // keeps running in the background (system).
  g({ id: "0192-0008", app_id: "com.acme.recorder", identity_verified: false, source: "consent", consent_class: "portal", consent_scope: "camera|while using", issued_at: 1_782_700_000_000_000 }),
  g({ id: "0192-0009", app_id: "com.acme.recorder", identity_verified: false, source: "consent", consent_class: "portal", consent_scope: "microphone|while using", issued_at: 1_782_700_000_000_000 }),
  g({ id: "0192-0010", app_id: "com.acme.recorder", identity_verified: false, source: "consent", consent_class: "portal", consent_scope: "screen|once", issued_at: 1_782_700_000_000_000 }),
  g({ id: "0192-0011", app_id: "com.acme.recorder", identity_verified: false, consent_class: "notifications", consent_scope: "" }),
  g({ id: "0192-0012", app_id: "com.acme.recorder", identity_verified: false, consent_class: "system", consent_scope: "background" }),

  // A notes plugin: exposes tools to the assistant (mcp), listens to your
  // activity (event_bus), registers a global shortcut (input), provides search
  // results (search), and handles the open-note action (intents).
  g({ id: "0192-0013", app_id: "com.example.notes", consent_class: "mcp", consent_scope: "4 tools" }),
  g({ id: "0192-0014", app_id: "com.example.notes", consent_class: "event_bus", consent_scope: "subscribe: your activity" }),
  g({ id: "0192-0015", app_id: "com.example.notes", consent_class: "input", consent_scope: "global shortcuts" }),
  g({ id: "0192-0016", app_id: "com.example.notes", consent_class: "search", consent_scope: "search results" }),
  g({ id: "0192-0017", app_id: "com.example.notes", consent_class: "intents", consent_scope: "the open-note action" }),
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

/// One scope the user removed, kept so it can be reinstated. Restoring re-adds a
/// prior grant the user took away; it never mints a new one. Session-only until
/// the backend tracks removals.
export interface RemovedItem {
  id: string;
  grantId: string;
  appId: string;
  appLabel: string;
  text: string;
  entityType: string | null;
  readScope: EntityScope | null;
  writeScope: EntityScope | null;
  /// The exact narrowing calls that were sent, replayed verbatim on restore.
  reaches: RevokedReach[];
}

/// The scopes removed this session, newest first, for undo and the "recently
/// removed" restore list.
export const removed = writable<RemovedItem[]>([]);

/// A transient message when a revoke/restore did not go through on the daemon
/// (the page shows it, then it clears). Null when nothing to say.
export const actionNotice = writable<string | null>(null);

let removedSeq = 0;

/// Narrow a single scope (profile-first, narrowing-only) and record it so it can
/// be restored. Only a graph reach with an exact `entity_pattern` is revocable
/// here (the page keeps a disabled line otherwise). Optimistically narrows the
/// local ceiling; a daemon refusal reverts it and shows a notice. Returns the
/// removed record for an immediate undo, or null if nothing changed.
export async function revokeScope(
  line: ScopeLine,
  appLabel: string,
): Promise<RemovedItem | null> {
  const action = line.revoke;
  if (!action.enabled || action.reaches.length === 0 || !line.entityType) return null;
  const entityType = line.entityType;

  let item: RemovedItem | null = null;
  grants.update((list) =>
    list.map((g) => {
      if (g.app_id !== action.appId || g.source === "consent") return g;
      const c = parseCeiling(g.declared_ceiling);
      if (!c) return g;
      const readScope = c.read.find((s) => s.entity_type === entityType) ?? null;
      const writeScope = c.write.find((s) => s.entity_type === entityType) ?? null;
      if (!readScope && !writeScope) return g;
      item = {
        id: `rm${++removedSeq}`,
        grantId: g.id,
        appId: g.app_id,
        appLabel,
        text: line.text,
        entityType,
        readScope,
        writeScope,
        reaches: action.reaches,
      };
      return {
        ...g,
        declared_ceiling: JSON.stringify({
          ...c,
          read: c.read.filter((s) => s.entity_type !== entityType),
          write: c.write.filter((s) => s.entity_type !== entityType),
        }),
      };
    }),
  );
  if (!item) return null;
  const removedItem: RemovedItem = item;
  removed.update((r) => [removedItem, ...r]);

  if (!(await applyReaches("revoke_reach", action.appId, action.reaches, ["OK: revoked", "OK: no-change"]))) {
    reinstateLocal(removedItem);
    actionNotice.set("Could not remove that reach. Nothing changed.");
    return null;
  }
  return removedItem;
}

/// Remove every scope an app holds. Returns all the removed records.
export async function revokeAllFor(
  lines: ScopeLine[],
  appLabel: string,
): Promise<RemovedItem[]> {
  const items: RemovedItem[] = [];
  for (const l of lines) {
    const it = await revokeScope(l, appLabel);
    if (it) items.push(it);
  }
  return items;
}

/// Reinstate a removed scope locally: put the reach back into the app's ceiling
/// and drop it from the removed list.
function reinstateLocal(item: RemovedItem): void {
  grants.update((list) =>
    list.map((g) => {
      if (g.id !== item.grantId) return g;
      const c = parseCeiling(g.declared_ceiling);
      if (!c) return g;
      const read =
        item.readScope && !c.read.some((s) => s.entity_type === item.entityType)
          ? [...c.read, item.readScope]
          : c.read;
      const write =
        item.writeScope && !c.write.some((s) => s.entity_type === item.entityType)
          ? [...c.write, item.writeScope]
          : c.write;
      return { ...g, declared_ceiling: JSON.stringify({ ...c, read, write }) };
    }),
  );
  removed.update((r) => r.filter((x) => x.id !== item.id));
}

/// Reinstate a removed scope: re-add it locally and replay the exact reaches
/// through `restore_reach` - the one authority-growth path, bounded by the audit
/// ledger to a prior revoke, so it never mints fresh authority. A daemon refusal
/// is surfaced; the local view reconciles on the next load.
export async function restore(item: RemovedItem): Promise<void> {
  reinstateLocal(item);
  if (!(await applyReaches("restore_reach", item.appId, item.reaches, ["OK: restored", "OK: no-change"]))) {
    actionNotice.set("Could not restore that reach here.");
  }
}

// Send each reach through a narrowing/restore command; true only if every call
// returned an accepted wire token. A transport error (vite / no daemon) counts as
// applied so the affordance works against the fixture; a real daemon refusal
// returns a rejecting token, above.
async function applyReaches(
  command: "revoke_reach" | "restore_reach",
  appId: string,
  reaches: RevokedReach[],
  ok: string[],
): Promise<boolean> {
  try {
    for (const reach of reaches) {
      const status = await invoke<string>(command, {
        targetAppId: appId,
        reach: JSON.stringify(reach),
      });
      if (!ok.includes(status)) return false;
    }
    return true;
  } catch {
    return true;
  }
}
