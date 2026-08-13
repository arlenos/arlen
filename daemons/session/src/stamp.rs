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

/// The programs the session starts, named once.
///
/// Once, because the name is load-bearing twice: it is what gets spawned AND what
/// decides who may register. Two copies of a string with that job is a silent
/// failure waiting - a rename in one place leaves the chain compiling, booting and
/// stamping nothing, which reads as a broker problem.
pub const COMPOSITOR: &str = "arlen-compositor";
/// The shell, one of the two children granted the right to register in turn.
pub const SHELL: &str = "arlen-desktop-shell";
/// The supervisor, which registers the per-user daemons systemd starts.
pub const SUPERVISOR: &str = "arlen-session-supervisor";

/// The children of the session that may themselves register identities.
///
/// Two of the four things it starts, because two of them start other things. The
/// shell spawns `arlen-run` per launch, and `arlen-run` is what stamps the app. The
/// supervisor stamps the per-user daemons, which systemd starts and which cannot be
/// read from their own `/proc` once they are hardened. The compositor and the
/// boot-verify app start nothing and get no such right.
///
/// This IS a list of names, and it is a different kind from the one it replaces:
/// the session is naming its OWN children - programs it is about to spawn itself -
/// not deciding what a stranger presenting a name is allowed to do. A caller cannot
/// put itself on this list; only editing the session can, and the session is the
/// root of trust already.
pub const REGISTRAR_CHILDREN: &[&str] = &[SHELL, SUPERVISOR];

/// What `program`, as the session is about to spawn it, is granted.
///
/// The supervisor also gets the right to stamp RESERVED ids, and the shell does
/// not: the supervisor attests shipped daemons (`arlen-ai-engine-daemon.service`
/// IS `ai-agent`), while the shell's chain ends at `arlen-run`, which stamps user
/// apps whose ids are reverse-DNS. Keeping the second right off the launcher is
/// what makes a compromised launcher unable to mint `settings`.
pub fn grants_for(program: &str) -> arlen_permissions::identity_store::Grants {
    arlen_permissions::identity_store::Grants {
        register: REGISTRAR_CHILDREN.contains(&program),
        stamp_reserved: program == SUPERVISOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_children_that_start_things_may_register() {
        // The compositor and the verify app start nothing, so neither has any
        // reason to stamp an identity. Keeping the grant to the two that do is
        // what makes the two-level bound worth having at all.
        assert!(grants_for(SHELL).register);
        assert!(grants_for(SUPERVISOR).register);
        assert!(!grants_for(COMPOSITOR).register);
        assert!(!grants_for("arlen-boot-verify").register);
    }

    #[test]
    fn only_the_supervisor_may_stamp_a_reserved_id() {
        // The shell's chain ends at the launcher, which stamps user apps; a
        // reserved id from there is a bypass attempt and the broker refuses it.
        // The supervisor's units genuinely carry reserved ids, so it alone is
        // granted this - by the root, at spawn, rather than by anything it says.
        assert!(grants_for(SUPERVISOR).stamp_reserved);
        assert!(!grants_for(SHELL).stamp_reserved);
        assert!(!grants_for(COMPOSITOR).stamp_reserved);
    }
}
