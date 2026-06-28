//! Save an artifact to a user-chosen file - the artifact context-menu "Save"
//! action.
//!
//! Powerbox principle: the harness never holds blanket filesystem write
//! authority. The USER picks the destination through the system save dialog
//! (kdialog, then zenity), and only that one chosen path is written. The dialog
//! is seeded with a kind-appropriate filename so the common case is one click.
//!
//! The content is the artifact's structured source where it has one
//! (markdown/code/diagram), the decoded bytes for an image, or the mandatory
//! plain-text floor otherwise (Table/Chart/Links). The `Artifact` invariant
//! guarantees `text` is non-empty, so there is always something to save; we
//! never invent CSV escaping for a table here (the floor is the faithful,
//! escape-safe serialisation).

use std::process::Command;

use arlen_artifact::{Artifact, ArtifactPayload, DiagramLanguage, ImageMediaType};
use base64::Engine;

/// The bytes to write and a suggested filename for `artifact`. Pure, so the
/// kind to content/extension mapping is unit-tested without a dialog or disk.
fn save_bytes_and_name(artifact: &Artifact) -> (Vec<u8>, String) {
    match &artifact.payload {
        ArtifactPayload::Markdown { source } => {
            (source.clone().into_bytes(), "artifact.md".to_string())
        }
        ArtifactPayload::Code { source, language } => (
            source.clone().into_bytes(),
            format!("artifact.{}", code_extension(language.as_deref())),
        ),
        ArtifactPayload::Diagram { language, source } => (
            source.clone().into_bytes(),
            format!("artifact.{}", diagram_extension(*language)),
        ),
        ArtifactPayload::Image {
            media_type,
            data_base64,
        } => match base64::engine::general_purpose::STANDARD.decode(data_base64) {
            Ok(bytes) => (bytes, format!("artifact.{}", image_extension(*media_type))),
            // An undecodable image still saves its human-readable floor rather
            // than failing the whole action.
            Err(_) => (artifact.text.clone().into_bytes(), "artifact.txt".to_string()),
        },
        // Table / Chart / Links: the text floor is the faithful serialisation.
        _ => (artifact.text.clone().into_bytes(), "artifact.txt".to_string()),
    }
}

/// Filename extension for a code artifact, from its optional language hint.
/// Unknown or absent hints fall back to `txt` (the source is still saved
/// verbatim, just without a language-specific suffix).
fn code_extension(language: Option<&str>) -> &'static str {
    match language.map(str::to_ascii_lowercase).as_deref() {
        Some("rust" | "rs") => "rs",
        Some("python" | "py") => "py",
        Some("javascript" | "js") => "js",
        Some("typescript" | "ts") => "ts",
        Some("bash" | "sh" | "shell") => "sh",
        Some("json") => "json",
        Some("toml") => "toml",
        Some("yaml" | "yml") => "yaml",
        Some("c") => "c",
        Some("cpp" | "c++") => "cpp",
        Some("go") => "go",
        Some("java") => "java",
        Some("html") => "html",
        Some("css") => "css",
        Some("sql") => "sql",
        Some("markdown" | "md") => "md",
        _ => "txt",
    }
}

/// Filename extension for a diagram source by its language.
fn diagram_extension(language: DiagramLanguage) -> &'static str {
    match language {
        DiagramLanguage::Mermaid => "mmd",
        DiagramLanguage::Dot => "dot",
    }
}

/// Filename extension for an image by its media type.
fn image_extension(media_type: ImageMediaType) -> &'static str {
    match media_type {
        ImageMediaType::Png => "png",
        ImageMediaType::Jpeg => "jpg",
        ImageMediaType::Svg => "svg",
        ImageMediaType::Webp => "webp",
        ImageMediaType::Gif => "gif",
    }
}

/// Ask the user for a destination path via kdialog, then zenity. Returns the
/// chosen absolute path, or `None` on cancel or when no chooser is installed.
/// The dialog is seeded with `start_dir/suggested` so the default is a sensible
/// filename in the downloads folder.
fn pick_save_path(suggested: &str, start_dir: &str) -> Option<String> {
    let seed = if start_dir.ends_with('/') {
        format!("{start_dir}{suggested}")
    } else {
        format!("{start_dir}/{suggested}")
    };

    if let Some(path) = try_kdialog_save(&seed) {
        return Some(path);
    }
    if let Some(path) = try_zenity_save(&seed) {
        return Some(path);
    }
    log::warn!("artifact_save: no file chooser found (kdialog and zenity not installed)");
    None
}

