/// `arlen:host/events` import implementation.
///
/// Modules call `events::emit(event_type, payload)` to send events to
/// the system Event Bus. The host gates by event-type prefix
/// (`focus.`, `module.com.example.`, etc.) declared in
/// `event_bus.publish`. Cross-module observation is gated by
/// `event_bus.subscribe` symmetrically.

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;

pub fn check_publish(ctx: &CapabilityContext, event_type: &str) -> Result<()> {
    if !ctx.allow_event_publish(event_type) {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("events.publish({event_type})"),
        });
    }
    Ok(())
}

pub fn check_subscribe(ctx: &CapabilityContext, event_type: &str) -> Result<()> {
    if !ctx.allow_event_subscribe(event_type) {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("events.subscribe({event_type})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::{EventBusCapability, ModuleCapabilities};

    fn ctx(publish: Vec<&str>, subscribe: Vec<&str>) -> CapabilityContext {
        let mut caps = ModuleCapabilities::default();
        caps.event_bus = Some(EventBusCapability {
            publish: publish.into_iter().map(String::from).collect(),
            subscribe: subscribe.into_iter().map(String::from).collect(),
        });
        CapabilityContext::new("x", caps)
    }

    #[test]
    fn publish_in_allowlist_passes() {
        let c = ctx(vec!["module.com.example."], vec![]);
        assert!(check_publish(&c, "module.com.example.refreshed").is_ok());
    }

    #[test]
    fn publish_outside_allowlist_denied() {
        let c = ctx(vec!["module.com.example."], vec![]);
        assert!(check_publish(&c, "system.shutdown").is_err());
    }

    #[test]
    fn subscribe_independent_of_publish() {
        let c = ctx(vec![], vec!["focus."]);
        assert!(check_subscribe(&c, "focus.activated").is_ok());
        assert!(check_publish(&c, "focus.activated").is_err());
    }
}
