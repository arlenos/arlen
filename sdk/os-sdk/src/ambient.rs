//! `shell.ambient` — subtle desktop-wide visual effects while
//! the app is focused (slow accent pulse during a build, warning
//! tint while tests fail, etc.).
//!
//! Foundation §6.4 Listing 17, p55. Only token-system colors
//! permitted, intensity hard-capped at 0.5 — both enforced
//! SDK-side. The user can disable all ambient effects globally
//! via `~/.config/lunaris/shell.toml [ambient] enabled = false`.

use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::{
    AmbientClearedPayload, AmbientColor as ProtoColor, AmbientEffect as ProtoEffect,
    AmbientSetPayload, AmbientSpeed as ProtoSpeed,
};

/// Maximum allowed intensity. Foundation §6.4 Listing 17:
/// "Maximum intensity is capped at 0.5." Hard reject above
/// this — the cap is part of the write contract, not just a
/// renderer ceiling.
pub const MAX_INTENSITY: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AmbientEffect {
    Pulse,
    Tint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AmbientColor {
    Accent,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AmbientSpeed {
    Slow,
    Medium,
    Fast,
}

/// Parameters for [`Ambient::set`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmbientParams {
    pub effect: AmbientEffect,
    pub color: AmbientColor,
    /// Clamped to `[0.0, MAX_INTENSITY]`; non-finite rejected.
    pub intensity: f32,
    pub speed: AmbientSpeed,
    /// Free-form, for debug / future audit. Not rendered.
    #[serde(default)]
    pub reason: String,
    /// Optional shell-side autoClear timer in milliseconds.
    /// `None` (or 0) means no auto-clear; the app must call
    /// `clear()` itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_clear_ms: Option<u32>,
}

/// Surface for the `shell.ambient` API.
pub struct Ambient<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Ambient<E> {
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Set the ambient effect for this app. Replaces any
    /// previous ambient state. Render is gated by user setting
    /// (`shell.toml [ambient] enabled`) on the consumer side.
    ///
    /// # Errors
    /// [`EmitError::SerializationFailed`] if `intensity` is
    /// non-finite or > [`MAX_INTENSITY`]. Negative values
    /// clamp to 0.0 silently.
    pub fn set(
        &self,
        params: AmbientParams,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move {
            if !params.intensity.is_finite() {
                return Err(EmitError::SerializationFailed(format!(
                    "AmbientParams.intensity must be finite, got {}",
                    params.intensity
                )));
            }
            if params.intensity > MAX_INTENSITY {
                return Err(EmitError::SerializationFailed(format!(
                    "AmbientParams.intensity must be <= {MAX_INTENSITY}, got {}",
                    params.intensity
                )));
            }
            let intensity = params.intensity.max(0.0);

            let payload = AmbientSetPayload {
                app_id: self.app_id.clone(),
                effect: effect_to_proto(params.effect) as i32,
                color: color_to_proto(params.color) as i32,
                intensity,
                speed: speed_to_proto(params.speed) as i32,
                reason: params.reason,
                auto_clear_ms: params.auto_clear_ms.unwrap_or(0),
            };
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("AmbientSetPayload encode is infallible");
            self.emitter.emit("app.ambient.set", buf).await
        }
    }

    /// Clear the active ambient effect for this app
    /// immediately (cancels any pending autoClear timer).
    pub fn clear(&self) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = AmbientClearedPayload {
            app_id: self.app_id.clone(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("AmbientClearedPayload encode is infallible");
        async move { self.emitter.emit("app.ambient.cleared", buf).await }
    }
}

fn effect_to_proto(e: AmbientEffect) -> ProtoEffect {
    match e {
        AmbientEffect::Pulse => ProtoEffect::Pulse,
        AmbientEffect::Tint => ProtoEffect::Tint,
    }
}

fn color_to_proto(c: AmbientColor) -> ProtoColor {
    match c {
        AmbientColor::Accent => ProtoColor::Accent,
        AmbientColor::Warning => ProtoColor::Warning,
        AmbientColor::Error => ProtoColor::Error,
        AmbientColor::Success => ProtoColor::Success,
    }
}

fn speed_to_proto(s: AmbientSpeed) -> ProtoSpeed {
    match s {
        AmbientSpeed::Slow => ProtoSpeed::Slow,
        AmbientSpeed::Medium => ProtoSpeed::Medium,
        AmbientSpeed::Fast => ProtoSpeed::Fast,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode_set(bytes: &[u8]) -> AmbientSetPayload {
        AmbientSetPayload::decode(bytes).expect("valid payload")
    }

    #[tokio::test]
    async fn set_emits_correct_payload() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "com.example.builder");
        am.set(AmbientParams {
            effect: AmbientEffect::Pulse,
            color: AmbientColor::Accent,
            intensity: 0.3,
            speed: AmbientSpeed::Slow,
            reason: "build-running".into(),
            auto_clear_ms: None,
        })
        .await
        .unwrap();

        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.ambient.set");
        let p = decode_set(&events[0].payload);
        assert_eq!(p.app_id, "com.example.builder");
        assert_eq!(p.effect, ProtoEffect::Pulse as i32);
        assert_eq!(p.color, ProtoColor::Accent as i32);
        assert_eq!(p.speed, ProtoSpeed::Slow as i32);
        assert!((p.intensity - 0.3).abs() < f32::EPSILON);
        assert_eq!(p.reason, "build-running");
        assert_eq!(p.auto_clear_ms, 0);
    }

    #[tokio::test]
    async fn set_with_auto_clear_includes_ms() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "app");
        am.set(AmbientParams {
            effect: AmbientEffect::Tint,
            color: AmbientColor::Warning,
            intensity: 0.15,
            speed: AmbientSpeed::Medium,
            reason: "test-failing".into(),
            auto_clear_ms: Some(5000),
        })
        .await
        .unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert_eq!(p.auto_clear_ms, 5000);
    }

    #[tokio::test]
    async fn set_rejects_intensity_above_cap() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "app");
        let err = am
            .set(AmbientParams {
                effect: AmbientEffect::Pulse,
                color: AmbientColor::Accent,
                intensity: 0.6,
                speed: AmbientSpeed::Slow,
                reason: String::new(),
                auto_clear_ms: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, EmitError::SerializationFailed(_)));
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_rejects_nan_and_infinity() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "app");
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let err = am
                .set(AmbientParams {
                    effect: AmbientEffect::Pulse,
                    color: AmbientColor::Accent,
                    intensity: bad,
                    speed: AmbientSpeed::Slow,
                    reason: String::new(),
                    auto_clear_ms: None,
                })
                .await
                .unwrap_err();
            assert!(matches!(err, EmitError::SerializationFailed(_)));
        }
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_clamps_negative_intensity_to_zero() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "app");
        am.set(AmbientParams {
            effect: AmbientEffect::Pulse,
            color: AmbientColor::Accent,
            intensity: -0.2,
            speed: AmbientSpeed::Slow,
            reason: String::new(),
            auto_clear_ms: None,
        })
        .await
        .unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert!(p.intensity >= 0.0);
        assert!(p.intensity < f32::EPSILON);
    }

    #[tokio::test]
    async fn clear_emits_dedicated_event() {
        let emitter = MockEventEmitter::new();
        let am = Ambient::new(emitter.clone(), "app");
        am.clear().await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.ambient.cleared");
    }
}
