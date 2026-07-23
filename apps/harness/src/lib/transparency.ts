/// Types and plain-language helpers for the AI transparency surface
/// (ai-transparency-surface.md). AI-scoped only: the AI's own grants,
/// working memory, reads, cost, and the off switch. Never the whole
/// machine's authority (that is the Knowledge app, per
/// kg-surface-allocation.md).
///
/// Copy law (shared with display.ts): no em-dashes, no middot
/// separators, no internal vocabulary. Short active sentences. The
/// surface never renders an unmeasured zero as "never": until a feed
/// exists it says "not measured yet."
import { invoke } from "@tauri-apps/api/core";

/// One capability grant, mirroring `GrantView` in `sdk/os-sdk/src/graph.rs`
/// (the daemon's `access_grants` projection). The harness reads only the
/// AI-scoped slice; the command returns grants already filtered to the AI
/// principals.
export interface GrantView {
  /// The grant id (the projected token's UUIDv7).
  id: string;
  /// The principal the grant belongs to (an AI app id).
  app_id: string;
  /// The declared capability ceiling, as canonical scope JSON.
  declared_ceiling: string;
  /// Whether this reach was declared essential at enroll.
  required: boolean;
  /// Whether the app identity is verified (the F3 caveat when false).
  identity_verified: boolean;
  /// Whether the projected token still verifies (resolved fresh).
  live: boolean;
  /// Whether the user severed this reach.
  revoked: boolean;
  /// Whether a fresher mint replaced this node.
  superseded: boolean;
  /// When the grant was issued (epoch micros).
  issued_at: number;
  /// The entity types this grant can reach.
  reach: string[];
}

/// The shape, never the content, of what the AI currently holds, from the
/// net-new `ai_working_set` endpoint (AIT-R1). This is the form stub the
/// coder fills; the surface renders exactly this contract. Showing the
/// held content would itself be the Recall failure, so the endpoint
/// returns shape only (Tim, 14 June decision 2).
export interface WorkingSet {
  /// The endpoint answered (false when introspection is unavailable).
  available: boolean;
  /// Whether the AI is holding a context slice right now.
  held: boolean;
  /// Per entity type, how many nodes are held (no node contents).
  entityCounts: { type: string; count: number }[];
  /// The behaviour whose work is holding the slice, if any.
  activeBehaviour: string | null;
  /// What that behaviour declared it reads (a tier key), turned into a
  /// plain phrase by `readsSentence`.
  declaredReads: string | null;
}

/// Open the Settings app at its AI section, where the `[ai] enabled`
/// master switch lives. There is no `settings://` scheme (open_url only
/// allows http/https/mailto), so this rides a host launch command the
/// coder wires (arlen-run to the Settings app, AI section). Failure is
/// silent: the worst case is the button doing nothing, not a crash.
export async function openAiSettings(): Promise<void> {
  try {
    await invoke("open_ai_settings");
  } catch {
    // The command is not wired yet (or the launch failed); nothing to
    // surface on a transparency surface.
  }
}

/// Read the AI-scoped grants. `null` when the read fails, which callers
/// render honestly (never as "no access").
export async function readGrants(): Promise<GrantView[] | null> {
  try {
    return await invoke<GrantView[]>("ai_access_grants");
  } catch {
    return null;
  }
}

/// Read the working-set shape. `null` when the read fails (rendered as
/// "can't read"); a returned `available: false` is the honest "not
/// available yet" state, distinct from "holding nothing."
export async function readWorkingSet(): Promise<WorkingSet | null> {
  try {
    return await invoke<WorkingSet>("ai_working_set");
  } catch {
    return null;
  }
}

/// The two AI principals in plain words. The query assistant and the
/// autonomous background agent read as two different actors to a person.
const PRINCIPAL_LABELS: Record<string, string> = {
  "ai-daemon": "The assistant",
  "org.arlen.AI1": "The assistant",
  "ai-agent": "The background agent",
  "org.arlen.AIAgent1": "The background agent",
};

/// A plain name for an AI principal; unknown ids pass through so nothing
/// is silently mislabeled.
export function principalLabel(appId: string): string {
  return PRINCIPAL_LABELS[appId] ?? appId;
}

/// Plain plurals for the KG entity types a grant can reach. Unknown types
/// pass through unchanged.
const REACH_LABELS: Record<string, string> = {
  File: "Files",
  Project: "Projects",
  Event: "Activity events",
  Person: "People",
  Email: "Emails",
  Note: "Notes",
  Calendar: "Calendar",
};

/// A plain label for one reachable entity type.
export function reachLabel(type: string): string {
  return REACH_LABELS[type] ?? type;
}

/// Providers that run on the user's own machine, so there is no per-token
/// usage cost. Anything else is treated as a cloud provider.
const LOCAL_PROVIDERS = new Set(["ollama", "llama.cpp", "llamacpp", "local", "localai"]);

/// Whether the configured provider is local (no usage cost).
///
/// Matches the provider FAMILY, not an exact id: the configured provider is a
/// catalog id like `ollama-default`, not the bare kind `ollama`, so an exact-set
/// lookup missed it and rendered a local model as "a cloud service" with a cost -
/// a falsehood on the surface whose whole job is honesty. The family is the id's
/// leading segment before the first `-` (so `ollama-default` -> `ollama`), which
/// also matches a bare `ollama` and any future `ollama-<name>` local catalog id.
export function isLocalProvider(provider: string | null | undefined): boolean {
  if (provider == null) return false;
  const id = provider.toLowerCase();
  if (LOCAL_PROVIDERS.has(id)) return true;
  const family = id.split("-", 1)[0];
  return LOCAL_PROVIDERS.has(family);
}

/// Nicely cased display names for known providers; an unknown id gets its
/// first letter capitalised so a raw lowercase id never shows to a person.
const PROVIDER_NAMES: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  mistral: "Mistral",
  cohere: "Cohere",
};

/// A presentable provider name, or null when none is configured.
///
/// Matches the provider FAMILY as well as the exact id (the same catalog-id-vs-
/// bare-kind gap `isLocalProvider` had): a cloud id like `openai-gpt4` should
/// read "OpenAI", not "Openai-gpt4". A genuinely unknown provider keeps its FULL
/// id capitalised, so a user's custom `my-local-llm` is not truncated to "My".
export function providerDisplay(provider: string | null | undefined): string | null {
  if (!provider) return null;
  const id = provider.toLowerCase();
  const family = id.split("-", 1)[0];
  return (
    PROVIDER_NAMES[id] ??
    PROVIDER_NAMES[family] ??
    provider.charAt(0).toUpperCase() + provider.slice(1)
  );
}

/// The plain phrase for what a tier of reads means, matching the wording
/// the conversation surface uses. Unknown tiers return null.
const READS_PHRASES: Record<string, string> = {
  none: "nothing",
  metadata: "file names and dates, not what is inside files",
  structural: "file names and dates, not what is inside files",
  content: "the contents of your files",
  full: "the contents of your files",
};

/// The plain phrase for a declared-reads tier key, or null when unknown.
export function readsSentence(tier: string | null): string | null {
  if (!tier) return null;
  return READS_PHRASES[tier.toLowerCase()] ?? null;
}
