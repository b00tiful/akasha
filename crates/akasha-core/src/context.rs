use std::error::Error;
use std::fmt::{self, Write};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use serde::Serialize;

use crate::project_validation::{
    ProjectValidationError, ProjectValidationReport, canonical_note_paths, validate_project,
};
use crate::resolution::{ResolveError, ResolveRequest, RootConfig, load_root_config};
use crate::validation::{ValidationError, parse_leading_frontmatter_bytes};

pub const DEFAULT_CONTEXT_MAX_CHARS: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextSection {
    OpenTask,
    OpenProblem,
    Roadmap,
    EntityIndex,
    LatestHandoff,
    RecentEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextEntry {
    pub section: ContextSection,
    pub source: PathBuf,
    pub content: String,
}

/// A deterministic, bounded orientation bundle for one validated project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextBundle {
    pub project: String,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
    pub errors: Vec<String>,
    pub entries: Vec<ContextEntry>,
    pub truncated: bool,
    pub omitted_entries: usize,
    pub max_chars: usize,
    pub rendered_chars: usize,
}

/// A one-line session-start summary for one validated project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionBreadcrumb {
    pub project: String,
    pub open_tasks: usize,
    pub latest_handoff_date: Option<String>,
    pub handoff_age_days: Option<i64>,
}

#[derive(Debug)]
pub enum ContextError {
    ProjectValidation(Box<ProjectValidationError>),
    InvalidDocument {
        path: PathBuf,
        source: Box<ValidationError>,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Clock(SystemTimeError),
    Budget {
        required_chars: usize,
        max_chars: usize,
    },
}

impl ContextError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::ProjectValidation(source) => source.exit_code(),
            Self::InvalidDocument { source, .. } => source.exit_code(),
            Self::FileSystem { .. } => 6,
            Self::Clock(_) => 6,
            Self::Budget { .. } => 4,
        }
    }
}

impl fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectValidation(source) => source.fmt(formatter),
            Self::InvalidDocument { path, source } => {
                write!(
                    formatter,
                    "validation failed at {}: {source}",
                    path.display()
                )
            }
            Self::FileSystem {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} at {}: {source}",
                path.display()
            ),
            Self::Clock(source) => {
                write!(formatter, "failed to read the current UTC day: {source}")
            }
            Self::Budget {
                required_chars,
                max_chars,
            } => write!(
                formatter,
                "context identity and truncation marker require {required_chars} characters, exceeding the {max_chars}-character limit"
            ),
        }
    }
}

impl Error for ContextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ProjectValidation(source) => Some(source.as_ref()),
            Self::InvalidDocument { source, .. } => Some(source.as_ref()),
            Self::FileSystem { source, .. } => Some(source),
            Self::Clock(source) => Some(source),
            Self::Budget { .. } => None,
        }
    }
}

impl From<ProjectValidationError> for ContextError {
    fn from(source: ProjectValidationError) -> Self {
        Self::ProjectValidation(Box::new(source))
    }
}

/// Validate a project and assemble its default 16,000-character orientation bundle.
pub fn assemble_context(request: &ResolveRequest) -> Result<ContextBundle, ContextError> {
    let report = validate_project(request)?;
    let config = load_root_config(&report.root)
        .map_err(ProjectValidationError::from)
        .map_err(ContextError::from)?;
    let candidates = collect_candidates(&report, &config)?;
    fit_candidates(&report, candidates, DEFAULT_CONTEXT_MAX_CHARS)
}

/// Validate a project and assemble its compact session-start breadcrumb.
pub fn assemble_session_breadcrumb(
    request: &ResolveRequest,
) -> Result<SessionBreadcrumb, ContextError> {
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(ContextError::Clock)?
        .as_secs()
        / 86_400;
    assemble_session_breadcrumb_for_day(request, today as i64)
}

