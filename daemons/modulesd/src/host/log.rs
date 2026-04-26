/// `lunaris:host/log` import implementation.
///
/// Logging is the only host capability that is unconditionally
/// available to every module. Output is tagged with the module ID and
/// surfaced in Settings so users can see what their modules are
/// chattering about.

use tracing::{error, info, warn};

use crate::host::CapabilityContext;

pub fn log_info(ctx: &CapabilityContext, message: &str) {
    info!(module = %ctx.module_id, "{message}");
}

pub fn log_warn(ctx: &CapabilityContext, message: &str) {
    warn!(module = %ctx.module_id, "{message}");
}

pub fn log_error(ctx: &CapabilityContext, message: &str) {
    error!(module = %ctx.module_id, "{message}");
}
