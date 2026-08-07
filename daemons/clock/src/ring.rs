//! Saying that a moment arrived.
//!
//! The clock decides *what* is worth saying and how loudly; the notification
//! daemon decides how it is shown. The text is a pure function of what came due
//! so it is unit-tested without a bus, and the send is the thin part around it -
//! the same shape the power daemon's battery notifications use.
//!
//! **Only an alarm is critical.** Critical is the one tier that pierces
//! Do-Not-Disturb, and an alarm that Do-Not-Disturb swallows is not an alarm: a
//! person set it deliberately, for a moment they chose, and silence at that
//! moment is the single way a clock can fail completely. A timer running out or
//! a focus phase ending is ordinary news - worth seeing, not worth overriding a
//! decision to be left alone.

use crate::state::{Alarm, FocusPhase, FocusSession, Timer};

/// freedesktop urgency levels for the notification `urgency` hint.
const URGENCY_NORMAL: u8 = 1;
const URGENCY_CRITICAL: u8 = 2;

/// A notification to raise: summary, body, urgency.
pub type Notification = (String, String, u8);

/// What to say when an alarm's moment arrives.
///
/// The label leads when there is one, because that is what the person wrote to
/// recognise this alarm by; the time leads when there is not, since "Alarm" on
/// its own tells someone with three alarms nothing.
pub fn for_alarm(alarm: &Alarm) -> Notification {
    let label = alarm.label.trim();
    if label.is_empty() {
        (
            format!("Alarm - {}", alarm.time),
            String::new(),
            URGENCY_CRITICAL,
        )
    } else {
        (
            label.to_string(),
            format!("Alarm - {}", alarm.time),
            URGENCY_CRITICAL,
        )
    }
}

/// What to say when a timer runs out.
pub fn for_timer(timer: &Timer) -> Notification {
    (
        "Timer finished".to_string(),
        humanise(timer.duration_ms),
        URGENCY_NORMAL,
    )
}

/// What to say when a focus phase ends, given the session it became.
///
/// `None` is the session finishing. The break and the next round read
/// differently on purpose: one is permission to stop, the other is not.
pub fn for_focus(next: Option<&FocusSession>) -> Notification {
    match next {
        Some(s) if s.phase == FocusPhase::Break => (
            "Time for a break".to_string(),
            format!("Round {} of {} done.", s.round, s.rounds),
            URGENCY_NORMAL,
        ),
        Some(s) => (
            "Break over".to_string(),
            format!("Round {} of {}.", s.round, s.rounds),
            URGENCY_NORMAL,
        ),
        None => (
            "Focus session finished".to_string(),
            String::new(),
            URGENCY_NORMAL,
        ),
    }
}

/// A duration as a person would say it.
///
/// Whole units only, largest first, and never zero of anything: "1 hr 30 min",
/// not "1 hr 30 min 0 sec". A duration below a second reads as "less than a
/// second" rather than as an empty string, so a caller never renders nothing.
fn humanise(ms: i64) -> String {
    let total = ms.max(0) / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h} hr"));
    }
    if m > 0 {
        parts.push(format!("{m} min"));
    }
    if s > 0 {
        parts.push(format!("{s} sec"));
    }
    if parts.is_empty() {
        return "less than a second".to_string();
    }
    parts.join(" ")
}

/// Raise one notification over the session bus.
///
/// Best-effort by design: a clock whose notification daemon is down still keeps
/// time, still advances, still arms the next wake. Losing the announcement is
/// bad and is logged; refusing to keep time over it would be worse.
pub async fn send(conn: &zbus::Connection, (summary, body, urgency): Notification) {
    let proxy = match zbus::Proxy::new(
        conn,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("nothing to announce {summary:?} with: {e}");
            return;
        }
    };
    let hints: std::collections::HashMap<&str, zbus::zvariant::Value> =
        std::collections::HashMap::from([
            ("urgency", zbus::zvariant::Value::U8(urgency)),
            // The category the notification daemon reads for its own routing.
            ("category", zbus::zvariant::Value::new("x-arlen.clock")),
        ]);
    // Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout).
    // Timeout 0 means the daemon's own policy decides; a clock has no business
    // pinning how long its own message stays up.
    let reply: zbus::Result<u32> = proxy
        .call(
            "Notify",
            &(
                "Clock",
                0u32,
                "alarm-symbolic",
                summary.as_str(),
                body.as_str(),
                Vec::<&str>::new(),
                hints,
                0i32,
            ),
        )
        .await;
    if let Err(e) = reply {
        tracing::warn!("could not announce {summary:?}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(label: &str) -> Alarm {
        Alarm {
            id: "a".into(),
            time: "07:00".into(),
            label: label.into(),
            days: vec![],
            enabled: true,
            fire_late: true,
            next_fire_at: None,
        }
    }

    /// An alarm is the one thing here allowed to pierce Do-Not-Disturb, and the
    /// only one that should. If this ever flips, an alarm becomes silent for
    /// anyone who turned DND on the night before - the complete failure.
    #[test]
    fn only_an_alarm_is_critical() {
        assert_eq!(for_alarm(&alarm("Wake up")).2, URGENCY_CRITICAL);
        assert_eq!(
            for_timer(&Timer {
                id: "t".into(),
                duration_ms: 60_000,
                ends_at: None,
                remaining_ms: None,
                paused: false,
            })
            .2,
            URGENCY_NORMAL
        );
        assert_eq!(for_focus(None).2, URGENCY_NORMAL);
    }

    /// Someone with three alarms learns nothing from "Alarm", so the label they
    /// wrote leads when there is one and the time leads when there is not.
    #[test]
    fn an_alarm_names_itself_by_whichever_it_has() {
        let (summary, body, _) = for_alarm(&alarm("Wake up"));
        assert_eq!(summary, "Wake up");
        assert!(body.contains("07:00"));

        let (summary, _, _) = for_alarm(&alarm(""));
        assert!(summary.contains("07:00"), "the time carries it instead");
    }

    /// A label of nothing but spaces is no label; it must not produce a blank
    /// notification.
    #[test]
    fn a_blank_label_falls_back_to_the_time() {
        let (summary, _, _) = for_alarm(&alarm("   "));
        assert!(summary.contains("07:00"));
    }

    #[test]
    fn a_break_and_the_next_round_do_not_read_alike() {
        let session = |phase| FocusSession {
            phase,
            round: 2,
            rounds: 4,
            ends_at: 0,
            held: vec![],
        };
        let brk = for_focus(Some(&session(FocusPhase::Break)));
        let work = for_focus(Some(&session(FocusPhase::Focus)));
        assert_ne!(brk.0, work.0, "one is permission to stop, the other is not");
        assert_ne!(for_focus(None).0, brk.0);
    }

    #[test]
    fn a_duration_reads_the_way_it_was_set() {
        assert_eq!(humanise(25 * 60_000), "25 min");
        assert_eq!(humanise(90 * 60_000), "1 hr 30 min");
        assert_eq!(humanise(45_000), "45 sec");
        assert_eq!(humanise(3_600_000), "1 hr");
    }

    /// Never an empty string and never a zero unit: a caller renders whatever
    /// comes back, so "" would be a blank notification body.
    #[test]
    fn no_duration_ever_renders_as_nothing() {
        assert_eq!(humanise(0), "less than a second");
        assert_eq!(humanise(-5), "less than a second");
        assert_eq!(humanise(999), "less than a second");
        assert!(!humanise(3_600_000).contains('0'), "no zero units");
    }
}
