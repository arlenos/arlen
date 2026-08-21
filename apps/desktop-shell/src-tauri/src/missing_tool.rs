//! What to say when a helper program is not on this machine.
//!
//! The shell reaches for `nmcli`, `pactl`, `wpctl`, `rfkill`, `powerprofilesctl`,
//! `wl-copy`, `wl-paste` and `wf-recorder`, and `dev/scripts/runtime-deps.tsv`
//! records every one of them as ABSENT from the image we build. So on the machine
//! we ship, these are not rare failures: they are the ordinary state, and what a
//! person reads when they flip a switch is whatever this produces.
//!
//! What they used to read was the errno. `nmcli connect failed: No such file or
//! directory (os error 2)` says the connection failed and names a file nobody
//! mentioned; `wpctl not found: ...` names a program the person has never heard of
//! and does not say what stopped working. Neither tells them the machine cannot do
//! this at all, which is the one fact that matters, and both read as a fault to
//! retry.
//!
//! So a message here names the CAPABILITY, then the program, in that order.

/// Turn a failure to start a helper program into a sentence.
///
/// `does` completes "so ...": what the person cannot do, in their terms.
pub fn tool_error(tool: &str, does: &str, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!("{does}: {tool} is not installed on this machine")
    } else {
        // A different failure is a fault rather than an absence, and the errno is
        // worth keeping there: it is the difference between a machine that cannot
        // and a machine that would not.
        format!("{does}: {tool} could not be run ({e})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_tool_says_what_stopped_working_before_it_names_the_program() {
        let e = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let said = tool_error("nmcli", "the network cannot be changed from here", &e);
        assert!(
            said.starts_with("the network cannot be changed from here"),
            "{said}"
        );
        assert!(said.contains("nmcli is not installed"), "{said}");
        assert!(
            !said.contains("os error"),
            "the errno adds nothing here: {said}"
        );
    }

    #[test]
    fn any_other_failure_keeps_the_reason() {
        // Present and refusing is a different thing from absent, and the person
        // fixing it needs the reason.
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let said = tool_error("rfkill", "airplane mode cannot be switched", &e);
        assert!(said.contains("could not be run"), "{said}");
        assert!(said.contains("denied"), "{said}");
    }
}
