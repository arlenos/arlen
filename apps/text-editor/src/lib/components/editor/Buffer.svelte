<script lang="ts">
  /// The editable buffer: CodeMirror 6 over the open file.
  ///
  /// WHY THIS EXISTS. Until 16 August this app had no editing surface at all.
  /// `Canvas.svelte` takes its document as a read-only prop and renders it as
  /// styled segments - its own comment says the real engine "is the coder's" - and
  /// the host's `editor_save` command, which writes through a temp file and an
  /// atomic rename, had no caller anywhere in the frontend. So the text editor
  /// opened a file, showed it beautifully, and could neither change it nor write
  /// it back. `text-editor-app.md` assigns the coder "the editor engine
  /// (buffer/save/find)" and the jobs entry names CodeMirror 6; this is that.
  ///
  /// Canvas stays for the markdown reading stance (the iA-Writer surface with the
  /// focus mode); this is the surface you type into. Which one a file gets is the
  /// caller's decision, not this component's.
  import { onMount } from "svelte";
  import { EditorState, type Extension } from "@codemirror/state";
  import { EditorView, keymap, lineNumbers, highlightActiveLine } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
  import { syntaxHighlighting, defaultHighlightStyle, StreamLanguage } from "@codemirror/language";

  let {
    doc = "",
    language = "text",
    onchange,
    onsave,
  }: {
    /// The file's contents as opened. Replacing it rebuilds the buffer, so the
    /// caller must not feed back what `onchange` reports - that would reset the
    /// cursor on every keystroke.
    doc?: string;
    /// Which highlighting to load. `text` is no highlighting at all rather than a
    /// guess: colouring a file by the wrong grammar is worse than not colouring it.
    language?: "text" | "markdown" | "rust" | "javascript";
    /// Every document change, with the full text.
    onchange?: (text: string) => void;
    /// Ctrl+S / Cmd+S. The component does not save - it says the user asked, and
    /// the window decides what saving means.
    onsave?: () => void;
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  /// Grammar extensions, loaded on demand so a plain text file pays for nothing.
  async function grammar(lang: string): Promise<Extension[]> {
    if (lang === "markdown") {
      const { markdown } = await import("@codemirror/lang-markdown");
      return [markdown()];
    }
    if (lang === "rust") {
      const { rust } = await import("@codemirror/lang-rust");
      return [rust()];
    }
    if (lang === "javascript") {
      const { javascript } = await import("@codemirror/lang-javascript");
      return [javascript()];
    }
    return [];
  }

  /// The editor's own theme, bound to the Arlen tokens rather than CodeMirror's
  /// defaults, so the buffer is the same surface as the rest of the window.
  const theme = EditorView.theme(
    {
      "&": {
        height: "100%",
        backgroundColor: "transparent",
        color: "var(--color-fg-primary, #fafafa)",
        fontSize: "13.5px",
      },
      ".cm-content": { fontFamily: "var(--font-mono, ui-monospace, monospace)", padding: "12px 0" },
      ".cm-gutters": {
        backgroundColor: "transparent",
        border: "none",
        color: "color-mix(in srgb, var(--color-fg-primary, #fafafa) 35%, transparent)",
      },
      ".cm-activeLine": {
        backgroundColor: "color-mix(in srgb, var(--color-fg-primary, #fafafa) 5%, transparent)",
      },
      ".cm-cursor": { borderLeftColor: "var(--color-accent, #6366f1)" },
      "&.cm-focused": { outline: "none" },
    },
    { dark: true },
  );

  onMount(() => {
    let alive = true;
    let cleanup: (() => void) | undefined;
    (async () => {
      const lang = await grammar(language);
      if (!alive) return;
      view = new EditorView({
        parent: host,
        state: EditorState.create({
          doc,
          extensions: [
            lineNumbers(),
            history(),
            highlightActiveLine(),
            highlightSelectionMatches(),
            syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
            ...lang,
            // Save first in the list so Ctrl+S reaches the window rather than the
            // browser's own save dialog.
            keymap.of([
              {
                key: "Mod-s",
                preventDefault: true,
                run: () => {
                  onsave?.();
                  return true;
                },
              },
              ...defaultKeymap,
              ...historyKeymap,
              ...searchKeymap,
              indentWithTab,
            ]),
            EditorView.updateListener.of((u) => {
              if (u.docChanged) onchange?.(u.state.doc.toString());
            }),
            theme,
            EditorView.lineWrapping,
          ],
        }),
      });
      cleanup = () => view?.destroy();
    })();
    return () => {
      alive = false;
      cleanup?.();
    };
  });
</script>

<div class="buffer" bind:this={host}></div>

<style>
  .buffer {
    height: 100%;
    overflow: auto;
  }
</style>
