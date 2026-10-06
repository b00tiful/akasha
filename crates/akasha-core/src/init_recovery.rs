//! Read-only recovery review and snapshot-bound recovery without starting another init.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitRecoveryDirectory {
    pub path: PathBuf,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitRecoveryFile {
    pub path: PathBuf,
    pub present: bool,
    pub expected_fingerprint: String,
}

/// Metadata-only review. Present uncommitted artifacts will be removed; committed
/// artifacts will be retained. The registry is never replaced by recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitRecoveryPlan {
    pub root: PathBuf,
    pub registry: PathBuf,
    pub journal_path: PathBuf,
    pub project: String,
    pub repository_dir: PathBuf,
    pub repository_present: bool,
    pub project_dir: PathBuf,
    pub recovery: InitRecovery,
    pub configuration_fingerprint: String,
    pub journal_fingerprint: String,
    pub registry_fingerprint: String,
    pub directories: Vec<InitRecoveryDirectory>,
    pub files: Vec<InitRecoveryFile>,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitRecoveryResult {
    pub root: PathBuf,
    pub registry: PathBuf,
    pub journal_path: PathBuf,
    pub project: String,
    pub repository_dir: PathBuf,
    pub project_dir: PathBuf,
    pub recovery: InitRecovery,
    pub plan_id: String,
}

/// Validate a pending init transaction without locking, recovering or writing.
/// No project selection, repository pointer or current template tree is required.
pub fn prepare_init_recovery(
    request: &ResolveRequest,
) -> Result<Option<InitRecoveryPlan>, InitError> {
    Ok(prepare_review(request)?.map(|(plan, _)| plan))
}

/// Revalidate the entire review under the registry lock, then recover only that
/// transaction. Refusals preserve artifacts; the persistent lock may be created.
pub fn apply_init_recovery(
    request: &ResolveRequest,
    plan: &InitRecoveryPlan,
) -> Result<InitRecoveryResult, InitError> {
    let (_, registry, _, _) = recovery_target(request)?;
    let _lock = InitLock::acquire(&registry)?;
    let Some((current, checked)) = prepare_review(request)? else {
        return Err(stale_plan(&registry));
    };
    if &current != plan || current.registry != registry {
        return Err(stale_plan(&registry));
    }
    finish_checked_init_recovery(&registry, &checked)?;
    Ok(InitRecoveryResult {
        root: current.root,
        registry: current.registry,
        journal_path: current.journal_path,
        project: current.project,
        repository_dir: current.repository_dir,
        project_dir: current.project_dir,
        recovery: current.recovery,
        plan_id: current.plan_id,
    })
}

fn stale_plan(path: &Path) -> InitError {
    InitError::Conflict {
        path: path.into(),
        message: "initialization recovery plan changed; prepare and review a fresh plan".into(),
    }
}

fn recovery_target(
    request: &ResolveRequest,
) -> Result<(PathBuf, PathBuf, PathBuf, Vec<u8>), InitError> {
    let (root, _) = resolve_root(request)?;
    let config_path = root.join(ROOT_CONFIG_FILE);
    let config_source = read_regular_init_file(&config_path, "read recovery configuration")?;
    let config = load_root_config(&root)?;
    let projects_dir = canonicalize_directory(
        &root.join(&config.folders.projects),
        "configured projects directory",
    )?;
    ensure_inside_root(&projects_dir, &root, "configured projects directory")?;
    let (registry, _) = load_project_registry(&root, &config)?;
    if read_regular_init_file(&config_path, "recheck recovery configuration")? != config_source {
        return Err(stale_plan(&config_path));
    }
    Ok((root, registry, projects_dir, config_source))
}

fn prepare_review(
    request: &ResolveRequest,
) -> Result<Option<(InitRecoveryPlan, CheckedInitRecovery)>, InitError> {
    let (root, registry, projects_dir, config_source) = recovery_target(request)?;
    let Some(checked) = check_init_recovery(&registry, &projects_dir)? else {
        return Ok(None);
    };
    let journal_path = init_journal_path(&registry);
    let journal = &checked.journal;
    let repository_present = verify_journal_repository(
        &journal_path,
        &journal.repository_dir,
        checked.recovery == InitRecovery::Finalized,
    )?;
    let mut directories = Vec::new();
    for path in std::iter::once(journal.project_dir.clone()).chain(
        journal
            .directories
            .iter()
            .map(|path| journal.project_dir.join(path)),
    ) {
        let present = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
            Ok(_) => return Err(unexpected_init_bytes(&journal_path)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(source) => {
                return Err(InitError::FileSystem {
                    operation: "inspect recovery directory",
                    path,
                    source,
                });
            }
        };
        directories.push(InitRecoveryDirectory { path, present });
    }
    let mut files = Vec::new();
    for (path, expected) in journal
        .files
        .iter()
        .map(|file| (journal.project_dir.join(&file.path), &file.after))
        .chain(std::iter::once((
            journal.repository_dir.join(POINTER_FILE),
            &journal.pointer_after,
        )))
    {
        let present =
            inspect_optional_init_file(&journal_path, &path, expected, "recovery artifact")?;
        files.push(InitRecoveryFile {
            path,
            present,
            expected_fingerprint: expected.clone(),
        });
    }
    let mut plan = InitRecoveryPlan {
        root,
        registry,
        journal_path,
        project: journal.project.clone(),
        repository_dir: journal.repository_dir.clone(),
        repository_present,
        project_dir: journal.project_dir.clone(),
        recovery: checked.recovery,
        configuration_fingerprint: content_fingerprint(&config_source),
        journal_fingerprint: content_fingerprint(&checked.source),
        registry_fingerprint: checked.registry_fingerprint.clone(),
        directories,
        files,
        plan_id: String::new(),
    };
    let identity = serde_json::to_vec(&("akasha-init-recovery-v1", &plan)).map_err(|error| {
        InitError::Validation {
            path: plan.journal_path.clone(),
            message: format!("cannot encode initialization recovery plan: {error}"),
        }
    })?;
    plan.plan_id = content_fingerprint(&identity);
    Ok(Some((plan, checked)))
}
