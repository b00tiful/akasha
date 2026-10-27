use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::project_validation::ProjectValidationError;
use crate::validation::ValidationError;

/// Maximum exact bytes in an ordinary canonical note or configured index/roadmap.
/// Historical transaction recovery deliberately retains its existing unrestricted reads.
pub const MAX_MARKDOWN_SOURCE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn check_markdown_source(
    path: &Path,
    source: &[u8],
) -> Result<(), ProjectValidationError> {
    if source.len() > MAX_MARKDOWN_SOURCE_BYTES {
        Err(excessive_source(path))
    } else {
        Ok(())
    }
}

fn excessive_source(path: &Path) -> ProjectValidationError {
    ProjectValidationError::InvalidDocument {
        path: path.to_path_buf(),
        source: Box::new(ValidationError::InvalidSchema {
            document: "Markdown source",
            message: format!(
                "source exceeds the {MAX_MARKDOWN_SOURCE_BYTES}-byte (16 MiB) limit; split or reduce the document before retrying"
            ),
        }),
    }
}

pub(crate) fn read_markdown_source(
    path: &Path,
    operation: &'static str,
) -> Result<Vec<u8>, ProjectValidationError> {
    let io_error = |source| ProjectValidationError::FileSystem {
        operation,
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(io_error)?;
    // Do not trust a metadata length: a source may grow after inspection or during this read.
    let source = read_with_sentinel(file, MAX_MARKDOWN_SOURCE_BYTES).map_err(io_error)?;
    check_markdown_source(path, &source)?;
    Ok(source)
}

fn read_with_sentinel(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut source = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut source)?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_boundary_preserves_bytes_and_excess_precedes_decoding() {
        let mut source = vec![b'x'; MAX_MARKDOWN_SOURCE_BYTES];
        source[..6].copy_from_slice("界\r\n!".as_bytes());
        check_markdown_source(Path::new("exact.md"), &source).unwrap();
        source.push(0xff);
        let error = check_markdown_source(Path::new("excess.md"), &source).unwrap_err();
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("16777216-byte"));
        assert!(!error.to_string().contains("UTF-8"));
    }

    #[test]
    fn streaming_reader_consumes_only_limit_plus_one_even_when_it_grows() {
        // This reader has no usable size hint and keeps producing data after the allowance.
        struct GrowingReader(usize);
        impl Read for GrowingReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = buffer.len().min(3);
                buffer[..count].fill(b'x');
                self.0 += count;
                Ok(count)
            }
        }
        let mut reader = GrowingReader(0);
        assert_eq!(read_with_sentinel(&mut reader, 16).unwrap(), vec![b'x'; 17]);
        assert_eq!(reader.0, 17);
        for size in [0, 15, 16, 17, 100] {
            let input = vec![b'x'; size];
            assert_eq!(
                read_with_sentinel(input.as_slice(), 16).unwrap(),
                input[..size.min(17)]
            );
        }
    }
}
