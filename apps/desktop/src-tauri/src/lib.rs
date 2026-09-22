use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use akasha_core::{
    LibraryDocument, LibraryProjection, LibraryScope, LibrarySearchResult, NoteEditRecovery,
    NoteEditResult, ResolveRequest, build_library_projection, load_library_document,
    recover_pending_note_edit, render_library_markdown, replace_library_document, search_library,
};
use serde::{Deserialize, Serialize};

const LOCAL_NAVIGATION_VERSION: u8 = 1;
const LOCAL_NAVIGATION_MAX_BYTES: u64 = 64 * 1024;
const LOCAL_NAVIGATION_MAX_PAGE_ANCHORS: usize = 128;
#[cfg(feature = "runtime-probe")]
const RUNTIME_PROBE_REPORT_MAX_BYTES: usize = 64 * 1024;
static NEXT_STATE_STAGE: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalNavigationState {
    pub version: u8,
    pub root: String,
    pub project: String,
    pub section: Option<String>,
    pub sky_anchor: Option<String>,
    pub page_anchors: BTreeMap<String, String>,
    pub note: Option<String>,
}

pub fn load_local_navigation_file(
    path: &Path,
    root: &str,
    project: &str,
) -> Result<Option<LocalNavigationState>, DesktopError> {
    validate_binding(root, project)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(state_io_error("inspect", path, error)),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(state_validation_error(format!(
            "local navigation state is not a regular file: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(state_validation_error(format!(
            "local navigation state is not private (expected mode 0600): {}",
            path.display()
        )));
    }
    if metadata.len() > LOCAL_NAVIGATION_MAX_BYTES {
        return Err(state_validation_error(format!(
            "local navigation state exceeds {LOCAL_NAVIGATION_MAX_BYTES} bytes"
        )));
    }

    let file = File::open(path).map_err(|error| state_io_error("open", path, error))?;
    let mut bytes = Vec::new();
    file.take(LOCAL_NAVIGATION_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| state_io_error("read", path, error))?;
    if bytes.len() as u64 > LOCAL_NAVIGATION_MAX_BYTES {
        return Err(state_validation_error(format!(
            "local navigation state exceeds {LOCAL_NAVIGATION_MAX_BYTES} bytes"
        )));
    }
    let state: LocalNavigationState = serde_json::from_slice(&bytes).map_err(|error| {
        state_validation_error(format!("invalid local navigation state JSON: {error}"))
    })?;
    validate_local_navigation_state(&state)?;
    if state.root != root || state.project != project {
        return Ok(None);
    }
    Ok(Some(state))
}

pub fn save_local_navigation_file(
    path: &Path,
    state: &LocalNavigationState,
) -> Result<(), DesktopError> {
    validate_local_navigation_state(state)?;
    let mut bytes = serde_json::to_vec(state).map_err(|error| {
        state_validation_error(format!("could not encode local navigation state: {error}"))
    })?;
    bytes.push(b'\n');
    if bytes.len() as u64 > LOCAL_NAVIGATION_MAX_BYTES {
        return Err(state_validation_error(format!(
            "local navigation state exceeds {LOCAL_NAVIGATION_MAX_BYTES} bytes"
        )));
    }

    let parent = path.parent().ok_or_else(|| {
        state_validation_error("local navigation state path has no parent directory".to_owned())
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| state_io_error("create state directory", parent, error))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| state_io_error("inspect state directory", parent, error))?;
    if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(state_validation_error(format!(
            "local navigation state parent is not a regular directory: {}",
            parent.display()
        )));
    }
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|error| state_io_error("secure state directory", parent, error))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            state_validation_error("local navigation state filename is invalid".to_owned())
        })?;
    let stage = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_STATE_STAGE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&stage)
            .map_err(|error| state_io_error("create private staging file", &stage, error))?;
        file.write_all(&bytes)
            .map_err(|error| state_io_error("write staging file", &stage, error))?;
        file.sync_all()
            .map_err(|error| state_io_error("sync staging file", &stage, error))?;
        fs::rename(&stage, path)
            .map_err(|error| state_io_error("publish local navigation state", path, error))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| state_io_error("sync state directory", parent, error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result
}

fn validate_local_navigation_state(state: &LocalNavigationState) -> Result<(), DesktopError> {
    if state.version != LOCAL_NAVIGATION_VERSION {
        return Err(state_validation_error(format!(
            "unsupported local navigation state version {}",
            state.version
        )));
    }
    validate_binding(&state.root, &state.project)?;
    validate_optional_identity("section", state.section.as_deref(), 512)?;
    validate_optional_identity("sky anchor", state.sky_anchor.as_deref(), 512)?;
    validate_optional_identity("note", state.note.as_deref(), 4096)?;
    if state.page_anchors.len() > LOCAL_NAVIGATION_MAX_PAGE_ANCHORS {
        return Err(state_validation_error(format!(
            "local navigation state has more than {LOCAL_NAVIGATION_MAX_PAGE_ANCHORS} page anchors"
        )));
    }
    for (section, note) in &state.page_anchors {
        validate_identity("page-anchor section", section, 512)?;
        validate_identity("page-anchor note", note, 4096)?;
    }
    Ok(())
}

