/// Svelte action for rendered-markdown containers: upgrade the agent's inert
/// file-reference anchors (`[name](arlenfile:///abs/path)`, kept by the markdown
/// sanitizer) into clickable file pills. A left-click opens the file AS THE USER
/// through the desktop opener (`fileref_open`, routed to the portal, NOT the AI
/// daemon - the powerbox seam: the authority to open is the user's click, not
/// the agent's read slice). A right-click opens the shared pill menu. A path the
/// opener cannot resolve is marked muted ("not found"), not a dead link.
///
/// Mirrors `externalLinks`: the click/contextmenu handlers are delegated on the
/// container, so they survive prose re-renders; the per-anchor decoration (icon
/// + class + resolvable check) re-runs whenever the message text changes (the
/// action argument), because `{@html}` rebuilds the inner DOM on each render.

import { invoke } from "@tauri-apps/api/core";
import { mount, unmount } from "svelte";
import { entryIcon } from "@arlen/ui-kit/components/browser";
import { openFileRefMenu } from "$lib/stores/fileRefMenu";

const PREFIX = "arlenfile://";

/// The absolute path an `arlenfile:///abs/path` href points at.
function refPath(href: string): string {
  const raw = href.slice(PREFIX.length);
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/// The trailing name of a path, for the pill label when the anchor has no text.
function basename(p: string): string {
  const trimmed = p.replace(/\/+$/, "");
  const i = trimmed.lastIndexOf("/");
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

/// `node` is the rendered-markdown container; `_text` is the message text, passed
/// only so the action's `update` fires when the prose re-renders.
export function fileRefs(node: HTMLElement, _text?: string) {
  let icons: Record<string, unknown>[] = [];

  function decorate(): void {
    // `{@html}` rebuilt the inner DOM, so the previous icon instances are
    // detached - unmount them before re-decorating the fresh anchors.
    for (const inst of icons) unmount(inst);
    icons = [];

    const anchors = node.querySelectorAll<HTMLAnchorElement>('a[href^="arlenfile://"]');
    const paths: string[] = [];
    for (const a of anchors) {
      const path = refPath(a.getAttribute("href") ?? "");
      const name = a.textContent?.trim() || basename(path);
      a.classList.add("fileref");
      a.setAttribute("title", path);
      a.dataset.path = path;
      a.dataset.name = name;

      const host = document.createElement("span");
      host.className = "fileref-icon";
      a.prepend(host);
      const Icon = entryIcon({ name, kind: "file" });
      icons.push(mount(Icon, { target: host, props: { size: 13, strokeWidth: 2 } }));
      paths.push(path);
    }

    if (paths.length === 0) return;
    // Resolvability is a backend question (the opener's MIME/path check); an
    // unreachable command leaves every pill clickable rather than fake-muted.
    invoke<{ path: string; resolvable: boolean }[]>("fileref_resolve", { paths })
      .then((rows) => {
        const missing = new Set(rows.filter((r) => !r.resolvable).map((r) => r.path));
        for (const a of node.querySelectorAll<HTMLAnchorElement>("a.fileref")) {
          if (missing.has(a.dataset.path ?? "")) {
            a.classList.add("missing");
            a.setAttribute("title", `${a.dataset.path} (not found)`);
          }
        }
      })
      .catch(() => {});
  }

  function onClick(event: MouseEvent): void {
    const a = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>("a.fileref");
    if (!a) return;
    event.preventDefault();
    if (a.classList.contains("missing")) return;
    invoke("fileref_open", { path: a.dataset.path }).catch((err) =>
      console.error("fileref_open failed", err),
    );
  }

  function onContext(event: MouseEvent): void {
    const a = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>("a.fileref");
    if (!a) return;
    event.preventDefault();
    openFileRefMenu({
      x: event.clientX,
      y: event.clientY,
      path: a.dataset.path ?? "",
      name: a.dataset.name ?? "",
      resolvable: !a.classList.contains("missing"),
    });
  }

  decorate();
  node.addEventListener("click", onClick);
  node.addEventListener("contextmenu", onContext);

  return {
    update() {
      decorate();
    },
    destroy() {
      node.removeEventListener("click", onClick);
      node.removeEventListener("contextmenu", onContext);
      for (const inst of icons) unmount(inst);
      icons = [];
    },
  };
}
