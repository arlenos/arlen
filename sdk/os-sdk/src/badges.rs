//! `shell.badges` — per-app status indicator rendered as an
//! overlay on the GlobalMenuBar app name.
//!
//! Foundation §6.4 Listing 14, p54. Four variants (count,
//! dot, status, count_with_status). Mutually exclusive per
//! app: setting a new badge replaces any previous.
//!
//! Wire: `app.badge.set` / `app.badge.cleared`. The Knowledge
//! Daemon promotes error / warning badges into UserAction
//! graph nodes (foundation requirement); count-only / dot do
//! not promote.

use std::future::Future;

use prost::Message;
use serde::{Deserialize, Serialize};

use crate::event::{EmitError, EventEmitter};
use crate::proto::{
    BadgeClearedPayload, BadgeSetPayload, BadgeStatus as ProtoBadgeStatus,
    BadgeVariant,
};

/// Badge status type. Encoded as enum (not free string) so
/// invalid statuses cannot reach the wire from valid SDK
/// callers. Foundation §6.4 Listing 14 lists these four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BadgeStatus {
    Success,
    Warning,
    Error,
    Progress,
}

/// One of four badge variants. Mutually exclusive per app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum BadgeKind {
    /// Numeric badge ("3", "12", "99+").
    Count { count: u32 },
    /// Plain dot — presence indicator without count.
    Dot,
    /// Status indicator. Optional `value` is only meaningful
    /// for `Progress` (fill 0..=1, NaN/Infinity rejected).
    Status {
        status: BadgeStatus,
        value: Option<f32>,
    },
    /// Count + status combo (foundation example: 2 unread
    /// warnings).
    CountWithStatus {
        count: u32,
        status: BadgeStatus,
    },
}

/// Surface for the `shell.badges` API.
pub struct Badges<E: EventEmitter> {
    emitter: E,
    app_id: String,
}

impl<E: EventEmitter> Badges<E> {
    pub fn new(emitter: E, app_id: impl Into<String>) -> Self {
        Self {
            emitter,
            app_id: app_id.into(),
        }
    }

    /// Set the app's badge. Replaces any previous variant.
    ///
    /// # Errors
    /// [`EmitError::SerializationFailed`] for non-finite
    /// `value` on a `Progress` status.
    pub fn set(
        &self,
        badge: BadgeKind,
    ) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        async move {
            let payload = badge_to_proto(self.app_id.clone(), badge)?;
            let mut buf = Vec::with_capacity(payload.encoded_len());
            payload
                .encode(&mut buf)
                .expect("BadgeSetPayload encode is infallible");
            self.emitter.emit("app.badge.set", buf).await
        }
    }

    /// Clear any active badge for this app.
    pub fn clear(&self) -> impl Future<Output = Result<(), EmitError>> + Send + '_ {
        let payload = BadgeClearedPayload {
            app_id: self.app_id.clone(),
        };
        let mut buf = Vec::with_capacity(payload.encoded_len());
        payload
            .encode(&mut buf)
            .expect("BadgeClearedPayload encode is infallible");
        async move { self.emitter.emit("app.badge.cleared", buf).await }
    }
}

fn badge_to_proto(app_id: String, badge: BadgeKind) -> Result<BadgeSetPayload, EmitError> {
    let (variant, count, status, progress_value) = match badge {
        BadgeKind::Count { count } => (
            BadgeVariant::Count,
            count,
            ProtoBadgeStatus::Unspecified,
            None,
        ),
        BadgeKind::Dot => (
            BadgeVariant::Dot,
            0,
            ProtoBadgeStatus::Unspecified,
            None,
        ),
        BadgeKind::Status { status, value } => {
            // Codex Sprint-B-fat C10: enforce the badge
            // wire-contract invariants at the SDK boundary so
            // shell + Knowledge-Graph consumers never see a
            // value attached to the wrong status.
            //
            // - `value` is meaningful ONLY for
            //   `BadgeStatus::Progress` (foundation Listing 14
            //   line 4). Any other status with a value is a
            //   caller bug.
            // - When supplied for Progress, it must be finite
            //   and inside `[0.0, 1.0]`. Out-of-range = wire
            //   contract violation.
            if let Some(v) = value {
                if !matches!(status, BadgeStatus::Progress) {
                    return Err(EmitError::SerializationFailed(format!(
                        "BadgeKind::Status.value is only valid for status=Progress, \
                         got status={status:?} with value={v}"
                    )));
                }
                if !v.is_finite() {
                    return Err(EmitError::SerializationFailed(format!(
                        "BadgeKind::Status.value must be finite, got {v}"
                    )));
                }
                if !(0.0..=1.0).contains(&v) {
                    return Err(EmitError::SerializationFailed(format!(
                        "BadgeKind::Status.value must be in [0.0, 1.0], got {v}"
                    )));
                }
            }
            (
                BadgeVariant::Status,
                0,
                badge_status_to_proto(status),
                value,
            )
        }
        BadgeKind::CountWithStatus { count, status } => (
            BadgeVariant::CountWithStatus,
            count,
            badge_status_to_proto(status),
            None,
        ),
    };

    Ok(BadgeSetPayload {
        app_id,
        variant: variant as i32,
        count,
        status: status as i32,
        progress_value,
    })
}

