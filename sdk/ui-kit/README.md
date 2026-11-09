# ui-kit

The Arlen component library. Built with Tauri, SvelteKit, Tailwind v4 and shadcn-svelte. All first-party Arlen apps import from here to get consistent UI, theming and window decorations.

This is not a standalone app. It is the shared foundation that desktop-shell, Waypointer, Settings and other Arlen apps build on top of.

## What's here

```
ui-kit/
├── src/
│   ├── lib/
│   │   ├── components/ui/    shadcn-svelte components
│   │   ├── theme.ts          theme loader (fetches tokens from Tauri backend)
│   │   └── utils.ts          shadcn utilities (cn, etc.)
│   ├── routes/
│   │   └── +layout.svelte    loads theme on startup
│   └── app.css               surface token definitions + Tailwind setup
└── src-tauri/
    └── src/
        └── theme.rs          reads ~/.config/arlen/theme.toml
```

## Surface Token System

Arlen uses a two-level token system. The first level is defined in `~/.config/arlen/theme.toml` and loaded at runtime. The second level maps those tokens to shadcn-svelte CSS variables so Shadcn components automatically use Arlen colors.

**theme.toml format:**

```toml
[color.bg]
shell   = "#1a1a2e"   # shell chrome (panel, launcher)
app     = "#ffffff"   # app window backgrounds
card    = "#f5f5f7"   # cards, secondary surfaces
overlay = "#00000080" # modals, popovers
input   = "#f0f0f0"   # input fields

[color.fg]
shell = "#e8e8f0"     # text on shell surfaces
app   = "#1a1a2e"     # text on app surfaces

[color]
accent = "#7c6af7"    # buttons, focus rings, highlights
border = "#e2e2e8"    # separators, input outlines

radius = "0.5rem"
```

If `theme.toml` does not exist or cannot be parsed, the built-in default is used: a monochrome dark shell (`dark.toml`), with `light.toml` as the light counterpart.

**CSS Custom Properties:**

All tokens are available as CSS custom properties: `--color-bg-shell`, `--color-bg-app`, `--color-accent`, etc. They are set by the theme loader at startup and can be updated at runtime without a page reload.

**Shell surface context:**

Components inside a `.shell-surface` element automatically get shell colors instead of app colors. Use this for panel, launcher and other shell chrome:

```svelte
<div class="shell-surface">
  <!-- These components will use --color-bg-shell, --color-fg-shell etc. -->
  <Button>Launch</Button>
</div>
```

## Using in another app

Add ui-kit as a local npm dependency:

```json
{
  "dependencies": {
    "@arlen/ui-kit": "file:../ui-kit"
  }
}
```

Then import components and the theme loader:

```svelte
<script>
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { loadTheme } from "@arlen/ui-kit/theme";
  import { onMount } from "svelte";

  onMount(() => loadTheme());
</script>
```

## Development

```bash
npm install
npm run dev       # starts Vite dev server at localhost:1420
cargo tauri dev   # starts full Tauri app (reads theme.toml, renders shell)
```

**Rust tests** (theme loader):

```bash
cd src-tauri
cargo test --lib
```

## Adding a component

```bash
npx shadcn-svelte@latest add button
npx shadcn-svelte@latest add dialog
```

Components are copied into `src/lib/components/ui/` and can be modified freely. They are not updated automatically after installation.

## Learnings from the website and halbat

Collected on 9 November 2026 from the redesigns of tim.kicker.dev and halbat, whose kits take their tokens from this one. Each one says whether the kit took it and why.

- **No dot inside a tag.** Tim asked for it to go everywhere. Both sites drew it as a `::before` on the pill and both removed it (website `1ea000f`, halbat `e819bb92`). Colour and word carry the state. Badge and Chip here never had one, and `tag-no-dot.test.ts` keeps it that way. The dots that remain in Arlen are passed in by apps (harness capability bar, settings providers, the files filter menu).
- **A fixed overlay must not inherit a column's width.** On the website a column rule capped every child at 560px, so a `position: fixed` game screen sat as a strip on a phone in landscape. A full-screen layer sets `max-width: none` and `margin: 0` itself.
- **Segment labels are centred** and keep a small inner padding. A global `button { text-align: left }` had pushed them against the left border.
- **No middle dot as a separator.** Use layout or a comma instead.
- From halbat's design rules (halbat repo, `.claude/skills/halbat-frontend-design/SKILL.md`, section Visual system), checked against Arlen's own system on the same day:
  - **Radii stay Arlen's.** halbat has three (6 clickable, 12 floating, round); Arlen has the five semantic tokens in `docs/architecture/theme-system.md` (chip 4, button 6, input 8, card 12, modal 16, full) and the concentric rule in `docs/architecture/rounding-fix.md`. What the two agree on is taken from Arlen's table: a button face is `--radius-button`, a menu is `--radius-card` and its rows are concentric, an avatar is round. No token value changes.
  - Spacing in steps of four is not an Arlen rule. Arlen's spacing tokens are still to be written (design-system §3), so a pass over the kit's gaps waits for that rather than borrowing halbat's scale.
  - One selection, one mark: taken for menu rows (a neutral hover, no accent fill) and for the focus ring after a mouse click. The segmented control keeps its fill and outline (8 October 2026) and a chosen option in a select keeps its tint beside the check.
  - No 3D hover: Arlen is flat already.
  - Scrollbars handled once in the kit (`motion.css`), not per app.
  - Contrast measured at 4.5:1: the floor in `docs/architecture/theming-system-plan.md`, applied to the kit's muted text.
  - Not taken yet: a colour keeping one role, thousands with a narrow no-break space, `tabular-nums` only on number cells. Each is a pass of its own.

## Part of

[Arlen](https://github.com/arlenos): a Linux desktop OS built around a system-wide knowledge graph.