fn validate_binding(root: &str, project: &str) -> Result<(), DesktopError> {
    validate_identity("root binding", root, 8192)?;
    validate_identity("project binding", project, 512)
}

fn validate_optional_identity(
    label: &str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), DesktopError> {
    if let Some(value) = value {
        validate_identity(label, value, max_bytes)?;
    }
    Ok(())
}

fn validate_identity(label: &str, value: &str, max_bytes: usize) -> Result<(), DesktopError> {
    if value.is_empty() || value.len() > max_bytes || value.contains('\0') {
        return Err(state_validation_error(format!(
            "local navigation {label} is empty, too long, or contains NUL"
        )));
    }
    Ok(())
}

fn state_validation_error(message: String) -> DesktopError {
    DesktopError { code: 4, message }
}

fn state_io_error(action: &str, path: &Path, error: std::io::Error) -> DesktopError {
    DesktopError {
        code: 3,
        message: format!("could not {action} {}: {error}", path.display()),
    }
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

pub fn search_project_library(
    root: Option<PathBuf>,
    project: String,
    query: &str,
) -> Result<LibrarySearchResult, DesktopError> {
    let request = request(root, Some(project.clone()))?;
    search_library(
        &request,
        query,
        Some(&LibraryScope::Project { project }),
        30,
    )
    .map_err(|error| DesktopError {
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
fn search_project(
    root: Option<PathBuf>,
    project: String,
    query: String,
) -> Result<LibrarySearchResult, DesktopError> {
    search_project_library(root, project, &query)
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
fn local_navigation_path(app: &tauri::AppHandle) -> Result<PathBuf, DesktopError> {
    use tauri::Manager;

    app.path()
        .app_local_data_dir()
        .map(|directory| directory.join("state/local-navigation.json"))
        .map_err(|error| DesktopError {
            code: 3,
            message: format!("could not resolve the desktop state directory: {error}"),
        })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn load_local_navigation(
    app: tauri::AppHandle,
    root: String,
    project: String,
) -> Result<Option<LocalNavigationState>, DesktopError> {
    load_local_navigation_file(&local_navigation_path(&app)?, &root, &project)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn save_local_navigation(
    app: tauri::AppHandle,
    state: LocalNavigationState,
) -> Result<(), DesktopError> {
    save_local_navigation_file(&local_navigation_path(&app)?, &state)
}

#[cfg(all(feature = "desktop", feature = "runtime-probe"))]
#[tauri::command]
fn write_runtime_probe_report(report: String) -> Result<(), DesktopError> {
    let path = std::env::var_os("AKASHA_RUNTIME_REPORT")
        .map(PathBuf::from)
        .ok_or_else(|| {
            state_validation_error("runtime probe report path is unavailable".to_owned())
        })?;
    write_runtime_probe_report_file(&path, &report)
}

#[cfg(feature = "runtime-probe")]
pub fn write_runtime_probe_report_file(path: &Path, report: &str) -> Result<(), DesktopError> {
    if report.len() > RUNTIME_PROBE_REPORT_MAX_BYTES {
        return Err(state_validation_error(format!(
            "runtime probe report exceeds {RUNTIME_PROBE_REPORT_MAX_BYTES} bytes"
        )));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| state_io_error("create runtime probe report", path, error))?;
    file.write_all(report.as_bytes())
        .map_err(|error| state_io_error("write runtime probe report", path, error))?;
    file.sync_all()
        .map_err(|error| state_io_error("sync runtime probe report", path, error))?;
    Ok(())
}

#[cfg(feature = "desktop")]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(feature = "runtime-probe")]
    let builder = builder.setup(|app| {
        use tauri::Manager;

        if std::env::var_os("AKASHA_RUNTIME_FULLSCREEN").is_some_and(|value| value == "1") {
            app.get_webview_window("main")
                .expect("runtime probe main window")
                .set_fullscreen(true)?;
        }
        Ok(())
    });
    #[cfg(feature = "runtime-probe")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        load_library,
        load_document,
        search_project,
        save_document,
        load_local_navigation,
        save_local_navigation,
        write_runtime_probe_report
    ]);
    #[cfg(not(feature = "runtime-probe"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        load_library,
        load_document,
        search_project,
        save_document,
        load_local_navigation,
        save_local_navigation
    ]);
    builder
        .run(tauri::generate_context!())
        .expect("run Akasha desktop application");
}
