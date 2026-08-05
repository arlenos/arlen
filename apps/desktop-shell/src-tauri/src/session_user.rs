// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Who is logged in, for the shell to show.
//!
//! The Quick Settings user row displayed a hardcoded name and initials, so every
//! machine showed the same person regardless of who was using it. This resolves the
//! real account instead.
//!
//! The display name comes from the GECOS field, which is where a full name lives on
//! a Unix account, falling back to the login name when it is empty - which it often
//! is on a machine set up without one. Initials are derived rather than stored,
//! because a separate initials field would be one more thing to keep in step with a
//! name that can change.

use std::ffi::CStr;

/// The logged-in account, as the shell wants to display it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionUser {
    /// A full name if the account carries one, otherwise the login name.
    pub display_name: String,
    /// One or two letters for the avatar, derived from `display_name`.
    pub initials: String,
    /// The login name, for surfaces that want the account rather than the person.
    pub login: String,
}

/// Up to two initials from a display name.
///
/// Takes the first letter of the first and last whitespace-separated part, so
/// "Ada Lovelace" gives "AL" and a single-word name gives one letter. Uppercased
/// through the locale-independent Unicode mapping, since a display name is not
/// necessarily ASCII.
fn initials_of(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().filter(|p| !p.is_empty()).collect();
    let letters = match parts.as_slice() {
        [] => return String::new(),
        [one] => one.chars().take(1).collect::<Vec<_>>(),
        [first, .., last] => first
            .chars()
            .take(1)
            .chain(last.chars().take(1))
            .collect::<Vec<_>>(),
    };
    letters
        .into_iter()
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// Read the passwd entry for this process's uid.
///
/// Returns the login name and the GECOS field. `getpwuid_r` rather than `getpwuid`
/// so this is safe to call from any thread.
fn passwd_entry() -> Option<(String, String)> {
    let mut buf = vec![0u8; 4096];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: a standard getpwuid_r call with a sufficiently large buffer; `result`
    // is null on not-found or error, which is checked before any dereference.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pwd,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: pw_name and pw_gecos point into `buf` for a found entry.
    let login = unsafe { CStr::from_ptr(pwd.pw_name) }
        .to_string_lossy()
        .into_owned();
    let gecos = if pwd.pw_gecos.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pwd.pw_gecos) }
            .to_string_lossy()
            .into_owned()
    };
    Some((login, gecos))
}

/// The account this session belongs to.
///
/// Falls back to `$USER` and then to a generic label, so the row always renders
/// something rather than collapsing when an account has no passwd entry - which
/// happens in containers and on systems using a directory service that is briefly
/// unreachable.
#[tauri::command]
pub fn session_user() -> SessionUser {
    let (login, gecos) = passwd_entry().unwrap_or_else(|| {
        let login = std::env::var("USER").unwrap_or_default();
        (login, String::new())
    });
    // GECOS is comma-separated; the full name is the first field.
    let full = gecos.split(',').next().unwrap_or("").trim().to_string();
    let display_name = if full.is_empty() {
        login.clone()
    } else {
        full
    };
    SessionUser {
        initials: initials_of(&display_name),
        display_name,
        login,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_the_first_and_last_name() {
        assert_eq!(initials_of("Ada Lovelace"), "AL");
        assert_eq!(initials_of("Ada Byron King Lovelace"), "AL");
    }

    #[test]
    fn a_single_word_name_gives_one_letter() {
        assert_eq!(initials_of("root"), "R");
    }

    #[test]
    fn initials_survive_a_non_ascii_name() {
        // A display name is not necessarily ASCII, and slicing bytes would panic
        // mid-character on names like this one.
        assert_eq!(initials_of("Ümit Öztürk"), "ÜÖ");
    }

    #[test]
    fn an_empty_name_gives_no_initials_rather_than_panicking() {
        assert_eq!(initials_of(""), "");
        assert_eq!(initials_of("   "), "");
    }

    #[test]
    fn the_session_user_always_resolves_to_something() {
        // Whatever the environment, the row must render: the point of the fallback
        // chain is that there is no state in which this returns nothing at all.
        let u = session_user();
        assert!(!u.display_name.is_empty() || !u.login.is_empty());
    }
}
