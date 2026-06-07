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
import DOMPurify from "dompurify";

/// Parse `text` as GitHub-flavoured markdown and return sanitized HTML.
export function renderMarkdown(text: string): string {
  // Synchronous parse (no async marked extensions are registered). `breaks`
  // turns a single newline into <br> so model output that relies on line
  // breaks reads as written.
  const html = marked.parse(text, { gfm: true, breaks: true }) as string;
  // Default DOMPurify config drops <script>, event-handler attributes, and
  // other active content while keeping the formatting tags markdown produces.
  return DOMPurify.sanitize(html);
}
