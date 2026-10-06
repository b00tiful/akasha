//! Explicit, reviewed creation of a private data root. Existing roots are never adopted.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::resolution::{ROOT_CONFIG_FILE, RootConfig, validate_root_config};
use crate::writes::{AtomicCreateError, create_file_atomically, io_call, sync_directory};

pub(crate) const JOURNAL: &str = ".akasha-setup.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootSetupPlan {
    pub root: PathBuf,
    pub directories: Vec<PathBuf>,
    pub files: BTreeMap<PathBuf, String>,
    /// Exact recognized partial tree; empty on a new setup. Bound into the plan ID.
    pub present: Vec<PathBuf>,
    pub resuming: bool,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootSetupResult {
    pub root: PathBuf,
    pub resumed: bool,
    pub files: usize,
}

#[derive(Debug)]
pub enum RootSetupError {
    Configuration(String),
    Conflict(String),
    FileSystem { path: PathBuf, source: io::Error },
    Write(AtomicCreateError),
}

impl RootSetupError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Configuration(_) => 3,
            Self::Conflict(_) => 5,
            Self::FileSystem { .. } => 6,
            Self::Write(error) => error.exit_code(),
        }
    }
}
impl fmt::Display for RootSetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) | Self::Conflict(message) => f.write_str(message),
            Self::FileSystem { path, source } => write!(
                f,
                "root setup at {path:?}: {source}; preserve partial files and prepare a fresh setup review"
            ),
            Self::Write(error) => write!(
                f,
                "{error}; preserve partial files and prepare a fresh setup review"
            ),
        }
    }
}
impl std::error::Error for RootSetupError {}
impl From<AtomicCreateError> for RootSetupError {
    fn from(error: AtomicCreateError) -> Self {
        Self::Write(error)
    }
}
fn io_at<T>(path: &Path, result: io::Result<T>) -> Result<T, RootSetupError> {
    result.map_err(|source| RootSetupError::FileSystem {
        path: path.into(),
        source,
    })
}
fn conflict(path: &Path, reason: &str) -> RootSetupError {
    RootSetupError::Conflict(format!(
        "root setup refuses {path:?}: {reason}; preserve existing files, inspect/back up partial setup or choose a new destination"
    ))
}

/// Read-only preparation. The parent must exist; final symlinks and every unowned directory refuse.
pub fn prepare_root_setup(destination: &Path) -> Result<RootSetupPlan, RootSetupError> {
    if !matches!(
        destination.components().next_back(),
        Some(Component::Normal(_))
    ) || destination
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(RootSetupError::Configuration(
            "setup destination must name a new directory without parent traversal".into(),
        ));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = io_at(parent, fs::canonicalize(parent))?;
    let root = parent.join(destination.file_name().expect("normal final component"));
    if root.to_str().is_none() {
        return Err(RootSetupError::Configuration(
            "setup destination must be UTF-8".into(),
        ));
    }
    let mut plan = bundle(root)?;
    match fs::symlink_metadata(&plan.root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(RootSetupError::FileSystem {
                path: plan.root,
                source,
            });
        }
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(conflict(
                    &plan.root,
                    "destination already exists and is not an owned partial setup",
                ));
            }
            let journal = plan.root.join(JOURNAL);
            let expected = journal_source(&plan);
            check_file(&journal, expected.as_bytes())?;
            plan.resuming = true;
            let mut present = Vec::new();
            inspect_tree(&plan, &plan.root, &mut present)?;
            present.sort();
            plan.present = present;
            // Configuration is the final publication. Its presence requires the complete bundle.
            if plan.present.contains(&PathBuf::from(ROOT_CONFIG_FILE))
                && plan.present.len() != plan.directories.len() + plan.files.len() + 1
            {
                return Err(conflict(
                    &plan.root,
                    "configuration exists but the setup tree is incomplete",
                ));
            }
        }
    }
    plan.plan_id =
        crate::state::content_fingerprint(&serde_json::to_vec(&plan).expect("setup serialization"));
    Ok(plan)
}