/// Assemble the breadcrumb when the current directory belongs to an Akasha project.
///
/// A missing repository pointer is the only not-applicable state. Invalid pointers, explicit
/// projects, root configuration, and project contents continue to fail normally.
pub fn assemble_session_breadcrumb_if_linked(
    request: &ResolveRequest,
) -> Result<Option<SessionBreadcrumb>, ContextError> {
    match assemble_session_breadcrumb(request) {
        Ok(breadcrumb) => Ok(Some(breadcrumb)),
        Err(ContextError::ProjectValidation(source))
            if matches!(
                source.as_ref(),
                ProjectValidationError::Resolve(resolve)
                    if matches!(resolve.as_ref(), ResolveError::ProjectPointerNotFound { .. })
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn assemble_session_breadcrumb_for_day(
    request: &ResolveRequest,
    today: i64,
) -> Result<SessionBreadcrumb, ContextError> {
    let report = validate_project(request)?;
    let config = load_root_config(&report.root)
        .map_err(ProjectValidationError::from)
        .map_err(ContextError::from)?;
    let open_tasks = open_record_entries(
        &report,
        &config,
        &config.context.tasks,
        ContextSection::OpenTask,
    )?
    .len();
    let mut handoffs = dated_entries(
        &report,
        &config,
        std::slice::from_ref(&config.context.handoffs),
        ContextSection::LatestHandoff,
    )?;
    for handoff in &handoffs {
        validate_calendar_date(&handoff.date).map_err(|message| ContextError::InvalidDocument {
            path: report.project_dir.join(&handoff.entry.source),
            source: Box::new(ValidationError::InvalidSchema {
                document: "canonical note",
                message,
            }),
        })?;
    }
    sort_newest_first(&mut handoffs);
    let latest_handoff_date = handoffs.first().map(|handoff| handoff.date.clone());
    let handoff_age_days = latest_handoff_date
        .as_deref()
        .map(|date| {
            validate_calendar_date(date)
                .expect("every handoff date was validated before selecting the latest")
        })
        .map(|handoff_day| today - handoff_day);

    Ok(SessionBreadcrumb {
        project: report.project,
        open_tasks,
        latest_handoff_date,
        handoff_age_days,
    })
}

fn collect_candidates(
    report: &ProjectValidationReport,
    config: &RootConfig,
) -> Result<Vec<ContextEntry>, ContextError> {
    let mut candidates = Vec::new();
    candidates.extend(open_record_entries(
        report,
        config,
        &config.context.tasks,
        ContextSection::OpenTask,
    )?);
    candidates.extend(open_record_entries(
        report,
        config,
        &config.context.problems,
        ContextSection::OpenProblem,
    )?);
    candidates.push(read_project_file(
        report,
        &config.project.roadmap,
        ContextSection::Roadmap,
    )?);
    candidates.push(read_project_file(
        report,
        &config.project.index,
        ContextSection::EntityIndex,
    )?);

    let mut handoffs = dated_entries(
        report,
        config,
        std::slice::from_ref(&config.context.handoffs),
        ContextSection::LatestHandoff,
    )?;
    sort_newest_first(&mut handoffs);
    if let Some(latest) = handoffs.into_iter().next() {
        candidates.push(latest.entry);
    }

    let mut recent_events = dated_entries(
        report,
        config,
        &config.context.recent_events,
        ContextSection::RecentEvent,
    )?;
    sort_newest_first(&mut recent_events);
    candidates.extend(recent_events.into_iter().map(|event| event.entry));
    Ok(candidates)
}

fn open_record_entries(
    report: &ProjectValidationReport,
    config: &RootConfig,
    note_type_name: &str,
    section: ContextSection,
) -> Result<Vec<ContextEntry>, ContextError> {
    let note_type = config
        .project
        .note_types
        .get(note_type_name)
        .expect("validated context roles always name configured note types");
    let folder = report.project_dir.join(&note_type.folder);
    let mut entries = Vec::new();
    for path in canonical_note_paths(&folder)? {
        let note = read_note(&path, &report.project_dir, section)?;
        let status = metadata_string(&note.metadata, "status", &path)?;
        if config
            .context
            .open_statuses
            .iter()
            .any(|open| open == status)
        {
            entries.push(note.entry);
        }
    }
    Ok(entries)
}

fn read_project_file(
    report: &ProjectValidationReport,
    relative_path: &Path,
    section: ContextSection,
) -> Result<ContextEntry, ContextError> {
    let path = report.project_dir.join(relative_path);
    let content = fs::read_to_string(&path).map_err(|source| ContextError::FileSystem {
        operation: "read context projection",
        path: path.clone(),
        source,
    })?;
    Ok(ContextEntry {
        section,
        source: relative_path.to_path_buf(),
        content: content.trim().to_owned(),
    })
}

struct DatedEntry {
    date: String,
    entry: ContextEntry,
}

fn dated_entries(
    report: &ProjectValidationReport,
    config: &RootConfig,
    note_type_names: &[String],
    section: ContextSection,
) -> Result<Vec<DatedEntry>, ContextError> {
    let mut entries = Vec::new();
    for note_type_name in note_type_names {
        let note_type = config
            .project
            .note_types
            .get(note_type_name)
            .expect("validated context roles always name configured note types");
        let folder = report.project_dir.join(&note_type.folder);
        for path in canonical_note_paths(&folder)? {
            let note = read_note(&path, &report.project_dir, section)?;
            let date = metadata_string(&note.metadata, "date", &path)?.to_owned();
            entries.push(DatedEntry {
                date,
                entry: note.entry,
            });
        }
    }
    Ok(entries)
}

fn sort_newest_first(entries: &mut [DatedEntry]) {
    entries.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then_with(|| right.entry.source.cmp(&left.entry.source))
    });
}

struct ReadNote {
    metadata: serde_json::Value,
    entry: ContextEntry,
}

fn read_note(
    path: &Path,
    project_dir: &Path,
    section: ContextSection,
) -> Result<ReadNote, ContextError> {
    let source = fs::read(path).map_err(|source| ContextError::FileSystem {
        operation: "read context note",
        path: path.to_path_buf(),
        source,
    })?;
    let parsed = parse_leading_frontmatter_bytes(&source).map_err(|source| {
        ContextError::InvalidDocument {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    let relative = path
        .strip_prefix(project_dir)
        .expect("validated canonical notes remain inside the project directory")
        .to_path_buf();
    let entry = ContextEntry {
        section,
        source: relative,
        content: parsed.body.trim().to_owned(),
    };
    Ok(ReadNote {
        metadata: parsed.metadata.clone(),
        entry,
    })
}

fn metadata_string<'a>(
    metadata: &'a serde_json::Value,
    field: &str,
    path: &Path,
) -> Result<&'a str, ContextError> {
    metadata
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ContextError::InvalidDocument {
            path: path.to_path_buf(),
            source: Box::new(ValidationError::InvalidSchema {
                document: "canonical note",
                message: format!("context field {field:?} must be a string"),
            }),
        })
}

fn fit_candidates(
    report: &ProjectValidationReport,
    candidates: Vec<ContextEntry>,
    max_chars: usize,
) -> Result<ContextBundle, ContextError> {
    let total_entries = candidates.len();
    let mut bundle = ContextBundle {
        project: report.project.clone(),
        repository_dir: report.repository_dir.clone(),
        project_dir: report.project_dir.clone(),
        errors: Vec::new(),
        entries: Vec::new(),
        truncated: total_entries > 0,
        omitted_entries: total_entries,
        max_chars,
        rendered_chars: 0,
    };

    for candidate in candidates {
        bundle.entries.push(candidate);
        bundle.omitted_entries = total_entries - bundle.entries.len();
        bundle.truncated = bundle.omitted_entries > 0;
        if char_count(&render_context_markdown(&bundle)) > max_chars {
            bundle.entries.pop();
            bundle.omitted_entries = total_entries - bundle.entries.len();
            bundle.truncated = bundle.omitted_entries > 0;
            break;
        }
    }

    bundle.rendered_chars = char_count(&render_context_markdown(&bundle));
    if bundle.rendered_chars > max_chars {
        return Err(ContextError::Budget {
            required_chars: bundle.rendered_chars,
            max_chars,
        });
    }
    Ok(bundle)
}

/// Render a context bundle as the canonical bounded Markdown CLI projection.
#[must_use]
pub fn render_context_markdown(bundle: &ContextBundle) -> String {
    let mut output = String::new();
    writeln!(output, "# Akasha context").expect("writing to a string cannot fail");
    writeln!(output).expect("writing to a string cannot fail");
    writeln!(output, "- Project: `{}`", bundle.project).expect("writing to a string cannot fail");
    writeln!(
        output,
        "- Repository: `{}`",
        bundle.repository_dir.display()
    )
    .expect("writing to a string cannot fail");
    writeln!(
        output,
        "- Project memory: `{}`",
        bundle.project_dir.display()
    )
    .expect("writing to a string cannot fail");
    if bundle.errors.is_empty() {
        writeln!(output, "- Errors: none").expect("writing to a string cannot fail");
    } else {
        writeln!(output, "- Errors: {}", bundle.errors.join("; "))
            .expect("writing to a string cannot fail");
    }

    for entry in &bundle.entries {
        writeln!(
            output,
            "\n## {} — `{}`\n",
            section_name(entry.section),
            entry.source.display()
        )
        .expect("writing to a string cannot fail");
        writeln!(output, "{}", entry.content).expect("writing to a string cannot fail");
    }

    if bundle.truncated {
        writeln!(output, "\n## Truncated\n").expect("writing to a string cannot fail");
        writeln!(
            output,
            "Omitted {} lower-priority context entries to stay within {} characters.",
            bundle.omitted_entries, bundle.max_chars
        )
        .expect("writing to a string cannot fail");
    }
    output
}

/// Render the stable one-line session-start breadcrumb.
#[must_use]
pub fn render_session_breadcrumb(breadcrumb: &SessionBreadcrumb) -> String {
    let task_label = if breadcrumb.open_tasks == 1 {
        "open task"
    } else {
        "open tasks"
    };
    let handoff = match (
        breadcrumb.latest_handoff_date.as_deref(),
        breadcrumb.handoff_age_days,
    ) {
        (Some(date), Some(age)) => format!("last handoff {date} ({})", format_age(age)),
        (Some(date), None) => format!("last handoff {date}"),
        (None, _) => "no handoff".to_owned(),
    };
    format!(
        "Akasha {} — {} {task_label} — {handoff}\n",
        breadcrumb.project, breadcrumb.open_tasks
    )
}

fn format_age(age_days: i64) -> String {
    match age_days {
        0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        days if days > 1 => format!("{days} days ago"),
        -1 => "in 1 day".to_owned(),
        days => format!("in {} days", days.unsigned_abs()),
    }
}

fn validate_calendar_date(value: &str) -> Result<i64, String> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err(format!(
            "context field \"date\" must be a valid YYYY-MM-DD Gregorian date; received {value:?}"
        ));
    }

    let year = value[0..4]
        .parse::<i64>()
        .expect("validated ASCII digits always parse as a four-digit year");
    let month = value[5..7]
        .parse::<u8>()
        .expect("validated ASCII digits always parse as a two-digit month");
    let day = value[8..10]
        .parse::<u8>()
        .expect("validated ASCII digits always parse as a two-digit day");
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > days_in_month {
        return Err(format!(
            "context field \"date\" must be a valid YYYY-MM-DD Gregorian date; received {value:?}"
        ));
    }

    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Ok(era * 146_097 + day_of_era - 719_468)
}

