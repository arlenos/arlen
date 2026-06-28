<script lang="ts">
  /// Headless look-mock for the terminal block-frame chrome — to DECIDE the
  /// styled-vs-classic A/B with Tim and iterate the look. This is a faithful
  /// fake terminal (the muted Arlen palette + mono font) with mock blocks, NOT
  /// the real terminal: the real chrome renders as xterm.js decorations anchored
  /// to OSC133 rows (metal-verify). Two panes show the same blocks in both
  /// styles, with rest / hover / focus / failed / agent-origin states. Not
  /// shipped in any nav; a dev/test route only (separate from the coder's
  /// `_rendertest` GridRegion harness).

  type Block = {
    cwd: string;
    git: string;
    dirty: boolean;
    cmd: string;
    out: string[];
    exit: number;
    dur: string;
    origin: "user" | "agent";
    state: "rest" | "hover" | "focus" | "failed";
  };

  const blocks: Block[] = [
    {
      cwd: "~/projects/arlen", git: "main", dirty: false,
      cmd: "cargo build",
      out: ["   Compiling arlen v0.1.0", "    Finished `release` in 2.3s"],
      exit: 0, dur: "2.3s", origin: "user", state: "hover",
    },
    {
      cwd: "~/projects/arlen", git: "main", dirty: true,
      cmd: "git add -A && git commit", out: ["[main 9f2c1a] terminal block chrome"],
      exit: 0, dur: "0.1s", origin: "user", state: "rest",
    },
    {
      cwd: "~/projects/arlen", git: "main", dirty: true,
      cmd: "cargo test --doc",
      out: ["error[E0433]: failed to resolve import", "error: could not compile `arlen`"],
      exit: 101, dur: "0.4s", origin: "user", state: "failed",
    },
    {
      cwd: "~/projects/arlen", git: "main", dirty: false,
      cmd: "ls docs/architecture", out: ["terminal.md   design-system.md   ai-app.md"],
      exit: 0, dur: "0.0s", origin: "agent", state: "focus",
    },
  ];
  const pchar = (b: Block) => (b.origin === "agent" ? "✦" : "❯");
</script>

<div class="page">
  <section class="col">
    <h2>B · Classic <span class="rec">recommended</span></h2>
    <p class="note">Shell prints prompt + command + output; chrome only on hover / focus.</p>
    <div class="term">
      {#each blocks as b (b.cmd)}
        <div class="b-block" class:hover={b.state === "hover"} class:focus={b.state === "focus"} class:failed={b.state === "failed"}>
          <div class="prompt">
            <span class="path">{b.cwd}</span>
            <span class="git">{b.git}{b.dirty ? "*" : ""}</span>
            <span class="spacer"></span>
            {#if b.state === "hover" || b.state === "focus"}<span class="actions">⋯</span>{/if}
            {#if b.exit !== 0}
              <span class="chip err">✗ {b.exit} · {b.dur}</span>
            {:else if b.state === "hover" || b.state === "focus"}
              <span class="chip">✓ {b.dur}</span>
            {/if}
          </div>
          <div class="cmd"><span class="pchar" class:agent={b.origin === "agent"}>{pchar(b)}</span> {b.cmd}</div>
          {#each b.out as line, i (i)}<div class="out" class:err={b.exit !== 0}>{line}</div>{/each}
        </div>
      {/each}
    </div>
  </section>

  <section class="col">
    <h2>A · Styled</h2>
    <p class="note">A header bar per block redraws cwd · git as chrome; result + actions always on.</p>
    <div class="term">
      {#each blocks as b (b.cmd)}
        <div class="a-block" class:focus={b.state === "focus"} class:failed={b.state === "failed"}>
          <div class="a-header">
            <span class="path">{b.cwd}</span>
            <span class="git">{b.git}{b.dirty ? "*" : ""}</span>
            <span class="spacer"></span>
            {#if b.exit !== 0}
              <span class="chip err">✗ {b.exit} · {b.dur}</span>
            {:else}
              <span class="chip">✓ {b.dur}</span>
            {/if}
            <span class="actions">⋯</span>
          </div>
          <div class="a-body">
            <div class="cmd"><span class="pchar" class:agent={b.origin === "agent"}>{pchar(b)}</span> {b.cmd}</div>
            {#each b.out as line, i (i)}<div class="out" class:err={b.exit !== 0}>{line}</div>{/each}
          </div>
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  /* The muted Arlen terminal palette (mirrors terminal-theme.ts). */
  .page {
    --bg: #0f0f0f;
    --fg: #e4e5ea;
    --muted: #8a8c94;
    --faint: #54565e;
    --green: #8fae74;
    --yellow: #d4b483;
    --red: #c96a6a;
    --blue: #7d9cc4;
    --accent: #83b3b1;
    display: flex;
    gap: 28px;
    padding: 28px;
    min-height: 100vh;
    background: #060606;
    font-family: ui-sans-serif, system-ui, sans-serif;
  }
  .col {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0 0 0.25rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: #e4e5ea;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .rec {
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--green);
  }
  .note {
    margin: 0 0 0.75rem;
    font-size: 0.6875rem;
    color: #888;
  }

  .term {
    background: var(--bg);
    color: var(--fg);
    border-radius: 8px;
    padding: 0.75rem 0;
    font-family: "Cascadia Code", "JetBrainsMono Nerd Font Mono", "JetBrains Mono", ui-monospace, monospace;
    font-size: 12.5px;
    line-height: 1.55;
    overflow: hidden;
  }

  .path { color: var(--blue); }
  .git { color: var(--green); margin-left: 0.75rem; }
  .cmd { color: var(--fg); }
  .pchar { color: var(--green); }
  .pchar.agent { color: var(--accent); }
  .out { color: color-mix(in srgb, var(--fg) 70%, transparent); }
  .out.err { color: var(--red); }
  .spacer { flex: 1; }
  .actions { color: var(--faint); }
  .chip {
    font-size: 0.6875rem;
    color: var(--muted);
  }
  .chip.err { color: var(--red); }

  /* B — classic: at rest pure, chrome on hover/focus, hairline divider. */
  .b-block {
    padding: 0.25rem 0.875rem;
    border-left: 2px solid transparent;
  }
  .b-block + .b-block {
    border-top: 1px solid color-mix(in srgb, var(--fg) 7%, transparent);
  }
  .b-block .prompt {
    display: flex;
    align-items: center;
  }
  .b-block.hover {
    background: color-mix(in srgb, var(--fg) 4%, transparent);
  }
  .b-block.focus {
    border-left-color: var(--accent);
    background: color-mix(in srgb, var(--fg) 3%, transparent);
  }
  .b-block.failed {
    border-left-color: color-mix(in srgb, var(--red) 55%, transparent);
  }

  /* A — styled: a header bar + framed body per block. */
  .a-block {
    margin: 0 0.625rem 0.625rem;
    border: 1px solid color-mix(in srgb, var(--fg) 10%, transparent);
    border-radius: 6px;
    overflow: hidden;
  }
  .a-block.focus {
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .a-block.failed {
    border-color: color-mix(in srgb, var(--red) 40%, transparent);
  }
  .a-header {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.625rem;
    background: color-mix(in srgb, var(--fg) 6%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--fg) 8%, transparent);
  }
  .a-body {
    padding: 0.375rem 0.625rem;
  }
</style>