fn bundle(root: PathBuf) -> Result<RootSetupPlan, RootSetupError> {
    let configuration = include_str!("../defaults/akasha.toml");
    let config: RootConfig = toml::from_str(configuration).map_err(|error| {
        RootSetupError::Configuration(format!("invalid bundled defaults: {error}"))
    })?;
    validate_root_config(&config)
        .map_err(|error| RootSetupError::Configuration(error.to_string()))?;
    let mut files = BTreeMap::from([
        (PathBuf::from(ROOT_CONFIG_FILE), configuration.into()),
        (config.files.registry, "{}\n".into()),
        (
            config.files.agent_instructions,
            include_str!("../defaults/AGENTS.md").into(),
        ),
    ]);
    for (name, source) in [
        ("session", include_str!("../defaults/templates/session.md")),
        ("handoff", include_str!("../defaults/templates/handoff.md")),
        ("task", include_str!("../defaults/templates/task.md")),
        ("problem", include_str!("../defaults/templates/problem.md")),
        ("idea", include_str!("../defaults/templates/idea.md")),
        (
            "decision",
            include_str!("../defaults/templates/decision.md"),
        ),
        ("entity", include_str!("../defaults/templates/entity.md")),
    ] {
        files.insert(
            config.folders.templates.join(format!("{name}.md")),
            source.into(),
        );
    }
    let mut directories = BTreeSet::from([
        config.folders.templates,
        config.folders.global,
        config.folders.projects,
        config.folders.inbox,
    ]);
    for file in files.keys() {
        for parent in file
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            directories.insert(parent.into());
        }
    }
    Ok(RootSetupPlan {
        root,
        directories: directories.into_iter().collect(),
        files,
        present: vec![],
        resuming: false,
        plan_id: String::new(),
    })
}

fn journal_source(plan: &RootSetupPlan) -> String {
    serde_json::to_string(&(1u32, &plan.root, &plan.directories, &plan.files))
        .expect("journal serialization")
}

fn check_file(path: &Path, expected: &[u8]) -> Result<(), RootSetupError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            conflict(path, "no matching setup journal/file")
        } else {
            RootSetupError::FileSystem {
                path: path.into(),
                source: error,
            }
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(conflict(path, "expected a regular non-symlink file"));
    }
    if io_at(path, fs::read(path))? != expected {
        return Err(conflict(
            path,
            "bytes differ from the bundled setup manifest",
        ));
    }
    Ok(())
}

fn inspect_tree(
    plan: &RootSetupPlan,
    directory: &Path,
    present: &mut Vec<PathBuf>,
) -> Result<(), RootSetupError> {
    for entry in io_at(directory, fs::read_dir(directory))? {
        let entry = io_at(directory, entry)?;
        let path = entry.path();
        let relative = path
            .strip_prefix(&plan.root)
            .expect("tree entry")
            .to_path_buf();
        let metadata = io_at(&path, fs::symlink_metadata(&path))?;
        if metadata.file_type().is_symlink() {
            return Err(conflict(&path, "symlink in partial setup"));
        }
        if plan.directories.contains(&relative) && metadata.is_dir() {
            inspect_tree(plan, &path, present)?;
        } else if let Some(source) = plan.files.get(&relative) {
            check_file(&path, source.as_bytes())?;
        } else if relative == Path::new(JOURNAL) {
            check_file(&path, journal_source(plan).as_bytes())?;
        } else {
            return Err(conflict(&path, "unexpected path in partial setup"));
        }
        present.push(relative);
    }
    Ok(())
}

