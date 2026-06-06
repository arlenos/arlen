# @lunaris/tauri-plugin-portal

Tauri plugin wrapping the standard `org.freedesktop.portal.Desktop`
FileChooser and OpenURI interfaces. First-party Lunaris apps use
this plugin to open file pickers and URIs without coupling to
either the upstream `xdg-desktop-portal-gtk` library or the
Lunaris-specific backend implementation.

Under a Lunaris session the calls are served by
`xdg-desktop-portal-lunaris` (Lunaris-themed picker UI). Under
GNOME/KDE the frontend daemon falls through to whichever backend
is configured for that desktop, so the plugin keeps working in
mixed environments.

## Usage

### Rust

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_lunaris_portal::init())
        .run(tauri::generate_context!())
        .expect("error running app");
}
```

### TypeScript

```typescript
import { pickFile, pickDirectory, saveFile, openUri } from '@lunaris/tauri-plugin-portal';

const dir = await pickDirectory({ title: 'Choose folder' });
if (dir !== null) {
  console.log('user picked:', dir);
}

const files = await pickFile({
  title: 'Open images',
  multiple: true,
  filters: [{ name: 'Images', patterns: [{ kind: 'glob', pattern: '*.png' }] }],
});

const target = await saveFile({
  title: 'Save report',
  currentName: 'report.pdf',
  filters: [{ name: 'PDF', patterns: [{ kind: 'glob', pattern: '*.pdf' }] }],
});

await openUri('https://example.com');
```

## Cancellation vs. error

The picker functions return `null` when the user dismisses the
dialog. Errors throw — they are reserved for actual failures
(portal frontend not installed, scheme rejected, backend failure).

```typescript
const dir = await pickDirectory();
if (dir === null) return; // user cancelled

try {
  await openUri('https://example.com');
} catch (err) {
  // portal unavailable, scheme rejected, ...
}
```

## Direct Rust API

Downstream Rust callers (e.g. `app-settings/picker.rs`) can call
`tauri_plugin_lunaris_portal::api::*` directly without going
through Tauri's invoke machinery. Same connection-per-call cost
as the Tauri-command path; saves the JSON serialisation hop.

```rust
use tauri_plugin_lunaris_portal::{api, PickFileOptions};

let result = api::pick_directory(PickFileOptions {
    title: Some("Choose folder".into()),
    ..Default::default()
}).await?;
```
