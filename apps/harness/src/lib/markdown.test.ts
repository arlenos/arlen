import { describe, it, expect } from "vitest";

import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("renders the formatting markdown produces", () => {
    const html = renderMarkdown("**bold** and `code`\n\n- a\n- b");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<code>code</code>");
    expect(html).toContain("<li>a</li>");
  });

  it("keeps an https link's href but no target (no reverse-tabnabbing)", () => {
    const html = renderMarkdown("[ok](https://example.com)");
    expect(html).toContain('href="https://example.com"');
    expect(html.toLowerCase()).not.toContain("target=");
  });

  it("accepts a link's scheme case-insensitively", () => {
    // Matches the backend open_url allowlist, which also compares
    // case-insensitively, so a rendered link is never rejected on open.
    const html = renderMarkdown("[ok](HTTPS://example.com)");
    expect(html).toContain('href="HTTPS://example.com"');
  });

  it("drops a bare http(s) scheme without //", () => {
    // `https:foo` is not a navigable link; both layers require the slashes.
    const html = renderMarkdown("[x](https:no-slashes)");
    expect(html).not.toContain("href");
  });

  // Security regression cases: untrusted model output must not inject active
  // content, overlay CSS, or smuggle data/script through the {@html} sink.
  it("strips <script>", () => {
    expect(renderMarkdown("hi <script>alert(1)</script>")).not.toContain("<script");
  });

  it("strips <style> and style attributes", () => {
    const html = renderMarkdown(
      '<style>body{display:none}</style><p style="position:fixed;top:0">x</p>',
    );
    expect(html).not.toContain("<style");
    expect(html).not.toContain("position:fixed");
    expect(html.toLowerCase()).not.toContain("style=");
  });

  it("drops javascript: links", () => {
    const html = renderMarkdown("[click](javascript:alert(1))");
    expect(html).not.toContain("javascript:");
  });

  it("strips images and data: media", () => {
    const html = renderMarkdown("![x](data:image/png;base64,AAAA)");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("data:");
  });

  it("strips form controls", () => {
    const html = renderMarkdown("<form><input value=x><button>go</button></form>");
    expect(html).not.toMatch(/<(form|input|button)/);
  });

  it("strips inline event-handler attributes", () => {
    const html = renderMarkdown('<a href="https://x.example" onclick="alert(1)">y</a>');
    expect(html.toLowerCase()).not.toContain("onclick");
  });
});
