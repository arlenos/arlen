# ui-kit

The Lunaris component library. Built with Tauri, SvelteKit, Tailwind v4, and shadcn-svelte. All first-party Lunaris apps import from here to get consistent UI, theming, and window decorations.

This is not a standalone app. It is the shared foundation that desktop-shell, Waypointer, Settings, and other Lunaris apps build on top of.

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
        └── theme.rs          reads ~/.config/lunaris/theme.toml
```

## Surface Token System

Lunaris uses a two-level token system. The first level is defined in `~/.config/lunaris/theme.toml` and loaded at runtime. The second level maps those tokens to shadcn-svelte CSS variables so Shadcn components automatically use Lunaris colors.

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

If `theme.toml` does not exist or cannot be parsed, the built-in **Panda** theme is used (dark shell, light apps).

**CSS Custom Properties:**

All tokens are available as CSS custom properties: `--color-bg-shell`, `--color-bg-app`, `--color-accent`, etc. They are set by the theme loader at startup and can be updated at runtime without a page reload.

**Shell surface context:**

Components inside a `.shell-surface` element automatically get shell colors instead of app colors. Use this for panel, launcher, and other shell chrome:

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
    "@lunaris/ui-kit": "file:../ui-kit"
  }
}
```

Then import components and the theme loader:

```svelte
<script>
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { loadTheme } from "@lunaris/ui-kit/theme";
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

## Part of

[Lunaris](https://github.com/lunaris-sys): a Linux desktop OS built around a system-wide knowledge graph.
