/// The store catalogue: the one seam to `store-backend` (`org.arlen.Store1`,
/// store-app.md §9.4). The data model is the §9.1 merge: one card per AppStream
/// component-id, with per-source install VARIANTS - each variant carries its own
/// capability set, so "install the least-privilege variant" is a real choice.
/// Trust is a tier (curated vs community, §3), never the package format.
///
/// Live, the Tauri commands proxy the session socket ops (`store_search`,
/// `store_app_detail`, `store_trust_signals`, `store_install`,
/// `store_observed_vs_declared`) - all coder seams. Under vite a fixture
/// catalogue stands in so browse, facets, the variant picker and the app page
/// all render and drive. Capability lines arrive as composed plain-language rows
/// from the backend - data, not UI copy.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The package layer a variant comes from. Visible ONLY in the install picker
/// (where the source is the choice); browse shows apps, never formats (§3).
export type Source = "forage" | "flathub" | "debian";
/// The trust tier (§3): curated/verified stays silent, community is flagged.
export type Trust = "curated" | "community";

/// One plain-language capability row; negatives ("Cannot reach the network")
/// carry the weight of the least-privilege story.
export interface CapLine {
  text: string;
  negative: boolean;
}

/// One installable variant of an app (§9.1): its own source, trust and
/// capability footprint.
export interface Variant {
  source: Source;
  trust: Trust;
  caps: CapLine[];
  /// Sort key for least-privilege ordering (fewer/narrower is less).
  capWeight: number;
  installed: boolean;
  verified: boolean;
  reproducible: boolean;
  /// A `.deb` brought under confinement by the apt-enroll hook.
  enrolledDeb?: boolean;
}

/// One merged catalogue card: display metadata once, N install variants.
export interface StoreApp {
  id: string;
  name: string;
  summary: string;
  /// A CSS background for the icon tile (live: the AppStream icon URL).
  icon: string;
  variants: Variant[];
  /// Index of the default variant (the forage resolution precedence).
  defaultVariant: number;
  description?: string;
  /// Screenshot backgrounds (live: AppStream screenshot URLs).
  shots?: string[];
  /// The developer's own donation page; absent = no affordance at all.
  donationUrl?: string | null;
}

/// The trust panel's signals (ODRS is one quiet row, never the headline).
export interface TrustSignals {
  reproducible: boolean;
  verifiedPublisher: boolean;
  installCount: number | null;
  odrsRating: number | null;
}

/// The local observed-vs-declared read (§8.2); "not observed", never "safe".
export interface ObservedLine {
  text: string;
}

/// The variant the card speaks for in browse (facets, sort, summary caps).
export function defaultVariantOf(app: StoreApp): Variant {
  return app.variants[app.defaultVariant] ?? app.variants[0];
}

/// An app is installed when any of its variants is.
export function isInstalled(app: StoreApp): boolean {
  return app.variants.some((v) => v.installed);
}

/// The app-level trust: community when its default variant is community.
export function trustOf(app: StoreApp): Trust {
  return defaultVariantOf(app).trust;
}

/// Facet flags, derived from the default variant's capability lines.
export function facetFlags(app: StoreApp): {
  noNetwork: boolean;
  offlineOnly: boolean;
  noGraph: boolean;
  verified: boolean;
  reproducible: boolean;
} {
  const v = defaultVariantOf(app);
  const has = (s: string) => v.caps.some((c) => c.negative && c.text.toLowerCase().includes(s));
  const noNetwork = has("network");
  return {
    noNetwork,
    offlineOnly: noNetwork,
    noGraph: has("knowledge graph") || has("graph"),
    verified: v.verified,
    reproducible: v.reproducible,
  };
}

const g = (a: string, b: string) => `linear-gradient(135deg, ${a}, ${b})`;

const NOTES_FORAGE: CapLine[] = [
  { text: "Reads and writes its own notes folder", negative: false },
  { text: "Cannot reach the network", negative: true },
  { text: "Cannot read the Knowledge Graph", negative: true },
];
const NOTES_FLATHUB: CapLine[] = [
  { text: "Reads and writes its own notes folder", negative: false },
  { text: "Checks for updates over the network", negative: false },
  { text: "Cannot read the Knowledge Graph", negative: true },
];
const NOTES_DEBIAN: CapLine[] = [
  { text: "Reads and writes your home folder", negative: false },
  { text: "Talks to the network", negative: false },
  { text: "Cannot read the Knowledge Graph", negative: true },
];

