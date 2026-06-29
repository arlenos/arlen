/// Render an assistant answer (markdown) to sanitized HTML for display.
///
/// The on-device model emits markdown - lists, emphasis, fenced code - so
/// rendering it as plain text reads poorly. We parse it to HTML and then run
/// the result through DOMPurify, so a model that emits raw HTML, a `<script>`,
/// or an event-handler attribute cannot inject anything into the webview. Only
/// assistant turns go through this; the user's own input and error text stay
/// plain. The app runs SPA-only (`ssr = false`), so DOMPurify always has a real
/// DOM and this never runs server-side.
import { marked } from "marked";
import createDOMPurify from "dompurify";

/// DOMPurify must be bound to a DOM `window`. Bound lazily on first use (always
/// at runtime in the webview, or under happy-dom in tests), so importing this
/// module never touches `window` at load time. The default export's implicit
/// binding is unreliable outside a real browser (it falls back to a no-op
/// passthrough), which would silently disable sanitization, so we bind
/// explicitly.
let purifier: ReturnType<typeof createDOMPurify> | null = null;
function dompurify(): ReturnType<typeof createDOMPurify> {
  if (!purifier) {
    purifier = createDOMPurify(window);
  }
  return purifier;
}

/// Exactly the tags markdown needs to render. Anything else the model emits
/// (raw HTML, `<style>`, `<img>`/media, `<svg>`, forms/inputs/buttons) is
/// stripped. The default DOMPurify profile is too broad for untrusted LLM
/// output dropped into the webview with {@html}: it would let a prompt-injected
/// answer overlay fixed-position CSS over the app chrome or pull external/data
/// media, so this is an explicit allowlist rather than the default deny-list.
const ALLOWED_TAGS = [
  "p", "br", "hr", "blockquote",
  "strong", "em", "del",
  "code", "pre",
  "ul", "ol", "li",
  "h1", "h2", "h3", "h4", "h5", "h6",
  "a",
];

/// The only attribute kept: a link target. No `style`, no `target` (so a link
/// cannot reverse-tabnab), no event handlers, no `class`/`id`.
const ALLOWED_ATTR = ["href"];

/// Link schemes the renderer trusts. `javascript:` and `data:` are excluded, so
/// a crafted link cannot execute script or smuggle a data payload; a link with
/// any other scheme keeps its text but loses its href (inert). http(s) must
/// carry `//` (a bare `https:foo` is not a real navigable link), and this must
/// stay in lockstep with the backend `open_url` allowlist that the click
/// handler forwards to, so a rendered link never opens to a silent rejection.
///
/// `arlenfile:` is the file-reference scheme: the agent names a file it touched
/// as `[name](arlenfile:///abs/path)`, and the `fileRefs` action upgrades that
/// inert anchor into a clickable file pill (opened AS THE USER via the portal,
/// never the AI daemon). Kept here so the anchor survives sanitization; the
/// action intercepts the click so it never navigates the webview.
const ALLOWED_URI_REGEXP = /^(?:https?:\/\/|mailto:|arlenfile:\/\/)/i;

/// Parse `text` as GitHub-flavoured markdown and return strictly-sanitized HTML.
export function renderMarkdown(text: string): string {
  // Synchronous parse (no async marked extensions are registered). `breaks`
  // turns a single newline into <br> so model output that relies on line
  // breaks reads as written.
  const html = marked.parse(text, { gfm: true, breaks: true }) as string;
  return dompurify().sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    ALLOWED_URI_REGEXP,
  });
}
