# tauri-plugin-lunaris-clipboard

Tauri plugin for the Lunaris clipboard subsystem.

Wraps `os_sdk::UnixClipboardClient` and exposes
`write` / `read` / `history` / `subscribe` / `unsubscribe`
Tauri commands plus a `lunaris://clipboard-changed` event that
the frontend can listen to for live updates.

The plugin talks to the desktop-shell broker over the unix
socket at `$XDG_RUNTIME_DIR/lunaris/clipboard.sock`, which is
the only process allowed to interact with the raw Wayland
`wl_data_device` interface. Apps therefore get clipboard access
without holding a Wayland connection of their own.

## Permissions

Each operation requires the matching scope in the app's
permission profile (see `sdk/permissions`):

| Operation     | Scope                   |
|---------------|-------------------------|
| `write`       | `clipboard.write`       |
| `read`        | `clipboard.read`        |
| `subscribe`   | `clipboard.read`        |
| `history`     | `clipboard.history`     |
| sensitive content read | `clipboard.read_sensitive` |

## Usage

### Rust

```rust,ignore
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_lunaris_clipboard::init())
        .run(tauri::generate_context!())
        .expect("error running app");
}
```

### TypeScript

```ts
import { write, subscribe } from '@lunaris/tauri-plugin-clipboard';
import { listen } from '@tauri-apps/api/event';

await write({
  content: new TextEncoder().encode('hello'),
  mime: 'text/plain',
});

await subscribe();
await listen('lunaris://clipboard-changed', (e) => {
  console.log('clipboard changed:', e.payload);
});
```

## Architecture

See `docs/architecture/clipboard-api.md` for the full broker
architecture, sensitivity-label handling, and edge-case
coverage.