const FIXTURE: StoreApp[] = [
  {
    id: "org.arlen.notes",
    name: "Quiet Notes",
    summary: "Plain notes that never leave the machine",
    icon: g("#1e3a5f", "#3b82a0"),
    variants: [
      { source: "forage", trust: "curated", caps: NOTES_FORAGE, capWeight: 1, installed: true, verified: true, reproducible: true },
      { source: "flathub", trust: "curated", caps: NOTES_FLATHUB, capWeight: 2, installed: false, verified: true, reproducible: false },
      { source: "debian", trust: "curated", caps: NOTES_DEBIAN, capWeight: 4, installed: false, verified: true, reproducible: true, enrolledDeb: true },
    ],
    defaultVariant: 0,
    description:
      "A small, fast notes app. Everything stays in one folder you choose; there is no sync, no account and no format lock-in - the notes are plain files.",
    shots: [g("#16324a", "#274e69"), g("#1b3a55", "#2f5a7d")],
    donationUrl: "https://liberapay.com/quietnotes",
  },
  {
    id: "org.example.sketch",
    name: "Sketchbook",
    summary: "Freehand drawing with pressure support",
    icon: g("#4a1e5f", "#a03b8a"),
    variants: [
      {
        source: "flathub",
        trust: "curated",
        caps: [
          { text: "Reads and writes your Pictures folder", negative: false },
          { text: "Uses your drawing tablet", negative: false },
          { text: "Cannot reach the network", negative: true },
        ],
        capWeight: 2,
        installed: false,
        verified: true,
        reproducible: false,
      },
    ],
    defaultVariant: 0,
    description: "Layers, brushes and a canvas that keeps up with a pen. Files save as ORA.",
    shots: [g("#3d1b4f", "#6d2f7d"), g("#4a2159", "#8a3b9a")],
    donationUrl: null,
  },
  {
    id: "org.example.plot",
    name: "Plotline",
    summary: "Quick charts from CSV files",
    icon: g("#1e5f3a", "#3ba06a"),
    variants: [
      {
        source: "forage",
        trust: "community",
        caps: [
          { text: "Reads files you open with it", negative: false },
          { text: "Cannot reach the network", negative: true },
          { text: "Cannot read the Knowledge Graph", negative: true },
        ],
        capWeight: 1,
        installed: false,
        verified: false,
        reproducible: true,
      },
    ],
    defaultVariant: 0,
    description: "Open a CSV, get a chart. Export as SVG or PNG.",
    shots: [g("#174a2e", "#27694a")],
    donationUrl: "https://github.com/sponsors/plotline",
  },
  {
    id: "org.example.stream",
    name: "Wavecast",
    summary: "Internet radio and podcasts",
    icon: g("#5f3a1e", "#a06a3b"),
    variants: [
      {
        source: "flathub",
        trust: "community",
        caps: [
          { text: "Talks to the stations you add", negative: false },
          { text: "Plays audio", negative: false },
          { text: "Cannot read your files", negative: true },
        ],
        capWeight: 3,
        installed: false,
        verified: false,
        reproducible: false,
      },
    ],
    defaultVariant: 0,
    description: "A calm player for radio streams and podcast feeds you bring yourself.",
    shots: [g("#4a2e17", "#69452a")],
    donationUrl: null,
  },
  {
    id: "org.example.mail",
    name: "Postfach",
    summary: "A focused mail client",
    icon: g("#3a1e5f", "#6a3ba0"),
    variants: [
      {
        source: "flathub",
        trust: "curated",
        caps: [
          { text: "Talks to your mail server", negative: false },
          { text: "Reads and writes its own mail store", negative: false },
          { text: "Cannot read the Knowledge Graph", negative: true },
        ],
        capWeight: 3,
        installed: false,
        verified: true,
        reproducible: false,
      },
      {
        source: "debian",
        trust: "curated",
        caps: [
          { text: "Talks to your mail server", negative: false },
          { text: "Reads and writes its own mail store", negative: false },
          { text: "Cannot read the Knowledge Graph", negative: true },
        ],
        capWeight: 4,
        installed: false,
        verified: true,
        reproducible: true,
        enrolledDeb: true,
      },
    ],
    defaultVariant: 0,
    description: "IMAP and SMTP, threaded conversations, offline archive.",
    shots: [g("#2e174a", "#452a69")],
    donationUrl: null,
  },
  {
    id: "org.example.timer",
    name: "Sandglass",
    summary: "Timers and focus sessions",
    icon: g("#5f1e2e", "#a03b52"),
    variants: [
      {
        source: "forage",
        trust: "curated",
        caps: [
          { text: "Shows notifications", negative: false },
          { text: "Cannot reach the network", negative: true },
          { text: "Cannot read your files", negative: true },
        ],
        capWeight: 1,
        installed: false,
        verified: true,
        reproducible: true,
      },
    ],
    defaultVariant: 0,
    description: "Countdowns, intervals and a gentle chime at the end.",
    shots: [g("#4a1724", "#692a3a")],
    donationUrl: "https://liberapay.com/sandglass",
  },
  {
    id: "org.example.reader",
    name: "Leselampe",
    summary: "EPUB reading, comfortably",
    icon: g("#1e4a5f", "#3b7ea0"),
    variants: [
      {
        source: "flathub",
        trust: "curated",
        caps: [
          { text: "Reads books you open with it", negative: false },
          { text: "Cannot reach the network", negative: true },
        ],
        capWeight: 1,
        installed: false,
        verified: true,
        reproducible: false,
      },
    ],
    defaultVariant: 0,
    description: "Typography-first EPUB reading with margins that breathe.",
    shots: [g("#173a4a", "#2a5569")],
    donationUrl: null,
  },
  {
    id: "org.example.weather",
    name: "Nimbus",
    summary: "Weather at a glance",
    icon: g("#2e3a5f", "#5a6aa0"),
    variants: [
      {
        source: "forage",
        trust: "curated",
        caps: [
          { text: "Talks to api.met.no:443", negative: false },
          { text: "Cannot read your files", negative: true },
          { text: "Cannot read the Knowledge Graph", negative: true },
        ],
        capWeight: 2,
        installed: false,
        verified: false,
        reproducible: true,
      },
    ],
    defaultVariant: 0,
    description: "The next hours and days, from a single forecast source you can see.",
    shots: [g("#24304a", "#3a4a69")],
    donationUrl: null,
  },
];

