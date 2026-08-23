/// The store catalogue: the one seam to `store-backend` (`org.arlen.Store1`,
/// store-app.md §9.4). The data model is the §9.1 merge: one card per AppStream
/// component-id, with per-source install VARIANTS - each variant carries its own
/// capability set, so "install the least-privilege variant" is a real choice.
/// Trust is a tier (curated vs community, §3), never the package format.
///
/// The types here are the BACKEND's wire shapes, verbatim (`view.rs` StoreCard,
/// camelCase). Capabilities arrive as identifiers (`network`, `filesystem`,
/// `read:File`), never prose - the prose lives in the message catalogue, because
/// the backend is not translated and this app is. A second frontend-only shape
/// is how the last mismatch happened, so there is none.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The mechanism that installs a variant. Visible ONLY in the install picker
/// (where the source is the choice); browse shows apps, never formats (§3).
export type Tier = "forage" | "flathub" | "debian" | "installed";
/// The trust tier (§3): curated/verified stays silent, community is flagged.
export type Trust = "curated" | "community";
/// What kind of item a card is; a bridge installs alongside the app it serves.
export type ItemKind = "app" | "bridge" | "module";
/// The source layer, resolution-precedence order. The wire spells the variant
/// names as Rust does.
export type SourceLayer = "Personal" | "Community" | "Official" | "Flatpak" | "Apt" | "Native";

/// One installable variant of an app (§9.1): its own source, trust and
/// capability footprint.
export interface StoreVariant {
  source: Tier;
  trust: Trust;
  /// Capability identifiers, sorted. Rendered into prose by the app.
  capabilities: string[];
  /// Least-privilege sort key: how many capabilities this variant asks for.
  capWeight: number;
  verified: boolean;
  reproducible: boolean;
  /// The version this source offers; empty when the source does not say.
  version: string;
  /// Whether the store has a route to install THIS variant.
  installable: boolean;
}

/// One merged catalogue card: display metadata once, N install variants. The
/// top-level capability fields describe the DEFAULT variant.
export interface StoreCard {
  id: string;
  name: string;
  summary: string;
  description: string | null;
  /// An icon reference: a URL, a local path, or a bare theme name. Only a URL
  /// is paintable in the webview today; the tile falls back honestly.
  icon: string | null;
  /// Screenshot URLs, in order.
  screenshots: string[];
  tier: Tier;
  kind: ItemKind;
  capabilities: string[];
  capWeight: number;
  noNetwork: boolean;
  offlineOnly: boolean;
  noGraph: boolean;
  verified: boolean;
  reproducible: boolean;
  installed: boolean;
  /// False for something the distribution installed or a card naming no
  /// package: the card is real and browsable, the action is not available.
  installable: boolean;
  variants: StoreVariant[];
  defaultVariant: number;
}

/// One layer's trust signals (§9.2). A field the layer does not attest is null
/// and its row is HIDDEN, never shown empty.
export interface TrustSignals {
  verified_publisher: string | null;
  reproducible_build: string | null;
  install_count: number | null;
  odrs_score: number | null;
  observed_vs_declared: string | null;
  attestation: { chain: string; signer: string; pinned_here: boolean } | null;
}

/// The per-layer trust resolution: one row per source layer that has signals.
export type LayerSignals = [SourceLayer, TrustSignals][];

/// What the store can honestly say about observed-vs-declared (§8.2).
/// "unavailable" and "measured with nothing observed" are different facts and
/// the surface keeps them apart.
export type ObservedStatus =
  | { state: "unavailable" }
  | { state: "measured"; declared: string[]; observed: string[]; windowDays: number };

/// One editorial collection (§8.7): the curator's own title per locale, the
/// member ids in the curator's order.
export interface Collection {
  id: string;
  titles: Record<string, string>;
  members: string[];
}

/// The variant the card speaks for in browse and the one Install takes by
/// default.
export function defaultVariantOf(card: StoreCard): StoreVariant | undefined {
  return card.variants[card.defaultVariant] ?? card.variants[0];
}

/// The app-level trust: what the badge on a card expresses.
export function trustOf(card: StoreCard): Trust {
  return defaultVariantOf(card)?.trust ?? "curated";
}

