//! The document reader's host: open a PDF and answer questions about it.
//!
//! `quickview-plan.md` gives this reader page navigation, an outline, in-document
//! search and text selection. Three of those are questions about the document and
//! are answered here, over `arlen-pdf-core`. Drawing the page is the piece that
//! is not here yet, and the reason is written down rather than left to be
//! guessed: it needs MuPDF, which is a large vendored C build, and a rasteriser
//! wants a surface to draw on, which is the slice after this one.
//!
//! **One document at a time, held open.** A reader window shows one file, and
//! re-parsing a hundred-megabyte PDF for every keystroke of a search would make
//! the search unusable on exactly the documents where it matters. So the open
//! document lives in app state and every command works against it.

use std::path::PathBuf;
use std::sync::Mutex;

use arlen_pdf_core::{Document, OutlineEntry, PdfError, SearchOutcome};
use serde::Serialize;

/// The file the reader was launched with, when it was launched with one.
///
/// `arlen-pdf <file>`, or the desktop entry's `%f` when somebody opens a PDF from
/// the file manager. `None` when launched bare, which is a reader with nothing
/// open rather than an error.
struct LaunchFile(Option<String>);

/// The document currently open, if any.
///
/// A `Mutex` rather than anything cleverer: the commands are short reads and a
/// reader has one document, so contention is a person pressing two keys at once.
#[derive(Default)]
struct Open(Mutex<Option<Held>>);

/// An open document and where it came from.
struct Held {
    path: PathBuf,
    doc: Document,
    /// The file as read.
    ///
    /// Kept because the page renderer is a separate process that takes the
    /// document on stdin: it holds no file capability of its own, by design, so
    /// the host is what hands it the bytes. Re-reading the file per page would
    /// also mean rendering a different document than the one whose outline is on
    /// screen, if it changed underneath.
    bytes: Vec<u8>,
}

/// What the surface needs to draw a reader around a document.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentInfo {
    /// The path as opened, so the window can name what it is showing.
    pub path: String,
    /// How many pages there are.
    pub pages: usize,
    /// The author's table of contents, empty when the document has none.
    ///
    /// Empty and absent are the same thing here and that is deliberate: a PDF
    /// either carries an outline or it does not, and there is no third state to
    /// distinguish. What the surface must NOT do is draw an empty contents pane
    /// as though the document were still loading.
    pub outline: Vec<OutlineEntry>,
}

/// The file the reader was launched with, for the page to ask about on mount.
#[tauri::command]
fn launch_file(state: tauri::State<'_, LaunchFile>) -> Option<String> {
    state.0.clone()
}

/// Open a document and describe it.
///
/// Replaces whatever was open before: this is a single-document reader, and
/// keeping the previous one alive would hold its memory for a window nobody is
/// looking at any more.
///
/// # Errors
/// The reason as a sentence, because it reaches a person: a file that is not
/// there, one that cannot be read, or bytes that are not a PDF are three
/// different things to be told.
#[tauri::command]
fn pdf_open(path: String, state: tauri::State<'_, Open>) -> Result<DocumentInfo, String> {
    let path = PathBuf::from(path);
    let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("there is no file at {}", path.display()),
        std::io::ErrorKind::PermissionDenied => {
            format!("this account may not read {}", path.display())
        }
        _ => format!("{} could not be read: {e}", path.display()),
    })?;
    let doc = Document::open(&bytes).map_err(|e| e.to_string())?;
    let info = DocumentInfo {
        path: path.display().to_string(),
        pages: doc.page_count(),
        outline: doc.outline(),
    };
    *state.0.lock().map_err(|_| lock_lost())? = Some(Held { path, doc, bytes });
    Ok(info)
}

/// The text on one page of the open document, one-based.
///
/// Here for selection and for a reader that wants to read a scanned-free
/// document as text. An empty string means the page carries no text, which is
/// what a scan looks like, and is not an error.
///
/// # Errors
/// When no document is open, or the page is outside it.
#[tauri::command]
fn pdf_page_text(page: usize, state: tauri::State<'_, Open>) -> Result<String, String> {
    let held = state.0.lock().map_err(|_| lock_lost())?;
    let held = held.as_ref().ok_or_else(no_document)?;
    held.doc.page_text(page).map_err(|e| match e {
        // The one case worth rewording: the core says "this PDF has no page 9",
        // and the surface can say which document that was.
        PdfError::NoSuchPage(n) => {
            format!("{} has {} pages, so there is no page {n}", held.path.display(), held.doc.page_count())
        }
        other => other.to_string(),
    })
}