/// The editorial collections (§8.7): hand-picked ids, never algorithmic.
export const COLLECTIONS: { labelKey: string; ids: string[] }[] = [
  { labelKey: "st.coll.essentials", ids: ["org.arlen.notes", "org.example.mail", "org.example.reader"] },
  { labelKey: "st.coll.leastPrivilege", ids: ["org.example.timer", "org.example.plot", "org.example.sketch"] },
  { labelKey: "st.coll.fresh", ids: ["org.example.weather", "org.example.stream"] },
];

const FIXTURE_TRUST: Record<string, TrustSignals> = {
  "org.arlen.notes": { reproducible: true, verifiedPublisher: true, installCount: 12400, odrsRating: 4.4 },
  "org.example.sketch": { reproducible: false, verifiedPublisher: true, installCount: 89000, odrsRating: 4.1 },
  "org.example.plot": { reproducible: true, verifiedPublisher: false, installCount: 3100, odrsRating: null },
  "org.example.stream": { reproducible: false, verifiedPublisher: false, installCount: 45000, odrsRating: 3.8 },
  "org.example.mail": { reproducible: true, verifiedPublisher: true, installCount: 210000, odrsRating: 4.0 },
  "org.example.timer": { reproducible: true, verifiedPublisher: true, installCount: 7800, odrsRating: 4.6 },
  "org.example.reader": { reproducible: false, verifiedPublisher: true, installCount: 56000, odrsRating: 4.3 },
  "org.example.weather": { reproducible: true, verifiedPublisher: false, installCount: 15600, odrsRating: null },
};

const FIXTURE_OBSERVED: Record<string, ObservedLine[]> = {
  "org.arlen.notes": [
    { text: "Declared capabilities match what it has used." },
    { text: "Has never tried to reach the network on your machine." },
  ],
};

/// The whole catalogue (fixture or live).
export const apps = writable<StoreApp[]>([]);
/// True while the catalogue is the FIXTURE - the surface says so, since install
/// decisions ride on it.
export const catalogMocked = writable(false);

/// Load the catalogue. Live: the composed catalog via `store_search`.
export async function loadCatalog(): Promise<void> {
  try {
    const list = await invoke<StoreApp[]>("store_search", { query: "", facets: [] });
    apps.set(list);
    catalogMocked.set(false);
  } catch {
    apps.set(FIXTURE);
    catalogMocked.set(true);
  }
}

/// The trust signals for one app. Live: `store_trust_signals`.
export async function trustFor(id: string): Promise<TrustSignals | null> {
  try {
    return await invoke<TrustSignals>("store_trust_signals", { id });
  } catch {
    return FIXTURE_TRUST[id] ?? null;
  }
}

/// The local observed-vs-declared lines for an INSTALLED app. Live:
/// `store_observed_vs_declared` reads the user's own audit ledger.
export async function observedFor(id: string): Promise<ObservedLine[]> {
  try {
    return await invoke<ObservedLine[]>("store_observed_vs_declared", { id });
  } catch {
    return FIXTURE_OBSERVED[id] ?? [];
  }
}

/// Install the CHOSEN variant: hands off to the consent friction-ladder (a
/// community variant rides the higher rung there).
export async function installApp(id: string, source: Source): Promise<void> {
  try {
    await invoke("store_install", { id, variant: source });
  } catch {
    // Seam unwired under vite; the consent ladder owns the real moment.
  }
}
