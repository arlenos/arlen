/// Bundled tile-default catalogue.
///
/// First-run fallback when `quicksettings.toml` is missing or empty.
/// Order matches the spec in `docs/architecture/quicksettings-system.md`
/// §"Phase-1 tile inventory".

use lunaris_modules::TileSize;
use serde::Serialize;

use super::layout::{LayoutFile, TileEntry};

/// Compile-time tile-id → default-size table.
///
/// Each entry maps to one `TileEntry { id, visible: true, size }` in
/// the bundled layout. Backed catalogue (label/icon/click/component)
/// lives in the frontend tile registry — this table only governs
/// *which* tiles appear and *where*.
/// Theme switching is intentionally NOT a tile — it lives in the
/// user-row footer because it's an account/identity-level setting.
/// Row layout: project + knowledge (1×1 context pair), then four
/// toggles (Network/Bluetooth/DND/Airplane) in two rows, then the
/// full-row sliders. No orphan cells in the 2-column grid.
const BUNDLED: &[(&str, TileSize)] = &[
    ("system.project-context", TileSize::OneByOne),
    ("system.knowledge", TileSize::OneByOne),
    ("system.network", TileSize::OneByOne),
    ("system.bluetooth", TileSize::OneByOne),
    ("system.dnd", TileSize::OneByOne),
    ("system.airplane", TileSize::OneByOne),
    ("system.brightness", TileSize::TwoByOne),
    ("system.audio", TileSize::TwoByOne),
    ("system.user-row", TileSize::TwoByOne),
];

/// Lookup the default size for a given tile id, if any.
pub fn default_size(tile_id: &str) -> Option<TileSize> {
    BUNDLED.iter().find(|(id, _)| *id == tile_id).map(|(_, s)| *s)
}

/// Construct the bundled layout. Callers use this when the user file
/// is empty so the very-first-render has *something* to draw.
pub fn bundled_layout() -> LayoutFile {
    LayoutFile {
        tiles: BUNDLED
            .iter()
            .map(|(id, size)| TileEntry {
                id: (*id).to_string(),
                visible: true,
                size: *size,
            })
            .collect(),
    }
}

/// Tauri command: returns the bundled defaults so the frontend can
/// merge them with the user file when populating the customisation UI.
#[tauri::command]
pub fn qs_layout_bundled_defaults() -> BundledDefaults {
    BundledDefaults {
        tiles: BUNDLED
            .iter()
            .map(|(id, size)| BundledTile {
                id: (*id).to_string(),
                size: *size,
            })
            .collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BundledDefaults {
    pub tiles: Vec<BundledTile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundledTile {
    pub id: String,
    pub size: TileSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_layout_has_nine_tiles() {
        // Theme moved into user-row footer (see BUNDLED comment).
        // Knowledge tile pairs with project-context in row 1.
        assert_eq!(bundled_layout().tiles.len(), 9);
    }

    #[test]
    fn bundled_layout_starts_with_project_context() {
        assert_eq!(bundled_layout().tiles[0].id, "system.project-context");
    }

    #[test]
    fn default_size_known_tile() {
        assert_eq!(default_size("system.network"), Some(TileSize::OneByOne));
        assert_eq!(default_size("system.knowledge"), Some(TileSize::OneByOne));
        assert_eq!(default_size("system.brightness"), Some(TileSize::TwoByOne));
        assert_eq!(default_size("system.project-context"), Some(TileSize::OneByOne));
    }

    #[test]
    fn default_size_unknown_tile_is_none() {
        assert!(default_size("module.foo:bar").is_none());
    }

    #[test]
    fn bundled_layout_all_visible() {
        for entry in bundled_layout().tiles {
            assert!(entry.visible, "{} should be visible by default", entry.id);
        }
    }
}
