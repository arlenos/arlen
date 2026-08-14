<script lang="ts">
  /// Headless render harness for the terminal grid (coder-jobs PR-2: the render
  /// bugs are frontend, verify by rendering GridRegion with an injected snapshot
  /// in a headless browser + screenshotting - no wry/PTY needed). Mounts
  /// GridRegion with a representative neofetch-like fixture (coloured SGR cells,
  /// a full ANSI palette row, aligned columns) so colour + fixed-width alignment
  /// are directly visible. Not shipped in any nav; a dev/test route only.
  import { onMount, tick } from "svelte";
  import { GridRegion } from "@arlen/ui-kit/components/console";
  import StreamBlock from "$lib/components/StreamBlock.svelte";
  import HistoryPalette from "$lib/components/HistoryPalette.svelte";
  import Composer from "$lib/components/Composer.svelte";
  import { newSession } from "$lib/stores/sessions";
  import StreamEmpty from "$lib/components/StreamEmpty.svelte";
  import { historyPaletteOpen } from "$lib/stores/history";
  import { tauriAvailable } from "$lib/tauri";
  import type { Block } from "$lib/contract";

  type CellColor =
    | { kind: "default" }
    | { kind: "indexed"; value: number }
    | { kind: "rgb"; value: [number, number, number] };
  type GridCell = {
    text: string;
    fg: CellColor;
    bg: CellColor;
    bold: boolean;
    italic: boolean;
    underline: boolean;
    inverse: boolean;
    wide: boolean;
  };

  const COLS = 72;
  const def: CellColor = { kind: "default" };
  const idx = (n: number): CellColor => ({ kind: "indexed", value: n });

  function blank(): GridCell {
    return { text: "", fg: def, bg: def, bold: false, italic: false, underline: false, inverse: false, wide: false };
  }
  function put(row: GridCell[], at: number, ch: string, fg: CellColor, bold = false, bg: CellColor = def) {
    for (let i = 0; i < ch.length && at + i < COLS; i++) {
      row[at + i] = { text: ch[i], fg, bg, bold, italic: false, underline: false, inverse: false, wide: false };
    }
  }
  function emptyRow(): GridCell[] {
    return Array.from({ length: COLS }, blank);
  }
  // A "key: value" line: key in default, value in a colour - tests that colour
  // starts mid-row and columns to the right stay aligned.
  function kv(key: string, value: string, vcol: CellColor): GridCell[] {
    const r = emptyRow();
    put(r, 0, key, def, true);
    put(r, 12, value, vcol);
    return r;
  }
  // The 16 base ANSI colours as solid blocks, to eyeball the palette mapping.
  function paletteRow(): GridCell[] {
    const r = emptyRow();
    for (let n = 0; n < 16; n++) put(r, n * 3, "██", idx(n));
    return r;
  }

  const cells: GridCell[][] = (() => {
    const rows: GridCell[][] = [];
    const title = emptyRow();
    put(title, 0, "tim@arlen", idx(2), true);
    rows.push(title);
    const rule = emptyRow();
    put(rule, 0, "---------", idx(8));
    rows.push(rule);
    rows.push(kv("OS:", "Arlen OS", idx(4)));
    rows.push(kv("Kernel:", "7.0.11-arch1-1", idx(4)));
    rows.push(kv("Shell:", "zsh 5.9", idx(4)));
    rows.push(kv("Terminal:", "arlen-terminal", idx(4)));
    rows.push(kv("Colours:", "red green yellow blue", idx(1)));
    rows.push(emptyRow());
    rows.push(paletteRow());
    rows.push(emptyRow());
    // Alignment ruler: every column boundary must line up under the digits.
    const ruler = emptyRow();
    put(ruler, 0, "0123456789 0123456789 0123456789 0123456789", idx(6));
    rows.push(ruler);
    const bars = emptyRow();
    put(bars, 0, "|....|....|....|....|....|....|....|....|", idx(3));
    rows.push(bars);
    rows.push(emptyRow());
    // Wide (double-width / CJK) glyphs: each is one `wide` cell that must render
    // two columns wide, so the trailing ASCII lines up under the ruler above.
    // This mirrors the engine snapshot, which emits one wide cell per glyph and
    // skips the continuation column (the wide-glyph alignment fix).
    const cjk: GridCell[] = [];
    for (const ch of "日本語ＡＢ") {
      cjk.push({ text: ch, fg: idx(2), bg: def, bold: false, italic: false, underline: false, inverse: false, wide: true });
    }
    for (const ch of " <- 5 wide glyphs end at col 10") {
      cjk.push({ text: ch, fg: idx(7), bg: def, bold: false, italic: false, underline: false, inverse: false, wide: false });
    }
    while (cjk.length < COLS) cjk.push(blank());
    rows.push(cjk);
    return rows;
  })();

  // The shell's own captured prompt line (prompt + echoed command, with the
  // real colours/highlighting the shell printed), as the engine would store it
  // in `block.prompt_cells`. A starship-block-style prompt over a subtle bar.
  const bar = { kind: "rgb", value: [28, 29, 33] } as CellColor;
  const promptCells: GridCell[][] = (() => {
    const r = emptyRow();
    const seg = (at: number, ch: string, fg: CellColor, bold = false) => put(r, at, ch, fg, bold, bar);
    seg(0, " ~/Repositories/arlen ", idx(7)); // path, in the bar
    seg(22, "main", idx(2)); // branch
    seg(27, " * ", idx(3)); // dirty marker
    // the echoed command sits after the bar, full strength
    put(r, 31, "neofetch", idx(15), false);
    return [r];
  })();

  // A finished block carrying the same fixture as its captured output, to verify
  // the "grid inside the block" path: the block frame (the captured prompt line,
  // exit chip, time) plus the per-cell output grid rendered inside it.
  const block: Block = {
    id: "b1",
    command: "neofetch",
    exit_code: 0,
    duration_ms: 42,
    cwd: "/home/tim/Repositories/arlen",
    git: { branch: "main", dirty_count: 2 },
    origin: "you",
    body_kind: "grid",
    body: { cells, rows: cells.length },
    prompt_cells: promptCells,
  };
  // The history palette, opened, with its project-scope read ANSWERING NOTHING.
  // The chip row used to just end after Agent for all three cases, so an absent
  // graph read as "there is nothing to scope to". Reached with
  // `?state=scopes-unavailable`; without it the palette stays closed and this
  // route renders exactly as before.
  let paletteReady = $state(false);

  // A live session for the composer row. `refused` drives the real path rather
  // than a faked flag: `terminal_input` is mocked to throw, then Enter is
  // dispatched on the actual input, so what the shot proves is the component's
  // own behaviour and not a state someone set by hand.
  const session = {
    id: "s-1",
    cwd: "/home/tim/projects/arlen",
    status: "running" as const,
    last_exit: null,
  };
  let composerReady = $state(false);
  let emptyReady = $state(false);
  // The stranded case: no session, one button, and the backend refusing to open
  // a shell. Driven through `newSession()` rather than by setting the store, so
  // the shot proves the store is actually reached from the refusal.
  onMount(async () => {
    if (new URLSearchParams(window.location.search).get("state") !== "session-refused") return;
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd) => {
        if (cmd === "terminal_new_session") throw new Error("no session service");
        if (cmd === "terminal_sessions") return [];
        return null;
      });
    }
    emptyReady = true;
    await newSession();
  });

  onMount(async () => {
    if (new URLSearchParams(window.location.search).get("state") !== "input-refused") return;
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd) => {
        if (cmd === "terminal_input") throw new Error("session is gone");
        return null;
      });
    }
    composerReady = true;
    await tick();
    const el = document.getElementById("terminal-composer-input") as HTMLInputElement | null;
    if (!el) return;
    el.value = "cargo test";
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });

  onMount(async () => {
    const state = new URLSearchParams(window.location.search).get("state");
    if (state !== "scopes-unavailable") return;
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd) => {
        if (cmd === "terminal_projects")
          return { state: "unavailable", reason: "graph unreachable" };
        if (cmd === "terminal_history_search") return [];
        return null;
      });
    }
    paletteReady = true;
    historyPaletteOpen.set(true);
  });
</script>

<div style="background:#0a0a0a;padding:8px;min-height:100vh;">
  <GridRegion {cells} />
  <div style="margin-top:16px;max-width:760px;">
    <StreamBlock {block} />
  </div>
  {#if paletteReady}
    <HistoryPalette />
  {/if}
  {#if emptyReady}
    <!-- The REAL panel, not a copy of its markup. A harness that redraws the
         surface proves its own stylesheet and nothing about the app. -->
    <div style="height:220px;max-width:760px;">
      <StreamEmpty kind="none" onretry={() => newSession()} />
    </div>
  {/if}
  {#if composerReady}
    <div style="margin-top:16px;max-width:760px;">
      <Composer {session} />
    </div>
  {/if}
</div>
