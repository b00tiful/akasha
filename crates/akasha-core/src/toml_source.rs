use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::project_validation::ProjectValidationError;
use crate::resolution::ResolveError;

pub(crate) const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;

// A metadata length is not evidence of the bytes returned by a growing source.
pub(crate) fn read_with_sentinel(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn check_config_source(
    path: &Path,
    description: &str,
    source: &[u8],
) -> Result<(), ResolveError> {
    if source.len() > MAX_CONFIG_BYTES {
        Err(ResolveError::Configuration(format!(
            "{description} at {} exceeds the {MAX_CONFIG_BYTES}-byte (1 MiB) limit; reduce the configuration before retrying",
            path.display()
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn check_state_source(source: &[u8]) -> Result<(), String> {
    if source.len() > MAX_STATE_BYTES {
        Err(format!(
            "project state exceeds the {MAX_STATE_BYTES}-byte (16 MiB) limit; preserve the trusted event history and consult the resource-limits recovery guidance"
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn read_state_source(
    path: &Path,
    operation: &'static str,
) -> Result<Vec<u8>, ProjectValidationError> {
    let io_error = |source| ProjectValidationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    };
    let bytes = read_with_sentinel(File::open(path).map_err(io_error)?, MAX_STATE_BYTES)
        .map_err(io_error)?;
    check_state_source(&bytes).map_err(|message| ProjectValidationError::InvalidState {
        path: path.to_path_buf(),
        message,
    })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn sentinel_stops_unknown_length_growth_and_preserves_exact_bytes() {
        for limit in [MAX_CONFIG_BYTES, MAX_STATE_BYTES] {
            let exact = vec![0xff; limit];
            assert_eq!(
                read_with_sentinel(Cursor::new(&exact), limit).unwrap(),
                exact
            );
            let mut growing = io::repeat(b'x');
            assert_eq!(
                read_with_sentinel(&mut growing, limit).unwrap().len(),
                limit + 1
            );
        }
        assert!(read_with_sentinel(Cursor::new([]), 0).unwrap().is_empty());
        assert_eq!(read_with_sentinel(io::repeat(1), 0).unwrap(), [1]);
    }

    #[test]
    fn sentinel_retains_io_errors_after_a_partial_read() {
        struct Broken(bool);
        impl Read for Broken {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    Err(io::Error::other("synthetic read failure"))
                } else {
                    self.0 = true;
                    output[0] = b'x';
                    Ok(1)
                }
            }
        }
        assert_eq!(
            read_with_sentinel(Broken(false), 4)
                .unwrap_err()
                .to_string(),
            "synthetic read failure"
        );
    }
}
