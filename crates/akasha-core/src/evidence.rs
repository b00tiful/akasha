use std::path::Path;

use crate::project_validation::{ProjectValidationError, canonical_note_paths};
use crate::resolution::RootConfig;
use crate::state::CanonicalNoteEvidence;

pub(crate) fn collect_canonical_evidence<E>(
    project_dir: &Path,
    config: &RootConfig,
    mut read_source: impl FnMut(&Path) -> Result<Vec<u8>, E>,
) -> Result<Vec<CanonicalNoteEvidence>, E>
where
    E: From<ProjectValidationError>,
{
    let mut evidence = Vec::new();
    for note_type in config.project.note_types.values() {
        for path in canonical_note_paths(&project_dir.join(&note_type.folder))? {
            let source = read_source(&path)?;
            evidence.push(CanonicalNoteEvidence {
                path,
                class: note_type.class,
                source,
            });
        }
    }
    Ok(evidence)
}
