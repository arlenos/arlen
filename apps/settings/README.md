# Lunaris Settings

System settings app for Lunaris OS. Tauri 2 + SvelteKit.

## Development

```bash
npm install
npm run tauri dev
```

## Structure

```
src/
  routes/            # SvelteKit pages (one per panel)
  lib/
    components/      # UI components + settings primitives
    stores/          # config.ts factory, navigation, theme
src-tauri/
  src/
    commands/        # Tauri command handlers (config, theme, ...)
    config_watcher.rs
```

## Sprint 1 Scope

- Sidebar navigation with 7 panels (Appearance active, others placeholder)
- Full Appearance panel: theme mode, accent, window, fonts
- Generic config store factory (`createConfigStore<T>`)
- Appearance.toml watcher -> live reload
- Backend: `config_get/set/reset/get_default`, `theme_get/set_mode/set_accent`
