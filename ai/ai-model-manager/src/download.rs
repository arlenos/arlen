//! Model-download resolution: turn a catalog [`crate::ModelSpec`]'s `source`
//! (a Hugging Face GGUF repo, e.g. `bartowski/Llama-3.2-1B-Instruct-GGUF`) plus a
//! chosen [`crate::Quant`] into the concrete GGUF file URL to fetch and the local
//! path to store it at.
//!
//! This is the PURE resolution half of the consent-gated downloader
//! (`local-model-bundle-plan.md`): no network, no filesystem. The fetch itself
//! (the one explicit, consented outbound a no-telemetry OS makes) is performed by
//! the I/O layer through the SSRF-safe egress + sha256 verify (`arlen-forage-fetch`),
//! never here - keeping URL/path construction unit-testable and free of the egress
//! stack.
//!
//! The catalog sources are all `bartowski/<Model>-GGUF` repos, whose files follow
//! the llama.cpp convention `<Model>-<QUANT>.gguf`. The resolver encodes exactly
//! that convention; an arbitrary HF repo (the future "Advanced" door) would need
//! the HF file-listing API instead, out of scope for the curated catalog.

use std::path::PathBuf;

use crate::Quant;

/// The Hugging Face host all catalog models resolve from. No registry gravity
/// (no ollama.com), raw GGUF over plain HTTPS (`local-model-bundle-plan.md`).
const HF_HOST: &str = "https://huggingface.co";

/// Whether `source` is a safe Hugging Face repo id of the form `owner/name`:
/// exactly one `/`, both parts non-empty, every character in `[A-Za-z0-9._-]`,
/// and no path-traversal component (`.`/`..`). The catalog is trusted, but the
/// `source` is interpolated into a URL and a filename, so it is validated rather
/// than trusted - a stray `/` or `..` could escape the resolve path or the store
/// dir.
pub fn is_valid_hf_repo(source: &str) -> bool {
    let mut parts = source.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [owner, name].into_iter().all(|p| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    })
}

/// The model base name of a `bartowski/<Model>-GGUF` source: the last path
/// segment with the `-GGUF` suffix stripped (e.g. `Llama-3.2-1B-Instruct`).
/// `None` if `source` is not a valid repo id or does not end in `-GGUF` (not a
/// GGUF repo this convention can resolve).
fn gguf_base(source: &str) -> Option<&str> {
    if !is_valid_hf_repo(source) {
        return None;
    }
    let last = source.rsplit('/').next()?;
    let base = last.strip_suffix("-GGUF")?;
    (!base.is_empty()).then_some(base)
}

/// The GGUF filename for `source` at `quant` under the bartowski convention,
/// e.g. `Llama-3.2-1B-Instruct-Q4_K_M.gguf`. `None` for a non-resolvable source.
pub fn gguf_filename(source: &str, quant: Quant) -> Option<String> {
    let base = gguf_base(source)?;
    Some(format!("{base}-{}.gguf", quant.gguf_tag()))
}

/// The Hugging Face `resolve` URL the GGUF is fetched from (the `main` revision),
/// e.g. `https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/\
/// Llama-3.2-1B-Instruct-Q4_K_M.gguf`. `None` for a non-resolvable source.
pub fn gguf_resolve_url(source: &str, quant: Quant) -> Option<String> {
    let file = gguf_filename(source, quant)?;
    Some(format!("{HF_HOST}/{source}/resolve/main/{file}"))
}

/// The per-user model store directory (`$XDG_DATA_HOME/arlen/models`, else
/// `$HOME/.local/share/arlen/models`) where a downloaded GGUF is written. The
/// baked offline default lives at the system `/usr/share/arlen/models`; a
/// consented download is the user's, so it goes under the user data dir. `None`
/// when neither env var is set (the caller fails closed rather than guess).
pub fn model_store_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("arlen/models"));
    }
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(home).join(".local/share/arlen/models"))
}

/// The local path a downloaded GGUF for `source` at `quant` is stored at:
/// [`model_store_dir`] joined with [`gguf_filename`]. `None` if the store dir is
/// unresolvable or the source is not a GGUF repo.
pub fn local_model_path(source: &str, quant: Quant) -> Option<PathBuf> {
    Some(model_store_dir()?.join(gguf_filename(source, quant)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "bartowski/Llama-3.2-1B-Instruct-GGUF";

    #[test]
    fn valid_repo_accepts_well_formed_and_rejects_unsafe() {
        assert!(is_valid_hf_repo(SRC));
        assert!(is_valid_hf_repo("bartowski/Qwen2.5-7B-Instruct-GGUF"));
        assert!(!is_valid_hf_repo("no-slash"));
        assert!(!is_valid_hf_repo("a/b/c")); // more than one segment
        assert!(!is_valid_hf_repo("/name")); // empty owner
        assert!(!is_valid_hf_repo("owner/")); // empty name
        assert!(!is_valid_hf_repo("owner/..")); // traversal
        assert!(!is_valid_hf_repo("owner/a b")); // space (would break the URL)
        assert!(!is_valid_hf_repo("owner/a\nb")); // control char
    }

    #[test]
    fn filename_follows_the_bartowski_convention() {
        assert_eq!(
            gguf_filename(SRC, Quant::Q4KM).as_deref(),
            Some("Llama-3.2-1B-Instruct-Q4_K_M.gguf")
        );
        assert_eq!(
            gguf_filename("bartowski/Qwen2.5-Coder-7B-Instruct-GGUF", Quant::Q6K).as_deref(),
            Some("Qwen2.5-Coder-7B-Instruct-Q6_K.gguf")
        );
        // Not a GGUF repo -> unresolvable by this convention.
        assert_eq!(gguf_filename("meta-llama/Llama-3.2-1B", Quant::Q4KM), None);
        assert_eq!(gguf_filename("bad source", Quant::Q4KM), None);
    }

    #[test]
    fn resolve_url_is_the_hf_main_revision() {
        assert_eq!(
            gguf_resolve_url(SRC, Quant::Q4KM).as_deref(),
            Some(
                "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/\
                 Llama-3.2-1B-Instruct-Q4_K_M.gguf"
            )
        );
        assert_eq!(gguf_resolve_url("no-slash", Quant::Q4KM), None);
    }

    #[test]
    fn local_path_joins_store_dir_and_filename() {
        // Deterministic via XDG_DATA_HOME; restore after.
        let prev = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/data");
        assert_eq!(
            local_model_path(SRC, Quant::Q4KM),
            Some(PathBuf::from("/data/arlen/models/Llama-3.2-1B-Instruct-Q4_K_M.gguf"))
        );
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }
}