/// A collection's title for a locale, falling back to English - the curator
/// always writes that one.
export function collectionTitle(coll: Collection, locale: string): string {
  return coll.titles[locale] ?? coll.titles[locale.split("-")[0]] ?? coll.titles["en"] ?? coll.id;
}

/// The layer `store_install` should be told about for a CHOSEN variant, or
/// undefined for the default (the backend resolves that itself). The wire
/// variant carries only its tier, so two forage layers are told apart by
/// trust; if a card ever carries a personal AND an official recipe this guesses
/// official - flagged as a seam (the variant wants its layer on the wire).
export function layerFor(card: StoreCard, index: number): SourceLayer | undefined {
  if (index === card.defaultVariant) return undefined;
  const v = card.variants[index];
  if (!v) return undefined;
  if (v.source === "flathub") return "Flatpak";
  if (v.source === "debian") return "Apt";
  if (v.source === "installed") return "Native";
  return v.trust === "community" ? "Community" : "Official";
}

const g = (a: string, b: string) => `linear-gradient(135deg, ${a}, ${b})`;

/// The vite stand-in catalogue, in the exact wire shape. Icons and screenshots
/// are CSS gradients - the tile component recognises the notation - so the
/// fixture never pretends to be a real image pipeline.
const FIXTURE: StoreCard[] = [
  {
    id: "org.arlen.notes",
    name: "Quiet Notes",
    summary: "Plain notes that never leave the machine",
    description:
      "A small, fast notes app. Everything stays in one folder you choose; there is no sync, no account and no format lock-in - the notes are plain files.",
    icon: g("#1e3a5f", "#3b82a0"),
    screenshots: [g("#16324a", "#274e69"), g("#1b3a55", "#2f5a7d")],
    tier: "forage",
    kind: "app",
    capabilities: ["filesystem"],
    capWeight: 1,
    noNetwork: true,
    offlineOnly: true,
    noGraph: true,
    verified: true,
    reproducible: true,
    installed: true,
    installable: true,
    variants: [
      { source: "forage", trust: "curated", capabilities: ["filesystem"], capWeight: 1, verified: true, reproducible: true, version: "1.4.2", installable: true },
      { source: "flathub", trust: "community", capabilities: ["filesystem", "network"], capWeight: 2, verified: true, reproducible: false, version: "1.4.2", installable: true },
      { source: "debian", trust: "curated", capabilities: ["filesystem", "network"], capWeight: 2, verified: true, reproducible: true, version: "1.3.9", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.sketch",
    name: "Sketchbook",
    summary: "Freehand drawing with pressure support",
    description: "Layers, brushes and a canvas that keeps up with a pen. Files save as ORA.",
    icon: g("#4a1e5f", "#a03b8a"),
    screenshots: [g("#3d1b4f", "#6d2f7d"), g("#4a2159", "#8a3b9a")],
    tier: "flathub",
    kind: "app",
    capabilities: ["filesystem", "system"],
    capWeight: 2,
    noNetwork: true,
    offlineOnly: true,
    noGraph: true,
    verified: true,
    reproducible: false,
    installed: false,
    installable: true,
    variants: [
      { source: "flathub", trust: "community", capabilities: ["filesystem", "system"], capWeight: 2, verified: true, reproducible: false, version: "3.1.0", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.plot",
    name: "Plotline",
    summary: "Quick charts from CSV files",
    description: "Open a CSV, get a chart. Export as SVG or PNG.",
    icon: g("#1e5f3a", "#3ba06a"),
    screenshots: [g("#174a2e", "#27694a")],
    tier: "forage",
    kind: "app",
    capabilities: ["filesystem"],
    capWeight: 1,
    noNetwork: true,
    offlineOnly: true,
    noGraph: true,
    verified: false,
    reproducible: true,
    installed: false,
    installable: true,
    variants: [
      { source: "forage", trust: "community", capabilities: ["filesystem"], capWeight: 1, verified: false, reproducible: true, version: "0.9.1", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.stream",
    name: "Wavecast",
    summary: "Internet radio and podcasts",
    description: "A calm player for radio streams and podcast feeds you bring yourself.",
    icon: g("#5f3a1e", "#a06a3b"),
    screenshots: [g("#4a2e17", "#69452a")],
    tier: "flathub",
    kind: "app",
    capabilities: ["audio", "network"],
    capWeight: 2,
    noNetwork: false,
    offlineOnly: false,
    noGraph: true,
    verified: false,
    reproducible: false,
    installed: false,
    installable: true,
    variants: [
      { source: "flathub", trust: "community", capabilities: ["audio", "network"], capWeight: 2, verified: false, reproducible: false, version: "2.0.4", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.mail",
    name: "Postfach",
    summary: "A focused mail client",
    description: "IMAP and SMTP, threaded conversations, offline archive.",
    icon: g("#3a1e5f", "#6a3ba0"),
    screenshots: [g("#2e174a", "#452a69")],
    tier: "flathub",
    kind: "app",
    capabilities: ["filesystem", "network"],
    capWeight: 2,
    noNetwork: false,
    offlineOnly: false,
    noGraph: true,
    verified: true,
    reproducible: false,
    installed: false,
    installable: true,
    variants: [
      { source: "flathub", trust: "community", capabilities: ["filesystem", "network"], capWeight: 2, verified: true, reproducible: false, version: "5.2.0", installable: true },
      { source: "debian", trust: "curated", capabilities: ["filesystem", "network"], capWeight: 2, verified: true, reproducible: true, version: "5.1.3", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.timer",
    name: "Sandglass",
    summary: "Timers and focus sessions",
    description: "Countdowns, intervals and a gentle chime at the end.",
    icon: g("#5f1e2e", "#a03b52"),
    screenshots: [g("#4a1724", "#692a3a")],
    tier: "forage",
    kind: "app",
    capabilities: ["notifications"],
    capWeight: 1,
    noNetwork: true,
    offlineOnly: true,
    noGraph: true,
    verified: true,
    reproducible: true,
    installed: false,
    installable: true,
    variants: [
      { source: "forage", trust: "curated", capabilities: ["notifications"], capWeight: 1, verified: true, reproducible: true, version: "1.1.0", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.reader",
    name: "Leselampe",
    summary: "EPUB reading, comfortably",
    description: "Typography-first EPUB reading with margins that breathe.",
    icon: g("#1e4a5f", "#3b7ea0"),
    screenshots: [g("#173a4a", "#2a5569")],
    tier: "flathub",
    kind: "app",
    capabilities: ["filesystem"],
    capWeight: 1,
    noNetwork: true,
    offlineOnly: true,
    noGraph: true,
    verified: true,
    reproducible: false,
    installed: false,
    installable: true,
    variants: [
      { source: "flathub", trust: "community", capabilities: ["filesystem"], capWeight: 1, verified: true, reproducible: false, version: "0.8.2", installable: true },
    ],
    defaultVariant: 0,
  },
  {
    id: "org.example.weather",
    name: "Nimbus",
    summary: "Weather at a glance",
    description: "The next hours and days, from a single forecast source you can see.",
    icon: g("#2e3a5f", "#5a6aa0"),
    screenshots: [g("#24304a", "#3a4a69")],
    tier: "forage",
    kind: "app",
    capabilities: ["network"],
    capWeight: 1,
    noNetwork: false,
    offlineOnly: false,
    noGraph: true,
    verified: false,
    reproducible: true,
    installed: false,
    installable: true,
    variants: [
      { source: "forage", trust: "curated", capabilities: ["network"], capWeight: 1, verified: false, reproducible: true, version: "0.5.0", installable: true },
    ],
    defaultVariant: 0,
  },
];

/// The fixture collections, in the wire shape `store_collections` answers with.
const FIXTURE_COLLECTIONS: Collection[] = [
  {
    id: "essentials",
    titles: { en: "Essentials", de: "Grundausstattung" },
    members: ["org.arlen.notes", "org.example.mail", "org.example.reader"],
  },
  {
    id: "least-privilege",
    titles: { en: "Least-privilege picks", de: "Wenigste-Rechte-Auswahl" },
    members: ["org.example.timer", "org.example.plot", "org.example.sketch"],
  },
  {
    id: "fresh",
    titles: { en: "New this week", de: "Neu diese Woche" },
    members: ["org.example.weather", "org.example.stream"],
  },
];

const FIXTURE_TRUST: Record<string, LayerSignals> = {
  "org.arlen.notes": [
    [
      "Official",
      { verified_publisher: "cookbook", reproducible_build: "attested", install_count: 12400, odrs_score: null, observed_vs_declared: null, attestation: { chain: "tuf", signer: "arlen cookbook", pinned_here: true } },
    ],
    [
      "Flatpak",
      { verified_publisher: "flathub", reproducible_build: null, install_count: 89000, odrs_score: 4.4, observed_vs_declared: null, attestation: null },
    ],
  ],
  "org.example.mail": [
    [
      "Flatpak",
      { verified_publisher: "flathub", reproducible_build: null, install_count: 210000, odrs_score: 4.0, observed_vs_declared: null, attestation: null },
    ],
    [
      "Apt",
      { verified_publisher: "debian", reproducible_build: "attested", install_count: null, odrs_score: null, observed_vs_declared: null, attestation: null },
    ],
  ],
  "org.example.timer": [
    [
      "Official",
      { verified_publisher: "cookbook", reproducible_build: "attested", install_count: 7800, odrs_score: null, observed_vs_declared: null, attestation: { chain: "tuf", signer: "arlen cookbook", pinned_here: false } },
    ],
  ],
  "org.example.sketch": [
    ["Flatpak", { verified_publisher: "flathub", reproducible_build: null, install_count: 89000, odrs_score: 4.1, observed_vs_declared: null, attestation: null }],
  ],
  "org.example.stream": [
    ["Flatpak", { verified_publisher: null, reproducible_build: null, install_count: 45000, odrs_score: 3.8, observed_vs_declared: null, attestation: null }],
  ],
  "org.example.reader": [
    ["Flatpak", { verified_publisher: "flathub", reproducible_build: null, install_count: 56000, odrs_score: 4.3, observed_vs_declared: null, attestation: null }],
  ],
};

const FIXTURE_OBSERVED: Record<string, ObservedStatus> = {
  "org.arlen.notes": {
    state: "measured",
    declared: ["filesystem"],
    observed: ["filesystem"],
    windowDays: 92,
  },
};

/// The whole catalogue (fixture or live).
export const apps = writable<StoreCard[]>([]);
/// The editorial collections (fixture or live).
export const collections = writable<Collection[]>([]);
/// True while the catalogue is the FIXTURE - the surface says so, since install
/// decisions ride on it.
export const catalogMocked = writable(false);

/// Load the catalogue and the collections. Live: the composed catalog via
/// `store_search`, the curator's file via `store_collections`.
export async function loadCatalog(): Promise<void> {
  try {
    const list = await invoke<StoreCard[]>("store_search", { query: "", facets: [] });
    apps.set(list);
    catalogMocked.set(false);
  } catch {
    apps.set(FIXTURE);
    catalogMocked.set(true);
  }
  try {
    collections.set(await invoke<Collection[]>("store_collections"));
  } catch {
    collections.set(FIXTURE_COLLECTIONS);
  }
}

/// The per-layer trust signals for one app. Live: `store_trust_signals`.
export async function trustFor(id: string): Promise<LayerSignals> {
  try {
    return await invoke<LayerSignals>("store_trust_signals", { id });
  } catch {
    return FIXTURE_TRUST[id] ?? [];
  }
}

/// The local observed-vs-declared status for an app (§8.2). "Unavailable" is a
/// state of its own, never an empty panel.
export async function observedFor(id: string): Promise<ObservedStatus> {
  try {
    return await invoke<ObservedStatus>("store_observed_vs_declared", { id });
  } catch {
    return FIXTURE_OBSERVED[id] ?? { state: "unavailable" };
  }
}

/// Install the CHOSEN variant: resolves the install handoff (the consent
/// friction-ladder owns the real moment; a community variant rides the higher
/// rung there).
export async function installApp(id: string, variant?: SourceLayer): Promise<void> {
  try {
    await invoke("store_install", { id, variant: variant ?? null });
  } catch {
    // Seam unwired under vite; the consent ladder owns the real moment.
  }
}
