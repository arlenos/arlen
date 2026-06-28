/// Frontend mirror of the locked `contracts/artifact` wire shape (the crate's
/// `Artifact` / `ArtifactPayload` / `ArtifactKind` / `ArtifactMeta`). This is a
/// STOPGAP: the coder adds ts-rs bindings to `contracts/artifact`, after which
/// these hand-written types are replaced by the generated ones. Kept byte-exact
/// to the Rust serde tags (snake_case, payload tagged on `kind`) so a swap is
/// mechanical. The renderers consume this; nothing here grants any authority
/// (origin is backend-stamped, never trusted from a payload).

/// The closed set of artifact kinds. `widget` is a separate trust class with no
/// program-constructible payload, so the inert renderers never see it.
export type ArtifactKind =
  | "markdown"
  | "code"
  | "table"
  | "chart"
  | "image"
  | "diagram"
  | "links"
  | "widget";

/// Where the bytes came from. Backend-stamped from the channel, never inferred
/// from the payload (the iTerm2 CVE-2026-41253 fix); cosmetic to the renderer.
export type ArtifactOrigin = "external_content" | "agent_generated" | "system_trusted";

/// A closed chart type, so a spec can never carry a renderer directive.
export type ChartType = "line" | "bar" | "scatter" | "pie" | "area";

/// A closed image media type. `svg` may carry script, so it is rendered through
/// an `<img>` data URI (which never executes embedded script).
export type ImageMediaType = "png" | "jpeg" | "svg" | "webp" | "gif";

/// A closed diagram source language.
export type DiagramLanguage = "mermaid" | "dot";

/// One labelled numeric series of a chart (plain numbers, never an expression).
export interface Series {
  name: string;
  values: number[];
}

/// A single link; the renderer applies its own scheme allow-list.
export interface Link {
  href: string;
  label?: string;
}

/// The typed body, tagged on `kind` (matches the Rust `#[serde(tag = "kind")]`).
export type ArtifactPayload =
  | { kind: "markdown"; source: string }
  | { kind: "code"; source: string; language?: string }
  | { kind: "table"; columns: string[]; rows: string[][] }
  | { kind: "chart"; chart_type: ChartType; series: Series[] }
  | { kind: "image"; media_type: ImageMediaType; data_base64: string }
  | { kind: "diagram"; language: DiagramLanguage; source: string }
  | { kind: "links"; links: Link[] };

/// Backend-stamped metadata. `origin` is authoritative; `title` is cosmetic.
export interface ArtifactMeta {
  origin: ArtifactOrigin;
  title?: string;
}

/// A unified rich-object artifact: a typed payload, a mandatory plain-text floor
/// (`text`, the pipe floor a dumb terminal or a copy sees), and metadata.
export interface Artifact {
  kind: ArtifactKind;
  payload: ArtifactPayload;
  text: string;
  meta: ArtifactMeta;
}

/// The `<img>` media-type string for an image payload's closed media type.
export function imageMime(t: ImageMediaType): string {
  switch (t) {
    case "png":
      return "image/png";
    case "jpeg":
      return "image/jpeg";
    case "svg":
      return "image/svg+xml";
    case "webp":
      return "image/webp";
    case "gif":
      return "image/gif";
  }
}

/// A short human label for a kind, for the card badge + panel header.
export function kindLabel(kind: ArtifactKind): string {
  switch (kind) {
    case "markdown":
      return "Document";
    case "code":
      return "Code";
    case "table":
      return "Table";
    case "chart":
      return "Chart";
    case "image":
      return "Image";
    case "diagram":
      return "Diagram";
    case "links":
      return "Links";
    case "widget":
      return "Widget";
  }
}
