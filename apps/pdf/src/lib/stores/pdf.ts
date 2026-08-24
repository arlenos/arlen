/// The reader's data spine, carrying the page's wiring verbatim: six real
/// commands (`launch_file`, `pdf_open`, `pdf_search`, `pdf_page_image`,
/// `pdf_text_layer`, `pdf_page_text`), one document at a time, lazy per-page
/// rasters from the confined worker. Under plain vite a fixture document
/// stands in - three drawn placeholder pages, an outline, a text layer and a
/// searchable word - marked as an example on the surface, so the whole
/// reading surface is designable on a machine with no engine.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

export type OutlineEntry = { title: string; depth: number; page: number | null };
export type Hit = { page: number; snippet: string };
export type SearchOutcome = { hits: Hit[]; unsearchable: number[] };
export type DocumentInfo = { path: string; pages: number; outline: OutlineEntry[] };
export type PageImage = { width: number; height: number; rgba: number[] };
export type TextLine = { text: string; x: number; y: number; width: number; height: number };

/// What one page slot holds once its render answered.
export interface PageState {
  image: ImageData | null;
  lines: TextLine[];
  /// The render refusal, reported where the page would be.
  failure: string | null;
  /// The page's words when the picture could not be had (the engine-less path).
  words: string;
  /// The scale the current contents were fetched at.
  scale: number;
}

export const doc = writable<DocumentInfo | null>(null);
export const failure = writable<string | null>(null);
/// Separate from `failure`, which is about a document: this one is about not
/// learning which document there was.
export const launchFailure = writable<string | null>(null);
/// True while the document is the FIXTURE - the surface says so.
export const pdfMocked = writable(false);

// ---------------------------------------------------------------------------
// Fixture: three drawn pages. The painter mimics a typeset page closely
// enough to design against (title, rules, paragraph bars); the text layer and
// search carry one real sentence so selection and hits drive.
// ---------------------------------------------------------------------------

const FIXTURE_DOC: DocumentInfo = {
  path: "/home/example/reading/sample.pdf",
  pages: 3,
  outline: [
    { title: "Chapter one", depth: 0, page: 1 },
    { title: "Background", depth: 1, page: 1 },
    { title: "Method", depth: 0, page: 2 },
    { title: "Appendix (unlinked)", depth: 0, page: null },
  ],
};

const FIXTURE_SENTENCE = "Chapter one begins here with a needle in it";

/// Paint one fixture page into an ImageData (A4-ish ratio at the given scale).
function paintFixturePage(page: number, scale: number): ImageData {
  const w = Math.round(595 * scale);
  const h = Math.round(842 * scale);
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const ctx = c.getContext("2d");
  if (!ctx) return new ImageData(w, h);
  ctx.fillStyle = "#f7f5f0";
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "#1a1a1a";
  const m = 60 * scale;
  ctx.font = `bold ${26 * scale}px serif`;
  ctx.fillText(page === 1 ? "Chapter one" : page === 2 ? "Method" : "Notes", m, m + 26 * scale);
  ctx.fillRect(m, m + 40 * scale, w - 2 * m, 2 * scale);
  if (page === 1) {
    ctx.font = `${13 * scale}px serif`;
    ctx.fillText(FIXTURE_SENTENCE, m, m + 80 * scale);
  }
  // Paragraph bars: grey lines standing in for body text.
  ctx.fillStyle = "#c9c5bc";
  const lineH = 12 * scale;
  let y = m + (page === 1 ? 110 : 80) * scale;
  for (let i = 0; i < 34 && y < h - m; i++) {
    const wl = (i % 7 === 6 ? 0.55 : 0.82 + (i % 3) * 0.05) * (w - 2 * m);
    ctx.fillRect(m, y, wl, lineH * 0.55);
    y += lineH * 1.5;
  }
  return ctx.getImageData(0, 0, w, h);
}

function fixtureTextLayer(page: number, scale: number): TextLine[] {
  if (page !== 1) return [];
  return [
    { text: "Chapter one", x: 60 * scale, y: 60 * scale, width: 180 * scale, height: 28 * scale },
    { text: FIXTURE_SENTENCE, x: 60 * scale, y: 128 * scale, width: 330 * scale, height: 15 * scale },
  ];
}

// ---------------------------------------------------------------------------
// The real wiring, verbatim in behaviour.
// ---------------------------------------------------------------------------

/// Open on launch. A THROW AND A NULL ARE DIFFERENT ANSWERS: `null` means
/// nothing was passed on the command line, an ordinary bare start; a throw
/// means the host could not say what it was asked to open. Under vite the
/// fixture document opens so the surface is designable.
export async function openLaunched(): Promise<void> {
  if (!tauriAvailable) {
    doc.set(FIXTURE_DOC);
    pdfMocked.set(true);
    return;
  }
  let launched: string | null = null;
  try {
    launched = await invoke<string | null>("launch_file");
  } catch (e) {
    launchFailure.set(String(e));
    return;
  }
  if (!launched) return;
  try {
    doc.set(await invoke<DocumentInfo>("pdf_open", { path: launched }));
    failure.set(null);
  } catch (e) {
    failure.set(String(e));
  }
}

/// Fetch one page's raster and text layer at `scale`. A page that will not
/// render is reported where the page would be, with its words fetched via the
/// engine-less path; a text layer that will not come back leaves the drawn
/// page standing.
export async function fetchPage(page: number, scale: number): Promise<PageState> {
  if (!tauriAvailable) {
    return {
      image: paintFixturePage(page, scale),
      lines: fixtureTextLayer(page, scale),
      failure: null,
      words: "",
      scale,
    };
  }
  try {
    const img = await invoke<PageImage>("pdf_page_image", { page, scale });
    const image = new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height);
    const lines = await invoke<TextLine[]>("pdf_text_layer", { page, scale }).catch(() => []);
    return { image, lines, failure: null, words: "", scale };
  } catch (e) {
    const words = await invoke<string>("pdf_page_text", { page }).catch(() => "");
    // The backend answers with a token, not a sentence: `no-renderer` when this
    // machine has nothing to draw with, `refused` when it had and would not. An
    // unrecognised value is passed through as `refused` rather than shown, so a
    // future token cannot arrive on screen as a bare word.
    const token = String(e);
    const failure = token === "no-renderer" ? token : "refused";
    return { image: null, lines: [], failure, words, scale };
  }
}

/// Search the document. An empty box is not a query.
export async function search(query: string): Promise<SearchOutcome | null> {
  if (!query.trim()) return null;
  if (!tauriAvailable) {
    const q = query.trim().toLowerCase();
    return FIXTURE_SENTENCE.toLowerCase().includes(q)
      ? { hits: [{ page: 1, snippet: FIXTURE_SENTENCE }], unsearchable: [3] }
      : { hits: [], unsearchable: [3] };
  }
  return await invoke<SearchOutcome>("pdf_search", { query });
}