fn badge_status_to_proto(status: BadgeStatus) -> ProtoBadgeStatus {
    match status {
        BadgeStatus::Success => ProtoBadgeStatus::Success,
        BadgeStatus::Warning => ProtoBadgeStatus::Warning,
        BadgeStatus::Error => ProtoBadgeStatus::Error,
        BadgeStatus::Progress => ProtoBadgeStatus::Progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEventEmitter;

    fn decode_set(bytes: &[u8]) -> BadgeSetPayload {
        BadgeSetPayload::decode(bytes).expect("valid payload")
    }

    #[tokio::test]
    async fn set_count_emits_with_count_variant() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "com.example.mail");
        b.set(BadgeKind::Count { count: 7 }).await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.badge.set");
        let p = decode_set(&events[0].payload);
        assert_eq!(p.app_id, "com.example.mail");
        assert_eq!(p.variant, BadgeVariant::Count as i32);
        assert_eq!(p.count, 7);
    }

    #[tokio::test]
    async fn set_dot() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        b.set(BadgeKind::Dot).await.unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert_eq!(p.variant, BadgeVariant::Dot as i32);
    }

    #[tokio::test]
    async fn set_status_warning() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        b.set(BadgeKind::Status {
            status: BadgeStatus::Warning,
            value: None,
        })
        .await
        .unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert_eq!(p.variant, BadgeVariant::Status as i32);
        assert_eq!(p.status, ProtoBadgeStatus::Warning as i32);
        assert_eq!(p.progress_value, None);
    }

    #[tokio::test]
    async fn set_status_progress_with_value() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        b.set(BadgeKind::Status {
            status: BadgeStatus::Progress,
            value: Some(0.67),
        })
        .await
        .unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert_eq!(p.status, ProtoBadgeStatus::Progress as i32);
        assert!((p.progress_value.unwrap() - 0.67).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn set_status_rejects_value_when_not_progress() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        // Codex C10: value attached to non-progress status
        // must be rejected at the SDK boundary.
        for bad_status in [
            BadgeStatus::Success,
            BadgeStatus::Warning,
            BadgeStatus::Error,
        ] {
            let err = b
                .set(BadgeKind::Status {
                    status: bad_status,
                    value: Some(0.4),
                })
                .await
                .unwrap_err();
            assert!(matches!(err, EmitError::SerializationFailed(_)));
        }
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_progress_rejects_value_out_of_range() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        for bad in [-0.1f32, 1.1, 2.5, -1.0] {
            let err = b
                .set(BadgeKind::Status {
                    status: BadgeStatus::Progress,
                    value: Some(bad),
                })
                .await
                .unwrap_err();
            match err {
                EmitError::SerializationFailed(msg) => {
                    assert!(
                        msg.contains("[0.0, 1.0]"),
                        "error must mention range: {msg}"
                    );
                }
                other => panic!("expected SerializationFailed, got {other:?}"),
            }
        }
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_progress_accepts_value_at_boundaries() {
        // 0.0 and 1.0 are explicitly inclusive boundaries.
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        for v in [0.0f32, 1.0] {
            b.set(BadgeKind::Status {
                status: BadgeStatus::Progress,
                value: Some(v),
            })
            .await
            .expect("boundary value must be accepted");
        }
        assert_eq!(emitter.emit_count().await, 2);
    }

    #[tokio::test]
    async fn set_progress_rejects_nan() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        let err = b
            .set(BadgeKind::Status {
                status: BadgeStatus::Progress,
                value: Some(f32::NAN),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, EmitError::SerializationFailed(_)));
        assert_eq!(emitter.emit_count().await, 0);
    }

    #[tokio::test]
    async fn set_count_with_status_combo() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        b.set(BadgeKind::CountWithStatus {
            count: 2,
            status: BadgeStatus::Warning,
        })
        .await
        .unwrap();
        let p = decode_set(&emitter.emitted().await[0].payload);
        assert_eq!(p.variant, BadgeVariant::CountWithStatus as i32);
        assert_eq!(p.count, 2);
        assert_eq!(p.status, ProtoBadgeStatus::Warning as i32);
    }

    #[tokio::test]
    async fn clear_emits_dedicated_event() {
        let emitter = MockEventEmitter::new();
        let b = Badges::new(emitter.clone(), "app");
        b.clear().await.unwrap();
        let events = emitter.emitted().await;
        assert_eq!(events[0].event_type, "app.badge.cleared");
    }
}