/// Apply only this exact review. Returned failures retain partial state; a fresh review is required.
pub fn apply_root_setup(plan: &RootSetupPlan) -> Result<RootSetupResult, RootSetupError> {
    // Reject tampering before even creating the persistent sibling lock.
    if prepare_root_setup(&plan.root)? != *plan {
        return Err(conflict(
            &plan.root,
            "setup plan changed; prepare a fresh review",
        ));
    }
    let parent = plan.root.parent().expect("canonical root parent");
    let lock_path = parent.join(format!(
        ".{}.akasha-setup.lock",
        plan.root.file_name().expect("root name").to_string_lossy()
    ));
    let _lock = acquire_lock(&lock_path)?;
    if prepare_root_setup(&plan.root)? != *plan {
        return Err(conflict(
            &plan.root,
            "setup plan changed while acquiring the lock",
        ));
    }
    if !plan.resuming {
        private_directory(&plan.root)?;
        io_at(parent, sync_directory(parent))?;
        checkpoint("reserved");
        create_file_atomically(plan.root.join(JOURNAL), journal_source(plan).as_bytes())?;
        io_at(&plan.root, sync_directory(&plan.root))?;
        checkpoint("journal");
    }
    for directory in &plan.directories {
        if !plan.present.contains(directory) {
            private_directory(&plan.root.join(directory))?;
        }
    }
    checkpoint("directories");
    for (path, source) in &plan.files {
        if path != Path::new(ROOT_CONFIG_FILE) && !plan.present.contains(path) {
            create_file_atomically(plan.root.join(path), source.as_bytes())?;
            checkpoint(path.to_str().expect("bundled path"));
        }
    }
    // Persist all supporting files/directories before the root becomes resolvable.
    sync_tree(plan)?;
    if !plan.present.contains(&PathBuf::from(ROOT_CONFIG_FILE)) {
        create_file_atomically(
            plan.root.join(ROOT_CONFIG_FILE),
            plan.files[Path::new(ROOT_CONFIG_FILE)].as_bytes(),
        )?;
    }
    io_at(&plan.root, sync_directory(&plan.root))?;
    checkpoint("configured");
    // Re-check the complete tree before removing recovery authority.
    let current = prepare_root_setup(&plan.root)?;
    if current.present.len() != plan.directories.len() + plan.files.len() + 1 {
        return Err(conflict(
            &plan.root,
            "setup did not produce the complete manifest",
        ));
    }
    sync_tree(plan)?;
    let journal = plan.root.join(JOURNAL);
    io_at(
        &journal,
        io_call!(&journal, Remove, fs::remove_file(&journal)),
    )?;
    checkpoint("unlinked");
    io_at(&plan.root, sync_directory(&plan.root))?;
    Ok(RootSetupResult {
        root: plan.root.clone(),
        resumed: plan.resuming,
        files: plan.files.len(),
    })
}

fn sync_tree(plan: &RootSetupPlan) -> Result<(), RootSetupError> {
    for directory in plan.directories.iter().rev() {
        let path = plan.root.join(directory);
        io_at(&path, sync_directory(&path))?;
    }
    io_at(&plan.root, sync_directory(&plan.root))
}
fn private_directory(path: &Path) -> Result<(), RootSetupError> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    io_at(path, io_call!(path, Create, builder.create(path)))
}
struct SetupLock(File);
impl Drop for SetupLock {
    fn drop(&mut self) {
        // Release explicitly even if an unrelated fork temporarily inherited the file.
        let _ = self.0.unlock();
    }
}
fn acquire_lock(path: &Path) -> Result<SetupLock, RootSetupError> {
    match create_file_atomically(path, b"") {
        Ok(()) => io_at(
            path.parent().expect("lock parent"),
            sync_directory(path.parent().expect("lock parent")),
        )?,
        Err(AtomicCreateError::Conflict { .. }) => check_file(path, b"")?,
        Err(error) => return Err(error.into()),
    }
    let file = io_at(path, OpenOptions::new().read(true).write(true).open(path))?;
    match file.try_lock() {
        Ok(()) => Ok(SetupLock(file)),
        Err(TryLockError::WouldBlock) => {
            Err(conflict(path, "another setup writer is busy; retry later"))
        }
        Err(TryLockError::Error(source)) => Err(RootSetupError::FileSystem {
            path: path.into(),
            source,
        }),
    }
}

fn checkpoint(_stage: &str) {
    #[cfg(test)]
    tests::checkpoint(_stage);
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "setup_error_tests.rs"]
mod error_tests;
