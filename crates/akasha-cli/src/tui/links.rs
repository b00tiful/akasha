//! In-memory report bindings; graph authority stays in the shared core.

use akasha_core::{
    LibraryDocument, LinkedLibraryDocument, NoteLinkDirection, NoteLinkKind, NoteLinksError,
    NoteLinksResult, ResolveRequest, activate_library_backlink, activate_library_document_link,
    inspect_library_backlinks, inspect_note_links, load_linked_library_document,
};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Report {
    pub result: NoteLinksResult,
    pub limit: usize,
    pub source: Option<LinkedLibraryDocument>,
}

impl Report {
    pub fn inspect(
        request: &ResolveRequest,
        id: &str,
        direction: NoteLinkDirection,
        limit: usize,
        canonical: bool,
    ) -> Result<Self, NoteLinksError> {
        // The loaded catalog only enables binding; the core checks read authority.
        let source = canonical
            .then(|| load_linked_library_document(request, id))
            .transpose()?;
        let result = match (&source, direction) {
            (Some(source), NoteLinkDirection::Incoming) => {
                inspect_library_backlinks(request, id, &source.document.source, limit)?
            }
            _ => inspect_note_links(request, id, direction, limit)?,
        };
        if let Some(source) = &source
            && load_linked_library_document(request, id)?.document.source != source.document.source
        {
            return Err(NoteLinksError::StaleSource(id.to_owned()));
        }
        Ok(Self {
            result,
            limit,
            source,
        })
    }

    pub fn follow(
        &self,
        request: &ResolveRequest,
        index: usize,
    ) -> Result<LibraryDocument, String> {
        let link = self
            .result
            .links
            .get(index)
            .ok_or("No result at that number.")?;
        if link.kind != NoteLinkKind::Canonical {
            return Err(
                "Projection/other results are inspection only; choose a canonical result.".into(),
            );
        }
        let source = self.source.as_ref().ok_or(
            "This report has no canonical source binding. Refresh the library and inspect a canonical note to use /follow.",
        )?;
        let document = match self.result.direction {
            NoteLinkDirection::Incoming => activate_library_backlink(
                request,
                &self.result.note,
                &source.document.source,
                &link.id,
                self.limit,
            ),
            NoteLinkDirection::Outgoing => {
                let occurrence = source
                    .links
                    .iter()
                    .find(|occurrence| {
                        occurrence.target_id == link.id
                            && occurrence.kind == NoteLinkKind::Canonical
                    })
                    .ok_or("The result has no bound canonical occurrence. Inspect again.")?;
                activate_library_document_link(
                    request,
                    &self.result.note,
                    &source.document.source,
                    occurrence.from,
                )
            }
        }
        .map_err(|error| error.to_string())?;
        Ok(document.document)
    }

    pub fn text(&self) -> String {
        format!(
            "{}\nSaved files only; editor drafts are excluded.\n/follow NUMBER opens a bound canonical result; Esc returns here, then to the library list.\n/open PATH remains available. Projection/other entries and projection reports are inspection only.\n",
            crate::render::numbered_note_links_text(&self.result),
        )
    }
}
