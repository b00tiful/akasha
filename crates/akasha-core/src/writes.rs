use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_STAGING_ATTEMPTS: u64 = 128;
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);
pub(crate) const PROJECT_WRITE_LOCK_FILE: &str = ".akasha-write.lock";

/// An exclusive-creation conflict or operational filesystem failure.
#[derive(Debug)]
pub enum AtomicCreateError {
    Conflict {
        path: PathBuf,
        source: io::Error,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl AtomicCreateError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Conflict { .. } => 5,
            Self::FileSystem { .. } => 6,
        }
    }
}

impl fmt::Display for AtomicCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { path, .. } => {
                write!(
                    formatter,
                    "refusing to overwrite existing path {}",
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
        }
    }
}

impl Error for AtomicCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Conflict { source, .. } | Self::FileSystem { source, .. } => Some(source),
        }
    }
}

/// An optimistic replacement conflict or operational filesystem failure.
#[derive(Debug)]
pub(crate) enum CheckedReplaceError {
    Conflict {
        path: PathBuf,
        source: io::Error,
    },
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for CheckedReplaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { path, .. } => write!(
                formatter,
                "checked replacement target changed at {}",
                path.display()
            ),
            Self::FileSystem {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} at {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for CheckedReplaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Conflict { source, .. } | Self::FileSystem { source, .. } => Some(source),
        }
    }
}

/// Create one regular file without exposing partial content or overwriting an existing path.
///
/// The destination's parent must already exist. Content is written and synced to an exclusively
/// created staging file in that directory, then published with an atomic hard link. The final
/// publication therefore fails if a file, directory, or symlink already occupies the destination.
pub fn create_file_atomically(
    destination: impl AsRef<Path>,
    contents: &[u8],
) -> Result<(), AtomicCreateError> {
    create_file_atomically_with(destination.as_ref(), |file| file.write_all(contents))
}

/// Replace one regular file only while its exact bytes still match the caller's snapshot.
pub(crate) fn replace_file_if_unchanged(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<bool, CheckedReplaceError> {
    let path = canonical_destination(path).map_err(map_create_error)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|source| CheckedReplaceError::FileSystem {
            operation: "inspect a checked replacement target",
            path: path.clone(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CheckedReplaceError::Conflict {
            path,
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "checked replacement target is not a regular file",
            ),
        });
    }
    let current = fs::read(&path).map_err(|source| CheckedReplaceError::FileSystem {
        operation: "read a checked replacement target",
        path: path.clone(),
        source,
    })?;
    if current != expected {
        return Err(changed_replacement(&path));
    }
    if replacement == expected {
        return Ok(false);
    }

    let (mut file, staging) = create_staging_file(&path).map_err(map_create_error)?;
    file.write_all(replacement)
        .map_err(|source| CheckedReplaceError::FileSystem {
            operation: "write a checked replacement stage",
            path: staging.path.clone(),
            source,
        })?;
    file.set_permissions(metadata.permissions())
        .map_err(|source| CheckedReplaceError::FileSystem {
            operation: "preserve checked replacement permissions",
            path: staging.path.clone(),
            source,
        })?;
    file.sync_all()
        .map_err(|source| CheckedReplaceError::FileSystem {
            operation: "sync a checked replacement stage",
            path: staging.path.clone(),
            source,
        })?;
    drop(file);

    let current = fs::read(&path).map_err(|source| CheckedReplaceError::FileSystem {
        operation: "verify a checked replacement target",
        path: path.clone(),
        source,
    })?;
    if current != expected {
        return Err(changed_replacement(&path));
    }
    fs::rename(&staging.path, &path).map_err(|source| CheckedReplaceError::FileSystem {
        operation: "publish a checked replacement",
        path: path.clone(),
        source,
    })?;
    Ok(true)
}

pub(crate) fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

/// One crash-released advisory lock shared by every project mutation path.
pub(crate) struct ProjectWriteLock {
    _file: File,
}

