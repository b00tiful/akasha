//! Question-only local vocabulary assistance. Shared core owns both independent source packets.
#[cfg(feature = "desktop")]
use crate::inference::InferenceJobs;
use crate::inference::{self, Cancellation, CompletionMetrics, NativeInference, Profile};
use crate::{DesktopError, DesktopLibrarianReview, librarian_error, request};
use akasha_core::librarian::{LibrarianPacket, prepare_librarian, review_librarian_sources};
use akasha_core::librarian_provenance::review_librarian_provenance;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};

const POLICY: &str = include_str!("../../../../scripts/librarian-query-policy.txt");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expansion {
    terms: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AssistedLibrarianReview {
    pub initial: DesktopLibrarianReview,
    pub expanded: Option<DesktopLibrarianReview>,
    pub terms: Vec<String>,
    pub raw_expansion: String,
    pub metrics: CompletionMetrics,
}

fn validate_question(question: &str) -> Result<(), DesktopError> {
    if question.trim().is_empty()
        || question.chars().count() > 160
        || question
            .chars()
            .any(|ch| ch.is_control() || ch.escape_debug().to_string().starts_with("\\u{"))
    {
        return Err(inference::error(
            "Local query assistance requires 1..160 characters without controls",
        ));
    }
    Ok(())
}

fn expansion_terms(answer: &str) -> Result<Vec<String>, DesktopError> {
    if answer.len() > 4096 {
        return Err(inference::error("Query expansion exceeds 4096 bytes"));
    }
    let expansion: Expansion = serde_json::from_str(answer)
        .map_err(|_| inference::error("Invalid query expansion JSON"))?;
    if expansion.terms.len() > 8 {
        return Err(inference::error(
            "Query expansion permits at most eight terms",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut terms = Vec::new();
    for term in expansion.terms {
        if !(2..=64).contains(&term.chars().count())
            || term.trim() != term
            || !term.chars().next().is_some_and(char::is_alphanumeric)
            || !term
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '_' | '.' | '-'))
        {
            return Err(inference::error("Invalid literal query expansion term"));
        }
        if seen.insert(term.to_lowercase()) {
            terms.push(term);
        }
    }
    Ok(terms)
}

fn checked_review(
    root: Option<PathBuf>,
    project: String,
    packet: LibrarianPacket,
    provenance: bool,
) -> Result<DesktopLibrarianReview, DesktopError> {
    let request = request(root, Some(project))?;
    let review = if provenance {
        review_librarian_provenance(&request, &packet, r#"{"selections":[]}"#)
    } else {
        review_librarian_sources(&request, &packet, r#"{"selections":[]}"#)
    }
    .map_err(librarian_error)?;
    Ok(DesktopLibrarianReview { packet, review })
}

/// One selected-model completion sees no project data. Both searches stay separately
/// reconstructible by core; initial passages cannot be replaced by expanded candidates.
pub async fn assist_librarian(
    root: Option<PathBuf>,
    project: String,
    question: String,
    provenance: bool,
    profile: Profile,
    cancellation: Cancellation,
) -> Result<AssistedLibrarianReview, DesktopError> {
    validate_question(&question)?;
    cancellation.check()?;
    let req = request(root.clone(), Some(project.clone()))?;
    let search = req.clone();
    let query = question.clone();
    let initial = tokio::task::spawn_blocking(move || {
        prepare_librarian(&search, &query, &[], None).map_err(librarian_error)
    })
    .await
    .map_err(|_| inference::error("Librarian source preparation failed"))??;
    cancellation.check()?;
    let mut worker = NativeInference::open(profile, cancellation.clone()).await?;
    let completion = worker
        .complete(
            &json!({"question":question}),
            &json!({"type":"object","additionalProperties":false,"required":["terms"],"properties":{"terms":{"type":"array","maxItems":8,"items":{"type":"string","minLength":2,"maxLength":64}}}}),
            POLICY.trim_end(),
        )
        .await;
    // Stop/reap and remove credentials before search/check/display, including failure.
    worker.close().await?;
    let (raw_expansion, metrics) = completion?;
    cancellation.check()?;
    let terms = expansion_terms(&raw_expansion)?;
    let search_terms = terms.clone();
    let result = tokio::task::spawn_blocking(move || {
        let expanded = if search_terms.is_empty() {
            None
        } else {
            let packet =
                prepare_librarian(&req, &question, &search_terms, None).map_err(librarian_error)?;
            Some(checked_review(
                root.clone(),
                project.clone(),
                packet,
                provenance,
            )?)
        };
        // Rebuild the original search after inference as well: stale input fails loudly.
        let initial = checked_review(root, project, initial, provenance)?;
        Ok::<_, DesktopError>((initial, expanded))
    })
    .await
    .map_err(|_| inference::error("Librarian source checking failed"))??;
    cancellation.check()?;
    Ok(AssistedLibrarianReview {
        initial: result.0,
        expanded: result.1,
        terms,
        raw_expansion,
        metrics,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn assist_librarian_sources(
    jobs: tauri::State<'_, InferenceJobs>,
    request_id: String,
    root: Option<PathBuf>,
    project: String,
    question: String,
    provenance: bool,
    profile: Profile,
) -> Result<AssistedLibrarianReview, DesktopError> {
    let lease = jobs.claim(&request_id)?;
    assist_librarian(
        root,
        project,
        question,
        provenance,
        profile,
        lease.cancellation.clone(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_is_bounded_literal_and_deduplicated_without_trusting_grammar() {
        assert_eq!(
            expansion_terms(r#"{"terms":["Queue","queue","重试","повтор","job_id","Qwen3.5"]}"#)
                .unwrap(),
            vec!["Queue", "重试", "повтор", "job_id", "Qwen3.5"]
        );
        assert!(expansion_terms(r#"{"terms":[]}"#).unwrap().is_empty());
        for answer in [
            r#"[]"#,
            r#"{"terms":[],"facts":[]}"#,
            r#"{"terms":[1]}"#,
            r#"{"terms":["/etc/passwd"]}"#,
            r#"{"terms":["--root"]}"#,
            r#"{"terms":["x"]}"#,
            r#"{"terms":[" two"]}"#,
            r#"{"terms":["two\nlines"]}"#,
            r#"{"terms":["ab\u202ecd"]}"#,
            r#"{"terms":["<script>"]}"#,
        ] {
            assert!(expansion_terms(answer).is_err(), "{answer}");
        }
        assert!(expansion_terms(&json!({"terms":vec!["term";9]}).to_string()).is_err());
        assert!(expansion_terms(&json!({"terms":["字".repeat(65)]}).to_string()).is_err());
        assert!(expansion_terms(&" ".repeat(4097)).is_err());
    }

    #[test]
    fn question_bounds_precede_source_or_worker_access() {
        for question in ["", " ", "two\nlines", "ab\u{202e}cd", &"字".repeat(161)] {
            assert!(validate_question(question).is_err());
        }
        assert!(validate_question("队列的容量是多少？").is_ok());
        assert!(validate_question(&"字".repeat(160)).is_ok());
    }
}
