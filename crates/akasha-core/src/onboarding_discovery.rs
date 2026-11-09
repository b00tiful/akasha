//! Opt-in, bounded source selection within explicitly authorized repository directories.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::onboarding::{OnboardingBatchError, validate_normalized_relative_path};
use crate::onboarding_local::{
    LocalOnboardingFocus, LocalOnboardingPacket, MAX_LOCAL_ONBOARDING_FILE_BYTES,
    MAX_LOCAL_ONBOARDING_FILES, MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES, bounded_json,
    checked_focus, focus_matches, invalid, local_onboarding_base, read_local_onboarding_source,
    rebuild_local_onboarding,
};
use crate::resolution::ResolveRequest;

const MAX_ENTRIES: usize = 2048;
const MAX_CANDIDATES: usize = 128;
const MAX_DEPTH: usize = 8;

#[derive(Debug, Serialize)]
pub struct LocalOnboardingDiscovery {
    pub schema_version: u32,
    pub packet: LocalOnboardingPacket,
    pub coverage: LocalSourceCoverage,
}

#[derive(Debug, Serialize)]
pub struct LocalSourceCoverage {
    pub directories: Vec<String>,
    pub entries_inspected: usize,
    pub candidates: usize,
    pub selected: Vec<LocalSourceSelection>,
    pub omitted: Vec<LocalSourceOmission>,
    pub excluded_entries: usize,
    pub excluded_examples: Vec<LocalSourceOmission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_scan_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct LocalSourceSelection {
    pub path: String,
    pub role: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LocalSourceOmission {
    pub path: String,
    pub reason: String,
}

fn io_error(path: &Path, source: std::io::Error) -> OnboardingBatchError {
    OnboardingBatchError::FileSystem {
        operation: "inspect local onboarding directory",
        path: path.into(),
        source,
    }
}

fn checked_directory(repository: &Path, relative: &str) -> Result<PathBuf, OnboardingBatchError> {
    let mut path = repository.to_path_buf();
    if relative != "." {
        validate_normalized_relative_path(relative, "discovery directory", repository)?;
        for part in relative.split('/') {
            path.push(part);
            let metadata = fs::symlink_metadata(&path).map_err(|e| io_error(&path, e))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid(
                    "discovery directories must be regular directories without symlinks",
                ));
            }
        }
    }
    if fs::canonicalize(&path).map_err(|e| io_error(&path, e))? != path {
        return Err(invalid(
            "discovery directory identity changed or escaped the repository",
        ));
    }
    Ok(path)
}

// Scope is operator-owned. This deliberately does not interpret .gitignore rules.
fn excluded(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target"
                | "node_modules"
                | "vendor"
                | "dist"
                | "build"
                | "coverage"
                | "__pycache__"
                | "venv"
                | "env"
        )
        || ["secret", "credential", "private-key"]
            .iter()
            .any(|word| name.contains(word))
}

pub(crate) fn role(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next()?.to_ascii_lowercase();
    if name.starts_with("readme.") {
        return Some("overview");
    }
    if matches!(
        name.as_str(),
        "cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "go.mod"
            | "cmakelists.txt"
            | "makefile"
            | "rust-toolchain.toml"
    ) {
        return Some("manifest");
    }
    let extension = name.rsplit_once('.')?.1;
    if !matches!(
        extension,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "py"
            | "go"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "java"
            | "kt"
            | "swift"
            | "cs"
            | "rb"
            | "sh"
            | "md"
    ) {
        return None;
    }
    if path
        .split('/')
        .any(|part| matches!(part, "tests" | "test" | "__tests__"))
        || name.starts_with("test_")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
    {
        return Some("tests");
    }
    Some(if extension == "md" {
        "documentation"
    } else if extension == "sh" {
        "workflow"
    } else {
        "implementation"
    })
}

/// Select complete sources, balancing roles and implementation directories within the old packet
/// bounds. Limits refuse the entire scan, never return a misleading partially scanned inventory.
pub fn discover_local_onboarding(
    request: &ResolveRequest,
    directories: &[String],
) -> Result<LocalOnboardingDiscovery, OnboardingBatchError> {
    discover_local_onboarding_mode(request, directories, false, &[])
}

/// Same scoped discovery, with explicit partial coverage for larger eligible sources.
pub fn discover_local_onboarding_sampled(
    request: &ResolveRequest,
    directories: &[String],
) -> Result<LocalOnboardingDiscovery, OnboardingBatchError> {
    discover_local_onboarding_mode(request, directories, true, &[])
}

/// Prioritize literal focus matches within each role, keeping overview/manifest context.
pub fn discover_local_onboarding_focused(
    request: &ResolveRequest,
    directories: &[String],
    terms: &[String],
    sample_large_files: bool,
) -> Result<LocalOnboardingDiscovery, OnboardingBatchError> {
    discover_local_onboarding_mode(
        request,
        directories,
        sample_large_files,
        &checked_focus(terms)?,
    )
}