impl ProjectWriteLock {
    pub(crate) fn acquire(project_dir: &Path) -> Result<Self, AtomicCreateError> {
        let project_dir =
            fs::canonicalize(project_dir).map_err(|source| AtomicCreateError::FileSystem {
                operation: "resolve the project writer lock directory",
                path: project_dir.to_path_buf(),
                source,
            })?;
        let path = project_dir.join(PROJECT_WRITE_LOCK_FILE);
        match create_file_atomically(&path, b"") {
            Ok(()) => {
                sync_directory(&project_dir).map_err(|source| AtomicCreateError::FileSystem {
                    operation: "sync the project writer lock directory",
                    path: project_dir.clone(),
                    source,
                })?
            }
            Err(error @ AtomicCreateError::Conflict { .. }) => {
                let metadata = fs::symlink_metadata(&path).map_err(|source| {
                    AtomicCreateError::FileSystem {
                        operation: "inspect the project writer lock",
                        path: path.clone(),
                        source,
                    }
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| AtomicCreateError::FileSystem {
                operation: "open the project writer lock",
                path: path.clone(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(AtomicCreateError::Conflict {
                path,
                source: io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another Akasha project writer holds the lock",
                ),
            }),
            Err(TryLockError::Error(source)) => Err(AtomicCreateError::FileSystem {
                operation: "acquire the project writer lock",
                path,
                source,
            }),
        }
    }
}

fn changed_replacement(path: &Path) -> CheckedReplaceError {
    CheckedReplaceError::Conflict {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "checked replacement target no longer matches the expected bytes",
        ),
    }
}

fn map_create_error(error: AtomicCreateError) -> CheckedReplaceError {
    match error {
        AtomicCreateError::Conflict { path, source } => {
            CheckedReplaceError::Conflict { path, source }
        }
        AtomicCreateError::FileSystem {
            operation,
            path,
            source,
        } => CheckedReplaceError::FileSystem {
            operation,
            path,
            source,
        },
    }
}

fn create_file_atomically_with(
    destination: &Path,
    write_contents: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), AtomicCreateError> {
    let destination = canonical_destination(destination)?;
    reject_existing_destination(&destination)?;

    let (mut file, staging) = create_staging_file(&destination)?;
    if let Err(source) = write_contents(&mut file) {
        let path = staging.path.clone();
        drop(file);
        drop(staging);
        return Err(AtomicCreateError::FileSystem {
            operation: "write the staging file",
            path,
            source,
        });
    }
    if let Err(source) = file.sync_all() {
        let path = staging.path.clone();
        drop(file);
        drop(staging);
        return Err(AtomicCreateError::FileSystem {
            operation: "sync the staging file",
            path,
            source,
        });
    }
    drop(file);

    match fs::hard_link(&staging.path, &destination) {
        Ok(()) => {
            drop(staging);
            Ok(())
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            Err(AtomicCreateError::Conflict {
                path: destination,
                source,
            })
        }
        Err(source) => Err(AtomicCreateError::FileSystem {
            operation: "publish the staging file",
            path: destination,
            source,
        }),
    }
}

fn canonical_destination(destination: &Path) -> Result<PathBuf, AtomicCreateError> {
    let file_name = destination
        .file_name()
        .ok_or_else(|| AtomicCreateError::FileSystem {
            operation: "resolve the destination filename",
            path: destination.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination must name a regular file",
            ),
        })?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|source| AtomicCreateError::FileSystem {
        operation: "resolve the destination directory",
        path: parent.to_path_buf(),
        source,
    })?;

    if !parent.is_dir() {
        return Err(AtomicCreateError::FileSystem {
            operation: "resolve the destination directory",
            path: parent,
            source: io::Error::new(
                io::ErrorKind::NotADirectory,
                "destination parent is not a directory",
            ),
        });
    }

    Ok(parent.join(file_name))
}

fn reject_existing_destination(destination: &Path) -> Result<(), AtomicCreateError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(AtomicCreateError::Conflict {
            path: destination.to_path_buf(),
            source: io::Error::from(io::ErrorKind::AlreadyExists),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AtomicCreateError::FileSystem {
            operation: "inspect the destination",
            path: destination.to_path_buf(),
            source,
        }),
    }
}

fn create_staging_file(destination: &Path) -> Result<(File, StagingFile), AtomicCreateError> {
    let parent = destination
        .parent()
        .expect("canonical destinations always have a parent");
    let file_name = destination
        .file_name()
        .expect("canonical destinations always have a filename");

    for _ in 0..MAX_STAGING_ATTEMPTS {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let mut staging_name = OsString::from(".");
        staging_name.push(file_name);
        staging_name.push(format!(".akasha-{}-{id}.tmp", std::process::id()));
        let path = parent.join(staging_name);

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, StagingFile { path })),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(AtomicCreateError::FileSystem {
                    operation: "create a staging file",
                    path,
                    source,
                });
            }
        }
    }

    Err(AtomicCreateError::FileSystem {
        operation: "create a unique staging file",
        path: destination.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "all staging filename attempts were already occupied",
        ),
    })
}

struct StagingFile {
    path: PathBuf,
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_failure_never_publishes_partial_content() {
        let root = std::env::temp_dir().join(format!(
            "akasha-atomic-interruption-{}-{}",
            std::process::id(),
            NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create test directory");
        let destination = root.join("note.md");

        let error = create_file_atomically_with(&destination, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "simulated interruption",
            ))
        })
        .expect_err("interrupted staging must fail");

        assert_eq!(error.exit_code(), 6);
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(&root).expect("read test directory").count(), 0);
        fs::remove_dir(&root).expect("remove test directory");
    }
}
