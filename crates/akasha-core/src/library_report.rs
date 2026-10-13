//! Bounded cross-project metadata reports over the validated library dashboard.

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

use crate::library::{LibraryDashboard, build_library_projection};
use crate::note_edit::{NoteEditError, inspect_pending_note_edit};
use crate::project_validation::ProjectValidationError;
use crate::resolution::{ResolveRequest, load_project_registry, load_root_config, resolve_project};
use crate::validation::ValidationError;

pub const DEFAULT_LIBRARY_REPORT_LIMIT: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryReportKind {
    Status,
    Digest,
}

/// Aggregates cover the entire library; only dashboard.project_metrics is bounded.
/// The selected project is a resolution anchor, not a filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryReport {
    pub kind: LibraryReportKind,
    pub root: PathBuf,
    pub selected_project: String,
    pub limit: usize,
    pub truncated: bool,
    pub dashboard: LibraryDashboard,
}

#[derive(Debug)]
pub enum LibraryReportError {
    Validation(Box<ProjectValidationError>),
    Inspection(Box<NoteEditError>),
    PendingJournal(PathBuf),
}

impl LibraryReportError {
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Validation(error) => error.exit_code(),
            Self::Inspection(error) => error.exit_code(),
            Self::PendingJournal(_) => 5,
        }
    }
}

impl fmt::Display for LibraryReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(error) => error.fmt(formatter),
            Self::Inspection(error) => error.fmt(formatter),
            Self::PendingJournal(path) => write!(
                formatter,
                "pending note recovery journal at {}; reconcile and recover before reporting",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LibraryReportError {}

impl From<ProjectValidationError> for LibraryReportError {
    fn from(error: ProjectValidationError) -> Self {
        Self::Validation(Box::new(error))
    }
}

/// Fresh read-only report. Never recovers journals or writes; concurrent reads are not atomic.
pub fn inspect_library_report(
    request: &ResolveRequest,
    kind: LibraryReportKind,
    limit: usize,
) -> Result<LibraryReport, LibraryReportError> {
    if !(1..=100).contains(&limit) {
        return Err(ProjectValidationError::InvalidDocument {
            path: request.cwd.clone(),
            source: Box::new(ValidationError::InvalidSchema {
                document: "library report",
                message: "project result limit must be between 1 and 100".into(),
            }),
        }
        .into());
    }
    let selected = resolve_project(request).map_err(ProjectValidationError::from)?;
    let config = load_root_config(&selected.root).map_err(ProjectValidationError::from)?;
    let (_, registry) =
        load_project_registry(&selected.root, &config).map_err(ProjectValidationError::from)?;
    let requests: Vec<_> = registry
        .projects
        .keys()
        .map(|project| ResolveRequest {
            root_override: Some(selected.root.clone()),
            project_override: Some(project.clone()),
            cwd: request.cwd.clone(),
            environment: request.environment.clone(),
        })
        .collect();
    refuse_pending(&requests)?;
    let projection = build_library_projection(request)?;
    refuse_pending(&requests)?;
    let mut dashboard = projection.dashboard;
    dashboard.project_metrics.sort_by(|left, right| {
        let date_order = match kind {
            LibraryReportKind::Status => std::cmp::Ordering::Equal,
            LibraryReportKind::Digest => right.latest_activity_date.cmp(&left.latest_activity_date),
        };
        date_order.then_with(|| left.project.cmp(&right.project))
    });
    let truncated = dashboard.project_metrics.len() > limit;
    dashboard.project_metrics.truncate(limit);
    Ok(LibraryReport {
        kind,
        root: projection.root,
        selected_project: projection.selected_project,
        limit,
        truncated,
        dashboard,
    })
}

fn refuse_pending(requests: &[ResolveRequest]) -> Result<(), LibraryReportError> {
    for request in requests {
        let inspection = inspect_pending_note_edit(request)
            .map_err(|error| LibraryReportError::Inspection(Box::new(error)))?;
        if inspection.pending {
            return Err(LibraryReportError::PendingJournal(inspection.journal_path));
        }
    }
    Ok(())
}
