/// `shell.spatial` — placement-hint surface for first-party apps.
///
/// Foundation §634 spec: applications provide window placement hints to
/// the compositor (preferred output, position, size). Hints are *not*
/// commands — the compositor applies them when the current layout
/// permits, and the user's manual moves always win.
///
/// # Status
///
/// Full spatial-hint support requires a compositor-side extension of
/// the cosmic-comp fork that has not landed yet. The foundation paper
/// is explicit: "Until then, `shell.spatial` calls are accepted and
/// silently ignored."
///
/// This module exists so applications can declare hints today without
/// needing source changes when compositor support lands. The methods
/// take fully-typed parameters and return `Ok(())` so call-sites read
/// like the production behaviour.
///
/// When the compositor extension lands, swap the body of [`Spatial::hint`]
/// for an Event Bus emit (likely `app.spatial.hint`) and add the
/// matching protobuf payload type. No app-facing API change.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};

/// A preferred output for a window.
///
/// `None` means "any output". The compositor matches against connector
/// names (e.g. `DP-1`, `eDP-1`) when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputHint {
    pub connector: Option<String>,
}

/// Optional position and size hints in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeometryHint {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A spatial hint for a window's placement.
///
/// All fields are optional — apps set only the hints they care about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpatialHint {
    /// The window the hint applies to. Identifier scheme TBD by the
    /// compositor extension; for now the field is round-tripped but
    /// not validated.
    pub window_id: String,
    pub output: Option<OutputHint>,
    pub geometry: Option<GeometryHint>,
}

/// Surface for `shell.spatial`. Generic over emitter for symmetry with
/// the other shell modules even though the current implementation
/// does not emit anything.
pub struct Spatial<E: EventEmitter> {
    _emitter: E,
    _app_id: String,
}

impl<E: EventEmitter> Spatial<E> {
    /// Create a new spatial surface bound to a specific emitter and app.
    ///
    /// The emitter is held for forward-compatibility — when the
    /// compositor extension lands, [`Spatial::hint`] will route through
    /// it. Today it is unused.
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            _emitter: emitter,
            _app_id: app_id.into(),
        }
    }

    /// Suggest a placement for a window. **Currently a no-op** —
    /// foundation §634: "accepted and silently ignored" until the
    /// compositor extension lands. Apps can call this safely now and
    /// will get real behaviour without a code change later.
    ///
    /// # Errors
    /// Currently never fails. Reserved for future use when the call
    /// routes through the Event Bus.
    pub fn hint(&self, _hint: SpatialHint) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    #[tokio::test]
    async fn hint_is_currently_a_noop_and_emits_nothing() {
        let emitter = MockEventEmitter::new();
        let spatial = Spatial::new(emitter.clone(), "com.example.app");

        spatial
            .hint(SpatialHint {
                window_id: "win-1".into(),
                output: Some(OutputHint {
                    connector: Some("DP-1".into()),
                }),
                geometry: Some(GeometryHint {
                    x: Some(100),
                    y: Some(100),
                    width: Some(800),
                    height: Some(600),
                }),
            })
            .await
            .unwrap();

        // Per foundation §634: accepted and silently ignored. When the
        // compositor extension lands this assertion will need to flip.
        assert_eq!(emitter.emitted().await.len(), 0);
    }
}
