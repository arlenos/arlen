//! The Arlen power daemon (`org.arlen.Power1`, system-services-plan.md Part A).
//!
//! Aggregates UPower battery/AC/lid state into a coarse [`power::PowerState`]
//! and publishes `power.state` on the event bus (PWR-R1), so the shell's
//! battery indicator and the AI layer consume one shared value instead of
//! each forking UPower. It is the future home for logind suspend/idle policy
//! (PWR-R2/R3/R4), power profiles (PWR-R5), critical-battery action (PWR-R6),
//! and the capability gate on those actions (PWR-R7).
//!
//! The state-aggregation core is pure and unit-tested ([`power`]); the daemon
//! binary wires it to the live system bus and the event-bus producer.

pub mod battery;
pub mod dbus;
pub mod lid;
pub mod logind;
pub mod notify;
pub mod power;
pub mod profiles;
