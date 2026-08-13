//! Which of the session's children may register identities of their own.
//!
//! The session is the root of trust for a user session: it is what the login path
//! starts, exactly once, and everything else descends from it. So it is also the
//! party that can say what those descendants ARE - and saying it is what makes an
//! identity survive a reader that cannot look for itself. A daemon would rather
//! just resolve its peer (read `/proc/<pid>/exe`, map the path to an app id), and
//! that stops working in the units most worth hardening: `ProtectSystem=strict`
//! refuses a same-uid peer's exe link, which is the `readlinkat(exe): Permission
//! denied` the undo signer hits on every boot. A stamp registered at spawn is a
//! fact recorded while it was still readable, answered later by lookup.
//!
//! The stamping itself is [`arlen_permissions::identity::app_id_for_program`],
//! shared with the shell, because the id must come from the binary rather than
//! from a name the caller chose. What is local to the session is the POLICY here:
//! which child gets the right to stamp in turn.
//!
//! Best-effort throughout, like the launcher's: a broker that is down or a program
//! that cannot be resolved must never stop the session from starting. The child
//! then resolves the old way, which is exactly the state this improves on.

/// The children of the session that may themselves register identities.
///
/// The session grants the right to ONE of the three things it starts, because one
/// of them launches apps: the shell spawns `arlen-run` per launch, and `arlen-run`
/// is what stamps the app. The compositor and the boot-verify app start nothing and
/// get no such right.
///
/// This IS a list of names, and it is a different kind from the one it replaces:
/// the session is naming its OWN children - programs it is about to spawn itself -
/// not deciding what a stranger presenting a name is allowed to do. A caller cannot
/// put itself on this list; only editing the session can, and the session is the
/// root of trust already.
pub const REGISTRAR_CHILDREN: &[&str] = &["arlen-desktop-shell"];

/// Whether `program`, as the session is about to spawn it, gets the right to
/// register identities of its own.
pub fn grants_registrar(program: &str) -> bool {
    REGISTRAR_CHILDREN.contains(&program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_child_that_launches_apps_may_register() {
        // The compositor and the verify app start nothing, so neither has any
        // reason to stamp an identity. Keeping the grant to one child is what
        // makes the two-level bound worth having at all.
        assert!(grants_registrar("arlen-desktop-shell"));
        assert!(!grants_registrar("arlen-compositor"));
        assert!(!grants_registrar("arlen-boot-verify"));
    }
}