const fn section_name(section: ContextSection) -> &'static str {
    match section {
        ContextSection::OpenTask => "Open task",
        ContextSection::OpenProblem => "Open problem",
        ContextSection::Roadmap => "Roadmap",
        ContextSection::EntityIndex => "Entity index",
        ContextSection::LatestHandoff => "Latest handoff",
        ContextSection::RecentEvent => "Recent event",
    }
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}

#[cfg(test)]
mod tests {
    use super::{format_age, validate_calendar_date};

    #[test]
    fn calendar_dates_map_to_unix_days_and_reject_invalid_values() {
        assert_eq!(validate_calendar_date("1970-01-01"), Ok(0));
        assert_eq!(validate_calendar_date("2000-02-29"), Ok(11_016));
        assert_eq!(validate_calendar_date("2026-07-16"), Ok(20_650));
        assert!(validate_calendar_date("2026-02-29").is_err());
        assert!(validate_calendar_date("2026-13-01").is_err());
        assert!(validate_calendar_date("16-07-2026").is_err());
    }

    #[test]
    fn handoff_age_covers_past_present_and_future_days() {
        assert_eq!(format_age(2), "2 days ago");
        assert_eq!(format_age(1), "1 day ago");
        assert_eq!(format_age(0), "today");
        assert_eq!(format_age(-1), "in 1 day");
        assert_eq!(format_age(-2), "in 2 days");
    }
}
