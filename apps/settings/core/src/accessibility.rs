//! Screen-filter (invert + colour-blindness modes) types the settings a11y
//! commands round-trip: the frontend camelCase DTO, the on-disk RON schema the
//! compositor watches, and the `ColorFilter` variant<->label mapping. Pure of
//! Tauri and unit-tested in CI - the tests guard the camelCase-rename contract
//! (without it `color_filter` silently dropped across the Tauri boundary, the
//! Sprint-C HIGH-1 finding). The file read/write commands stay in the host.

use serde::{Deserialize, Serialize};

/// Mirrors `compositor::config::ColorFilter` - the discriminant values matter
/// (`offscreen.frag` reads them) but for write-back we only need to round-trip
/// the variant names through RON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorFilter {
    Greyscale,
    Protanopia,
    Deuteranopia,
    Tritanopia,
}

impl ColorFilter {
    /// Parse a compositor variant label, rejecting anything unknown (the set
    /// command maps recognised None-sentinels to "no filter" separately).
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "Greyscale" => Some(Self::Greyscale),
            "Protanopia" => Some(Self::Protanopia),
            "Deuteranopia" => Some(Self::Deuteranopia),
            "Tritanopia" => Some(Self::Tritanopia),
            _ => None,
        }
    }
}

/// On-disk schema. Mirrors the compositor's `ScreenFilter` minus the
/// `night_light_tint` field, which is `#[serde(skip)]` there (computed live,
/// never persisted) - we omit it on write so the compositor's parser sees the
/// same shape it always has.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenFilter {
    pub inverted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_filter: Option<ColorFilter>,
}

/// Frontend-friendly view: `null` for "no filter" instead of the Rust `Option`.
/// Frontend sends `null` from the PopoverSelect when the user picks "None".
///
/// `rename_all = "camelCase"` keeps the JSON contract stable in the convention
/// Svelte stores expect - without it, `color_filter` silently dropped to `None`
/// on every save and reads came back as a missing field (Sprint-C HIGH 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFilterDto {
    pub inverted: bool,
    /// `null` => no colour filter. Otherwise one of the `ColorFilter` variant
    /// names ("Greyscale", "Protanopia", "Deuteranopia", "Tritanopia"). Unknown
    /// labels are rejected at parse time.
    #[serde(default)]
    pub color_filter: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through serde with the camelCase rename ensures
    /// the JSON contract matches the frontend expectation. Without
    /// `rename_all = "camelCase"`, color_filter would silently drop
    /// across the Tauri boundary (Sprint C review HIGH 1).
    #[test]
    fn dto_serialises_as_camel_case() {
        let dto = ScreenFilterDto {
            inverted: true,
            color_filter: Some("Protanopia".into()),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(
            json.contains("colorFilter"),
            "DTO must serialise as camelCase, got: {json}"
        );
        assert!(
            !json.contains("color_filter"),
            "snake_case key leaked through: {json}"
        );

        // And it must deserialise from the same shape.
        let back: ScreenFilterDto = serde_json::from_str(&json).unwrap();
        assert!(back.inverted);
        assert_eq!(back.color_filter, Some("Protanopia".to_string()));
    }

    /// The full set->get round-trip: sending a non-null filter
    /// from the frontend (camelCase) must come back the same way.
    #[test]
    fn dto_round_trips_non_null_filter() {
        let payload = serde_json::json!({
            "inverted": false,
            "colorFilter": "Deuteranopia"
        });
        let dto: ScreenFilterDto = serde_json::from_value(payload).unwrap();
        assert_eq!(dto.color_filter, Some("Deuteranopia".to_string()));

        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["colorFilter"], "Deuteranopia");
    }

    /// Frontend "None"/"none"/empty/missing all map to `Option::None`
    /// on disk so the compositor reads "no filter applied".
    #[test]
    fn from_label_handles_none_sentinels() {
        assert_eq!(ColorFilter::from_label("Greyscale"), Some(ColorFilter::Greyscale));
        assert_eq!(ColorFilter::from_label("Protanopia"), Some(ColorFilter::Protanopia));
        assert_eq!(ColorFilter::from_label("Deuteranopia"), Some(ColorFilter::Deuteranopia));
        assert_eq!(ColorFilter::from_label("Tritanopia"), Some(ColorFilter::Tritanopia));

        // Anything else returns None — the set command then maps
        // that to "no filter" for the recognised None-sentinels.
        assert_eq!(ColorFilter::from_label("None"), None);
        assert_eq!(ColorFilter::from_label(""), None);
        assert_eq!(ColorFilter::from_label("garbage"), None);
    }

    /// Round-trip through the on-disk RON schema matches the
    /// compositor's `ScreenFilter` shape (inverted + color_filter
    /// only — no night_light_tint).
    #[test]
    fn ron_roundtrip_matches_compositor_shape() {
        let state = ScreenFilter {
            inverted: true,
            color_filter: Some(ColorFilter::Greyscale),
        };
        let s = ron::ser::to_string(&state).unwrap();
        // Sanity: variant name should be in serialised output —
        // that's what the compositor parser keys on.
        assert!(s.contains("Greyscale"), "missing variant name: {s}");
        assert!(s.contains("inverted"));

        // Empty-filter shape: color_filter omitted thanks to
        // skip_serializing_if. The compositor's serde-default
        // for missing fields then resolves to None on read.
        let empty = ScreenFilter {
            inverted: false,
            color_filter: None,
        };
        let s2 = ron::ser::to_string(&empty).unwrap();
        assert!(!s2.contains("color_filter"), "should omit None: {s2}");
    }

    #[test]
    fn dto_default_color_filter_is_none() {
        let json = r#"{"inverted":false}"#;
        let dto: ScreenFilterDto = serde_json::from_str(json).unwrap();
        assert!(!dto.inverted);
        assert!(dto.color_filter.is_none());
    }
}
