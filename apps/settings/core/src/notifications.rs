//! DND-expiry timestamp math the settings notifications commands wrap: the
//! ISO-8601 UTC `expires_at` values the notification daemon compares against
//! `Utc::now()` for the "DND for N hours" / "until tomorrow morning" Quick
//! Actions. Pure time math, unit-tested in CI; the commands that write these
//! into `notifications.toml` stay in the host.

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};

/// An ISO-8601 UTC `expires_at` `seconds` into the future (negative clamps to
/// now). A full RFC-3339 timestamp the daemon's `is_expired()` compares against
/// `Utc::now()`.
pub fn dnd_expiry_in(seconds: i64) -> String {
    let when = Utc::now() + Duration::seconds(seconds.max(0));
    when.to_rfc3339()
}

/// An ISO-8601 UTC `expires_at` for "until tomorrow morning": the next 07:00 in
/// the user's local timezone.
pub fn dnd_expiry_until_morning() -> Result<String, String> {
    let now = Local::now();
    let target_time = NaiveTime::from_hms_opt(7, 0, 0).ok_or("invalid time")?;
    let mut target_date = now.date_naive();
    if now.time() >= target_time {
        target_date = target_date.succ_opt().ok_or("date overflow")?;
    }
    let local_dt = target_date.and_time(target_time);
    let local = Local
        .from_local_datetime(&local_dt)
        .single()
        .ok_or("ambiguous local time")?;
    let utc: DateTime<Utc> = local.with_timezone(&Utc);
    Ok(utc.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_in_positive() {
        let result = dnd_expiry_in(3600);
        let parsed = DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = parsed.with_timezone(&Utc) - Utc::now();
        // Should be ~3600s in the future (allow 5s tolerance for slow CI).
        assert!(
            diff.num_seconds() >= 3595 && diff.num_seconds() <= 3605,
            "expected ~3600s, got {}s",
            diff.num_seconds()
        );
    }

    #[test]
    fn expiry_in_zero() {
        let result = dnd_expiry_in(0);
        let parsed = DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = (parsed.with_timezone(&Utc) - Utc::now()).num_seconds().abs();
        assert!(diff <= 2, "0s expiry should be ~now, got {diff}s off");
    }

    #[test]
    fn expiry_in_negative_clamped_to_zero() {
        let result = dnd_expiry_in(-100);
        let parsed = DateTime::parse_from_rfc3339(&result).unwrap();
        let diff = (parsed.with_timezone(&Utc) - Utc::now()).num_seconds().abs();
        assert!(diff <= 2, "negative should clamp to 0 (now), got {diff}s off");
    }

    #[test]
    fn expiry_until_morning_valid_rfc3339() {
        let result = dnd_expiry_until_morning().unwrap();
        assert!(
            DateTime::parse_from_rfc3339(&result).is_ok(),
            "should be valid RFC-3339: {result}"
        );
    }

    #[test]
    fn expiry_until_morning_is_future() {
        let result = dnd_expiry_until_morning().unwrap();
        let parsed = DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&Utc);
        assert!(parsed > Utc::now(), "morning expiry should be in the future");
    }
}
