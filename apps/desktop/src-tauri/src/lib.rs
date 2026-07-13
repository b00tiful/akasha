use std::path::PathBuf;

use akasha_core::{
    LibraryDocument, LibraryProjection, ResolveRequest, build_library_projection,
    load_library_document, render_library_markdown,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DesktopLibrary {
    pub projection: LibraryProjection,
    pub fallback_markdown: String,
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
    let projection = build_library_projection(&request).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
    })?;
    let fallback_markdown = render_library_markdown(&projection);
    Ok(DesktopLibrary {
        projection,
        fallback_markdown,
    })
}

pub fn library_document(
    root: Option<PathBuf>,
    project: Option<String>,
    id: &str,
) -> Result<LibraryDocument, DesktopError> {
    let request = request(root, project)?;
    load_library_document(&request, id).map_err(|error| DesktopError {
        code: error.exit_code(),
        message: error.to_string(),
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
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![load_library, load_document])
        .run(tauri::generate_context!())
        .expect("run Akasha desktop application");
}
