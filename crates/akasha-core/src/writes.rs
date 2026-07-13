use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_STAGING_ATTEMPTS: u64 = 128;
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

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