fn discover_local_onboarding_mode(
    request: &ResolveRequest,
    directories: &[String],
    sample_large_files: bool,
    terms: &[String],
) -> Result<LocalOnboardingDiscovery, OnboardingBatchError> {
    if directories.is_empty() || directories.len() > 8 {
        return Err(invalid(
            "discovery requires 1..=8 explicit directories (use . for the repository)",
        ));
    }
    let (mut packet, repository) = local_onboarding_base(request)?;
    if sample_large_files {
        packet.schema_version = 3;
    }
    if !terms.is_empty() {
        packet.schema_version = 4;
        packet.focus = Some(LocalOnboardingFocus {
            terms: terms.to_vec(),
            sample_large_files,
        });
    }
    let file_limit = if sample_large_files {
        MAX_LOCAL_ONBOARDING_SAMPLED_FILE_BYTES
    } else {
        MAX_LOCAL_ONBOARDING_FILE_BYTES
    };
    let mut scopes = directories.to_vec();
    scopes.sort();
    for (i, scope) in scopes.iter().enumerate() {
        checked_directory(&repository, scope)?;
        if scopes[..i]
            .iter()
            .any(|prior| prior == scope || prior == "." || scope.starts_with(&format!("{prior}/")))
        {
            return Err(invalid("discovery directories must not repeat or overlap"));
        }
    }
    let mut coverage = LocalSourceCoverage {
        directories: scopes.clone(),
        entries_inspected: 0,
        candidates: 0,
        selected: Vec::new(),
        omitted: Vec::new(),
        excluded_entries: 0,
        excluded_examples: Vec::new(),
        focus_scan_bytes: (!terms.is_empty()).then_some(0),
    };
    let mut pending: VecDeque<_> = scopes.into_iter().map(|scope| (scope, 0)).collect();
    let mut candidates: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    while let Some((relative, depth)) = pending.pop_front() {
        let directory = checked_directory(&repository, &relative)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&directory).map_err(|e| io_error(&directory, e))? {
            coverage.entries_inspected += 1;
            if coverage.entries_inspected > MAX_ENTRIES {
                return Err(invalid(
                    "discovery exceeds 2048 entries; choose narrower directories",
                ));
            }
            entries.push(entry.map_err(|e| io_error(&directory, e))?);
        }
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("discovery paths must be UTF-8"))?;
            let path = if relative == "." {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if path.len() > 1024 {
                return Err(invalid("discovery path exceeds 1024 bytes"));
            }
            let kind = entry.file_type().map_err(|e| io_error(&entry.path(), e))?;
            let reason = if excluded(&name.to_ascii_lowercase()) {
                Some("excluded_name")
            } else if kind.is_symlink() {
                Some("symlink")
            } else if !kind.is_file() && !kind.is_dir() {
                Some("nonregular")
            } else {
                None
            };
            if let Some(reason) = reason {
                coverage.excluded_entries += 1;
                if coverage.excluded_examples.len() < 32 {
                    coverage.excluded_examples.push(LocalSourceOmission {
                        path,
                        reason: reason.into(),
                    });
                }
            } else if kind.is_dir() {
                if depth == MAX_DEPTH {
                    return Err(invalid(
                        "discovery exceeds 8 directory levels; choose narrower directories",
                    ));
                }
                pending.push_back((path, depth + 1));
            } else if let Some(role) = role(&path) {
                coverage.candidates += 1;
                if coverage.candidates > MAX_CANDIDATES {
                    return Err(invalid(
                        "discovery exceeds 128 candidates; choose narrower directories",
                    ));
                }
                candidates.entry(role).or_default().push(path);
            } else {
                coverage.excluded_entries += 1;
                if coverage.excluded_examples.len() < 32 {
                    coverage.excluded_examples.push(LocalSourceOmission {
                        path,
                        reason: "unsupported_name".into(),
                    });
                }
            }
        }
    }
    // Only explicit focus inspects unselected eligible content. The full bounded inventory is
    // checked first; excluded paths are never opened. Cache packed sources, at most 128 x 8 KiB.
    let mut focused_sources = BTreeMap::new();
    if !terms.is_empty() {
        for (role, paths) in &mut candidates {
            let mut eligible = Vec::new();
            for path in paths.drain(..) {
                let target = repository.join(&path);
                let metadata = fs::symlink_metadata(&target).map_err(|e| io_error(&target, e))?;
                if metadata.len() > file_limit as u64 {
                    coverage.omitted.push(LocalSourceOmission {
                        path,
                        reason: "file_too_large".into(),
                    });
                    continue;
                }
                let source = match read_local_onboarding_source(
                    &repository,
                    &path,
                    sample_large_files,
                    terms,
                ) {
                    Ok(source) => source,
                    Err(OnboardingBatchError::Validation { .. }) => {
                        coverage.omitted.push(LocalSourceOmission {
                            path,
                            reason: "invalid_source".into(),
                        });
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                *coverage.focus_scan_bytes.as_mut().expect("focused scan") += source
                    .sampling
                    .as_ref()
                    .map_or(source.text.len(), |sampling| sampling.total_bytes);
                let matches = focus_matches(terms, &path, &source.text);
                let reason = if source.excerpts.is_empty() || source.text.trim().is_empty() {
                    Some("empty_or_overlong_line")
                } else if matches.is_empty() && !matches!(*role, "overview" | "manifest") {
                    Some("focus_no_match")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    coverage.omitted.push(LocalSourceOmission {
                        path,
                        reason: reason.into(),
                    });
                } else {
                    focused_sources.insert(path.clone(), (source, matches));
                    eligible.push(path);
                }
            }
            *paths = eligible;
        }
    }
    let score = |path: &String| {
        focused_sources
            .get(path)
            .map_or(0, |(_, matches)| matches.len())
    };
    for paths in candidates.values_mut() {
        paths.sort_by_key(|path| {
            (
                std::cmp::Reverse(score(path)),
                path.matches('/').count(),
                path.clone(),
            )
        });
    }
    let mut selected_parents = BTreeSet::new();
    // Round-robin keeps a large source subtree from consuming every available slot.
    while candidates.values().any(|paths| !paths.is_empty()) {
        for role in [
            "overview",
            "manifest",
            "implementation",
            "tests",
            "documentation",
            "workflow",
        ] {
            let Some(paths) = candidates.get_mut(role).filter(|paths| !paths.is_empty()) else {
                continue;
            };
            let index = if role == "implementation" {
                paths
                    .iter()
                    .position(|p| {
                        score(p) == score(&paths[0])
                            && !selected_parents
                                .contains(p.rsplit_once('/').map_or("", |(parent, _)| parent))
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            let path = paths.remove(index);
            let metadata = fs::symlink_metadata(repository.join(&path))
                .map_err(|e| io_error(&repository.join(&path), e))?;
            let reason = if packet.sources.len() == MAX_LOCAL_ONBOARDING_FILES {
                Some("file_limit")
            } else if metadata.len() > file_limit as u64 {
                Some("file_too_large")
            } else {
                None
            };
            if let Some(reason) = reason {
                coverage.omitted.push(LocalSourceOmission {
                    path,
                    reason: reason.into(),
                });
                continue;
            }
            let source = match focused_sources
                .get(&path)
                .map(|(source, _)| Ok(source.clone()))
                .unwrap_or_else(|| {
                    read_local_onboarding_source(&repository, &path, sample_large_files, terms)
                }) {
                Ok(source) => source,
                Err(OnboardingBatchError::Validation { .. }) => {
                    coverage.omitted.push(LocalSourceOmission {
                        path,
                        reason: "invalid_source".into(),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            if source.excerpts.is_empty() || source.text.trim().is_empty() {
                coverage.omitted.push(LocalSourceOmission {
                    path,
                    reason: "empty_or_overlong_line".into(),
                });
                continue;
            }
            packet.sources.push(source);
            if bounded_json(&packet).is_err() {
                packet.sources.pop();
                coverage.omitted.push(LocalSourceOmission {
                    path,
                    reason: "packet_budget".into(),
                });
            } else {
                selected_parents.insert(
                    path.rsplit_once('/')
                        .map_or("", |(parent, _)| parent)
                        .to_owned(),
                );
                coverage.selected.push(LocalSourceSelection {
                    matched_terms: focused_sources
                        .get(&path)
                        .map_or_else(Vec::new, |(_, matches)| matches.clone()),
                    path,
                    role: role.into(),
                });
            }
        }
    }
    if packet.sources.is_empty() {
        return Err(invalid(
            "no eligible sources fit; choose different directories or explicit files",
        ));
    }
    if !terms.is_empty()
        && coverage
            .selected
            .iter()
            .all(|source| source.matched_terms.is_empty())
    {
        return Err(invalid(
            "no focus matches fit in the selected sources; choose different terms or narrower directories",
        ));
    }
    packet.sources.sort_by(|a, b| a.path.cmp(&b.path));
    coverage.selected.sort_by(|a, b| a.path.cmp(&b.path));
    coverage.omitted.sort_by(|a, b| a.path.cmp(&b.path));
    if packet != rebuild_local_onboarding(request, &packet)? {
        return Err(invalid(
            "discovered inputs changed during selection; discover again",
        ));
    }
    Ok(LocalOnboardingDiscovery {
        schema_version: 1,
        packet,
        coverage,
    })
}