/// kdialog save dialog. Cancel is a non-zero exit, which reads as `None`.
fn try_kdialog_save(seed: &str) -> Option<String> {
    let output = Command::new("kdialog")
        .args(["--getsavefilename", seed])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// zenity save dialog. `--confirm-overwrite` so an existing file prompts before
/// it is replaced by the chosen path.
fn try_zenity_save(seed: &str) -> Option<String> {
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--save",
            "--confirm-overwrite",
            "--title=Save artifact",
            &format!("--filename={seed}"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Save `artifact` to a file the user chooses. Returns the written path on
/// success, or `None` if the user cancelled the dialog (or no chooser is
/// installed). The path is the user's explicit choice through the system save
/// dialog, so writing it carries the user's authority (the powerbox model);
/// the harness never picks a path itself.
#[tauri::command]
pub async fn artifact_save(artifact: Artifact) -> Result<Option<String>, String> {
    let (bytes, suggested) = save_bytes_and_name(&artifact);
    let start_dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());

    let Some(dest) = pick_save_path(&suggested, &start_dir) else {
        return Ok(None);
    };

    std::fs::write(&dest, &bytes).map_err(|e| format!("write {dest}: {e}"))?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_artifact::{Artifact, ArtifactOrigin};

    fn artifact(payload: ArtifactPayload, text: &str) -> Artifact {
        Artifact::new(payload, text.to_string(), ArtifactOrigin::AgentGenerated, None)
            .expect("valid fixture")
    }

    #[test]
    fn markdown_saves_its_source_as_md() {
        let a = artifact(
            ArtifactPayload::Markdown {
                source: "# Title\n".to_string(),
            },
            "Title",
        );
        let (bytes, name) = save_bytes_and_name(&a);
        assert_eq!(bytes, b"# Title\n");
        assert_eq!(name, "artifact.md");
    }

    #[test]
    fn code_uses_the_language_extension_and_falls_back_to_txt() {
        let rust = artifact(
            ArtifactPayload::Code {
                source: "fn main() {}".to_string(),
                language: Some("Rust".to_string()),
            },
            "fn main() {}",
        );
        let (bytes, name) = save_bytes_and_name(&rust);
        assert_eq!(bytes, b"fn main() {}");
        assert_eq!(name, "artifact.rs");

        let unknown = artifact(
            ArtifactPayload::Code {
                source: "x".to_string(),
                language: Some("brainfuck".to_string()),
            },
            "x",
        );
        assert_eq!(save_bytes_and_name(&unknown).1, "artifact.txt");

        let none = artifact(
            ArtifactPayload::Code {
                source: "y".to_string(),
                language: None,
            },
            "y",
        );
        assert_eq!(save_bytes_and_name(&none).1, "artifact.txt");
    }

    #[test]
    fn diagram_maps_to_its_language_extension() {
        let mermaid = artifact(
            ArtifactPayload::Diagram {
                language: DiagramLanguage::Mermaid,
                source: "graph TD; A-->B".to_string(),
            },
            "graph TD; A-->B",
        );
        assert_eq!(save_bytes_and_name(&mermaid).1, "artifact.mmd");
    }

    #[test]
    fn image_decodes_base64_to_real_bytes_with_the_media_extension() {
        // base64 "PNG?" -> the raw bytes back.
        let raw = b"PNG?";
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let a = artifact(
            ArtifactPayload::Image {
                media_type: ImageMediaType::Png,
                data_base64: b64,
            },
            "an image",
        );
        let (bytes, name) = save_bytes_and_name(&a);
        assert_eq!(bytes, raw);
        assert_eq!(name, "artifact.png");
    }

    #[test]
    fn an_undecodable_image_falls_back_to_the_text_floor() {
        let a = artifact(
            ArtifactPayload::Image {
                media_type: ImageMediaType::Png,
                data_base64: "not valid base64 !!!".to_string(),
            },
            "fallback text",
        );
        let (bytes, name) = save_bytes_and_name(&a);
        assert_eq!(bytes, b"fallback text");
        assert_eq!(name, "artifact.txt");
    }

    #[test]
    fn table_and_links_save_the_text_floor() {
        let table = artifact(
            ArtifactPayload::Table {
                columns: vec!["a".to_string()],
                rows: vec![vec!["1".to_string()]],
            },
            "a\n1\n",
        );
        let (bytes, name) = save_bytes_and_name(&table);
        assert_eq!(bytes, b"a\n1\n");
        assert_eq!(name, "artifact.txt");
    }
}