/// Find a word in the open document.
///
/// Carries `unsearchable` through unchanged, because a page that could not be
/// read is not a page without matches and the surface has to be able to say so.
///
/// # Errors
/// When no document is open.
#[tauri::command]
fn pdf_search(query: String, state: tauri::State<'_, Open>) -> Result<SearchOutcome, String> {
    let held = state.0.lock().map_err(|_| lock_lost())?;
    let held = held.as_ref().ok_or_else(no_document)?;
    Ok(held.doc.search(&query))
}

/// One page, drawn.
#[derive(Debug, Clone, Serialize)]
pub struct PageImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA, four bytes a pixel.
    pub rgba: Vec<u8>,
}

/// Where the sandboxed page renderer lives: `ARLEN_PDF_WORKER_DIR` if set (the
/// dev and dist override), else the directory of the running reader, beside
/// which it ships.
fn worker_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARLEN_PDF_WORKER_DIR") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Draw one page and hand back its pixels.
///
/// The document goes to a separate process that can neither reach the network
/// nor read a file - it gets the bytes on stdin and writes a frame back - so a
/// bug in the renderer costs this page rather than the reader. MuPDF is
/// single-threaded here, which is the tight syscall profile.
///
/// # Errors
/// When no document is open, or the worker refused: a page that is not there,
/// a raster past the bound, or a document it could not read.
#[tauri::command]
fn pdf_page_image(page: usize, scale: f32, state: tauri::State<'_, Open>) -> Result<PageImage, String> {
    let bytes = {
        let held = state.0.lock().map_err(|_| lock_lost())?;
        held.as_ref().ok_or_else(no_document)?.bytes.clone()
    };
    let dir = worker_dir();
    let frame = arlen_worker_sandbox::run_confined_worker(
        &dir.to_string_lossy(),
        "arlen-pdf-decode-page",
        arlen_worker_sandbox::WorkerProfile::SINGLE_THREADED,
        &[page.to_string(), scale.to_string()],
        &bytes,
    )?;
    decode_frame(&frame)
}

/// Read the worker's frame: `RGBA`, width, height, then the body.
///
/// Checked rather than trusted. The worker is the component this design assumes
/// can be compromised, so a frame whose header is wrong, whose dimensions do not
/// match its body, or that is short is refused - drawing whatever arrived is how
/// a broken renderer becomes a rendering bug nobody can find.
fn decode_frame(frame: &[u8]) -> Result<PageImage, String> {
    if frame.len() < 12 || &frame[..4] != b"RGBA" {
        return Err("the page renderer sent something that is not a frame".to_string());
    }
    let width = u32::from_le_bytes(frame[4..8].try_into().map_err(|_| "short frame")?);
    let height = u32::from_le_bytes(frame[8..12].try_into().map_err(|_| "short frame")?);
    let body = &frame[12..];
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("the page renderer claimed a size that cannot exist")?;
    if body.len() != expected {
        return Err(format!(
            "the page renderer sent {} bytes for a {width} by {height} page, which needs {expected}",
            body.len()
        ));
    }
    Ok(PageImage { width, height, rgba: body.to_vec() })
}

/// What to say when a command arrives with nothing open.
fn no_document() -> String {
    "no document is open".to_string()
}

/// A poisoned lock means a command panicked while holding the document.
///
/// Reported rather than unwrapped: taking the window down because one search
/// went wrong loses whatever else the person had open.
fn lock_lost() -> String {
    "the open document is in an unknown state; reopen the file".to_string()
}

/// Run the reader.
///
/// # Panics
/// If Tauri cannot start, which means there is no window to report into.
pub fn run() {
    // `init()` alone defaults to `error`, which makes an app mute in the journal
    // for everything short of a failure. A reader that cannot open a file should
    // be able to say so where somebody debugging it will look.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_pdf_lib=info"),
    )
    .init();
    // `%f` from the desktop entry, or an argument. Skipped when it starts with a
    // dash so a flag is never mistaken for a path.
    let launched = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'));
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(LaunchFile(launched))
        .manage(Open::default())
        .invoke_handler(tauri::generate_handler![
            launch_file,
            pdf_open,
            pdf_page_text,
            pdf_search,
            pdf_page_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
