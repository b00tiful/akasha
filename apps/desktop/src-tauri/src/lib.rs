use std::path::PathBuf;

use akasha_core::{
    LibraryDocument, LibraryProjection, NoteEditRecovery, NoteEditResult, ResolveRequest,
    build_library_projection, load_library_document, recover_pending_note_edit,
    render_library_markdown, replace_library_document,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DesktopLibrary {
    pub projection: LibraryProjection,
    pub fallback_markdown: String,
    pub recovery: NoteEditRecovery,
}

#[derive(Debug, Serialize)]
pub struct DesktopError {
    pub code: u8,
    pub message: String,
}

pub fn library_projection(
    root: Option<PathBuf>,
    project: Option<String>,
) -> Result<DesktopLibrary, DesktopError> {
    let request = request(root, project)?;
    let recovery = recover_pending_note_edit(&request).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })?;
    let projection = build_library_projection(&request).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })?;
    let fallback_markdown = render_library_markdown(&projection);
    Ok(DesktopLibrary {
        projection,
        fallback_markdown,
        recovery,
    })
}

pub fn library_document(
    root: Option<PathBuf>,
    project: Option<String>,
    id: &str,
) -> Result<LibraryDocument, DesktopError> {
    let request = request(root, project)?;
    recover_pending_note_edit(&request).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })?;
    load_library_document(&request, id).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })
}

pub fn save_library_document(
    root: Option<PathBuf>,
    project: Option<String>,
    id: &str,
    expected_source: &str,
    replacement_source: &str,
) -> Result<NoteEditResult, DesktopError> {
    let request = request(root, project)?;
    replace_library_document(&request, id, expected_source, replacement_source).map_err(|error| {
        DesktopError {
            code: error.exit_code(),
            message: error.to_string(),
        }
    })
}

fn request(root: Option<PathBuf>, project: Option<String>) -> Result<ResolveRequest, DesktopError> {
    ResolveRequest::from_process(root, project).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn load_library(
    root: Option<PathBuf>,
    project: Option<String>,
) -> Result<DesktopLibrary, DesktopError> {
    library_projection(root, project)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn load_document(
    root: Option<PathBuf>,
    project: Option<String>,
    id: String,
) -> Result<LibraryDocument, DesktopError> {
    library_document(root, project, &id)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn save_document(
    root: Option<PathBuf>,
    project: Option<String>,
    id: String,
    expected_source: String,
    replacement_source: String,
) -> Result<NoteEditResult, DesktopError> {
    save_library_document(root, project, &id, &expected_source, &replacement_source)
}

#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_library,
            load_document,
            save_document
        ])
        .run(tauri::generate_context!())
        .expect("run Akasha desktop application");
}
