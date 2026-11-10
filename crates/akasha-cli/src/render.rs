use std::env;
use std::fmt::{Display, Write as _};
use std::io::{self, IsTerminal, Write as _};

use akasha_core::{
    AgentWiringAction, AgentWiringOperation, AgentWiringPlan, AgentWiringRecovery,
    AgentWiringResult, ContextBundle, ContextOmission, DEFAULT_CONTEXT_MAX_CHARS,
    EntityUpdateResult, EventCreationResult, InitRecovery, InitResult, LinkResult,
    MutableNoteCreationResult, NoteClass, NoteEditRecovery, ProjectValidationReport,
    RecordUpdateResult, ResolvedProject, SessionBreadcrumb, SessionHookWiringAction,
    SessionHookWiringOperation, SessionHookWiringPlan, SessionHookWiringRecovery,
    SessionHookWiringResult, context_section_label, render_context_markdown,
    render_session_breadcrumb,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputMode {
    json: bool,
    color: bool,
}

impl OutputMode {
    pub(crate) fn detect(json: bool, no_color: bool) -> Self {
        Self {
            json,
            color: !json
                && io::stdout().is_terminal()
                && !no_color
                && env::var_os("NO_COLOR").is_none(),
        }
    }
}

pub(crate) fn render_local_onboarding_plan(
    plan: &akasha_core::onboarding_local::LocalOnboardingIdentityPlan,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(plan))?);
    }
    let mut text =
        String::from("Planned initial notes (filename roles; semantic review required)\n");
    for slot in &plan.slots {
        writeln!(
            text,
            "Slot {} [{}] {}: {}\n  Source handles: {:?}",
            slot.slot,
            librarian_display(&slot.role),
            librarian_display(&slot.note_type),
            librarian_display(&slot.path.to_string_lossy()),
            slot.sources
        )
        .expect("writing to a string cannot fail");
        for (key, value) in &slot.fields {
            writeln!(
                text,
                "  {}: {}",
                librarian_display(key),
                librarian_display(value)
            )
            .expect("writing to a string cannot fail");
        }
        if !slot.unresolved_fields.is_empty() {
            writeln!(
                text,
                "  Needs reviewed metadata: {:?}",
                slot.unresolved_fields
            )
            .expect("writing to a string cannot fail");
        }
    }
    write_stdout(&text)
}

pub(crate) fn render_local_onboarding_discovery(
    discovery: &akasha_core::onboarding_discovery::LocalOnboardingDiscovery,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(discovery))?);
    }
    let coverage = &discovery.coverage;
    let mut text = format!(
        "Local onboarding source discovery (no writes/model)\nInspected: {} entries; {} candidates; {} selected; {} excluded entries\nCoverage is limited to the requested directories and selected source lines.\n",
        coverage.entries_inspected,
        coverage.candidates,
        coverage.selected.len(),
        coverage.excluded_entries
    );
    if let Some(focus) = &discovery.packet.focus {
        let _ = writeln!(
            text,
            "Literal focus: {}\nEligible source bytes inspected successfully: {}",
            librarian_display(&focus.terms.join(", ")),
            coverage.focus_scan_bytes.unwrap_or(0)
        );
    }
    for group in &coverage.implementation_groups {
        let _ = writeln!(
            text,
            "Implementation group [{}]: {} — {}/{} candidate files selected (filename heuristic)",
            if group.selected == 0 {
                "unrepresented"
            } else {
                "represented"
            },
            group.directory.escape_debug(),
            group.selected,
            group.candidates
        );
    }
    for source in &coverage.selected {
        let _ = writeln!(
            text,
            "Selected [{}]: {}",
            source.role,
            source.path.escape_debug()
        );
        if !source.matched_terms.is_empty() {
            let _ = writeln!(
                text,
                "  Matched focus terms: {}",
                librarian_display(&source.matched_terms.join(", "))
            );
        }
    }
    for source in &discovery.packet.sources {
        if let Some(sampling) = &source.sampling {
            append_local_sampling(&mut text, &source.path, source.lines, sampling);
        }
    }
    for source in &coverage.omitted {
        let _ = writeln!(
            text,
            "Omitted [{}]: {}",
            source.reason,
            source.path.escape_debug()
        );
    }
    for source in &coverage.excluded_examples {
        let _ = writeln!(
            text,
            "Excluded example [{}]: {}",
            source.reason,
            source.path.escape_debug()
        );
    }
    write_stdout(&text)
}

pub(crate) fn render_local_onboarding_packet(
    packet: &akasha_core::onboarding_local::LocalOnboardingPacket,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(packet))?);
    }
    let mut text = format!(
        "Local onboarding input — {}\nExplicit sources: {} ({}; no writes)\n",
        librarian_display(packet.preparation["project"].as_str().unwrap_or("")),
        packet.sources.len(),
        if packet
            .sources
            .iter()
            .any(|source| source.sampling.is_some())
        {
            "coverage below"
        } else {
            "complete files"
        }
    );
    if let Some(focus) = &packet.focus {
        let _ = writeln!(
            text,
            "Literal focus: {} (partial windows remain unknown outside selected lines)",
            librarian_display(&focus.terms.join(", "))
        );
    }
    for source in &packet.sources {
        if let Some(sampling) = &source.sampling {
            append_local_sampling(&mut text, &source.path, source.lines, sampling);
        }
        let _ = writeln!(
            text,
            "{} — {} lines — {} excerpts — {}",
            librarian_display(&source.path),
            source.lines,
            source.excerpts.len(),
            source.fingerprint
        );
        for (index, excerpt) in source.excerpts.iter().enumerate() {
            let _ = writeln!(
                text,
                "  Excerpt {}: lines {}-{}",
                index + 1,
                excerpt.line_start,
                excerpt.line_end
            );
            if source.sampling.is_some() {
                for (offset, line) in source.text[excerpt.byte_start..excerpt.byte_end]
                    .lines()
                    .enumerate()
                {
                    let _ = writeln!(
                        text,
                        "  {} | {}",
                        excerpt.line_start + offset,
                        librarian_display(line)
                    );
                }
            }
        }
        if source.sampling.is_none() {
            for line in source.text.lines() {
                let _ = writeln!(text, "  | {}", librarian_display(line));
            }
        }
    }
    write_stdout(&text)
}

fn append_local_sampling(
    text: &mut String,
    path: &str,
    lines: usize,
    sampling: &akasha_core::onboarding_local::LocalOnboardingSampling,
) {
    let omitted: usize = sampling
        .omitted_lines
        .iter()
        .map(|range| range.line_end - range.line_start + 1)
        .sum();
    let _ = writeln!(
        text,
        "Sampled: {} — {}/{} lines selected, {} whole-file bytes; partial coverage",
        librarian_display(path),
        lines - omitted,
        lines,
        sampling.total_bytes
    );
    for range in &sampling.omitted_lines {
        let _ = writeln!(
            text,
            "  Omitted lines: {}-{}",
            range.line_start, range.line_end
        );
    }
}

pub(crate) fn render_local_onboarding_draft(
    draft: &akasha_core::onboarding_local::LocalOnboardingDraft,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(draft))?);
    }
    let mut text = format!(
        "Local onboarding draft — {}\nNo writes. Factual support needs review.\nProposal: {}\nPreview: {}\n",
        librarian_display(&draft.preview.project),
        draft.preview.proposal_id,
        draft.preview.preview_id
    );
    for gap in &draft.review.uncited_focus_excerpts {
        let _ = writeln!(
            text,
            "Uncited focus excerpt: {}:{}-{} ({}); inspect before approval",
            librarian_display(&gap.path),
            gap.line_start,
            gap.line_end,
            librarian_display(&gap.matched_terms.join(", "))
        );
    }
    for source in &draft.review.sampled_sources {
        append_local_sampling(
            &mut text,
            &source.path,
            source.total_lines,
            &source.sampling,
        );
        for range in &source.selected_lines {
            let _ = writeln!(
                text,
                "  Selected lines: {}-{}",
                range.line_start, range.line_end
            );
        }
    }
    for preview in &draft.preview.notes {
        let note = draft
            .proposal
            .notes
            .iter()
            .find(|note| note.note_type == preview.note_type && note.path == preview.path)
            .expect("core preview names one exact proposed note");
        let _ = writeln!(
            text,
            "\n{} ({:?}, {} evidence claims)",
            librarian_display(&preview.path.display().to_string()),
            preview.action,
            preview.evidence_claims
        );
        for line in note.source.lines() {
            let _ = writeln!(text, "  | {}", librarian_display(line));
        }
        let review = draft
            .review
            .notes
            .iter()
            .find(|review| review.note_type == note.note_type && review.path == note.path)
            .expect("core review names one exact note");
        let _ = writeln!(
            text,
            "  Source review (exact excerpts; meaning and metadata need approval):"
        );
        for (index, claim) in review.claims.iter().enumerate() {
            let _ = writeln!(
                text,
                "  Claim {} [{}]: {}",
                index + 1,
                librarian_display(&claim.kind),
                librarian_display(&claim.claim)
            );
            if claim.sources.is_empty() {
                let _ = writeln!(text, "    No supporting source (unknown).");
            }
            for source in &claim.sources {
                let _ = writeln!(
                    text,
                    "    {}:{}-{} — {}",
                    librarian_display(&source.path),
                    source.line_start,
                    source.line_end,
                    source.fingerprint
                );
                for (offset, line) in source.excerpt.lines().enumerate() {
                    let _ = writeln!(
                        text,
                        "    {} | {}",
                        source.line_start + offset,
                        librarian_display(line)
                    );
                }
            }
        }
    }
    if !draft.review.uncited_sources.is_empty() {
        let _ = writeln!(
            text,
            "\nSelected but uncited: {}",
            draft
                .review
                .uncited_sources
                .iter()
                .map(|s| librarian_display(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for (label, source) in [
        ("Index", &draft.proposal.index),
        ("Roadmap", &draft.proposal.roadmap),
    ] {
        let _ = writeln!(text, "\n{label}:");
        for line in source.lines() {
            let _ = writeln!(text, "  | {}", librarian_display(line));
        }
    }
    write_stdout(&text)
}

pub(crate) fn render_local_onboarding_result(
    result: &akasha_core::OnboardingBatchResult,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(result))?);
    }
    let mut text = format!(
        "Local onboarding applied — {}\n",
        librarian_display(&result.project)
    );
    for (action, paths) in [
        ("Created", &result.created_notes),
        ("Unchanged", &result.unchanged_notes),
        ("Updated projection", &result.updated_projections),
    ] {
        for path in paths {
            let _ = writeln!(
                text,
                "{action}: {}",
                librarian_display(&path.display().to_string())
            );
        }
    }
    let _ = writeln!(
        text,
        "Project state: {}",
        librarian_display(&result.state.display().to_string())
    );
    write_stdout(&text)
}

pub(crate) fn render_librarian_packet(
    packet: &akasha_core::librarian::LibrarianPacket,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(packet))?);
    }
    let mut text = format!(
        "Librarian evidence — {}\n{} matched / {} scanned; {} omitted\n",
        librarian_display(&packet.project),
        packet.matched_notes,
        packet.scanned_notes,
        packet.omitted_notes
    );
    if packet.evidence.is_empty() {
        text.push_str("Insufficient evidence: no matching canonical notes.\n");
    }
    for item in &packet.evidence {
        let _ = writeln!(
            text,
            "[{}] {} (lines {}–{}, status: {}, partial: {})\nSHA-256: {}",
            item.citation,
            librarian_display(&item.id),
            item.line_start,
            item.line_end,
            librarian_display(item.status.as_deref().unwrap_or("unknown")),
            item.excerpt_truncated,
            item.fingerprint
        );
        for line in item.excerpt.lines() {
            let _ = writeln!(text, "  | {}", librarian_display(line));
        }
    }
    write_stdout(&text)
}

pub(crate) fn render_librarian_answer(
    checked: &akasha_core::librarian::CheckedLibrarianAnswer,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(checked))?);
    }
    let mut text = String::from(
        "Local librarian draft — citation identities checked; factual support still needs review.\n",
    );
    if checked.answer.insufficient_evidence {
        text.push_str("Insufficient evidence for a complete answer.\n");
    }
    for claim in &checked.answer.claims {
        let _ = writeln!(
            text,
            "- {} {:?}",
            librarian_display(&claim.text),
            claim.citations
        );
    }
    for source in &checked.evidence {
        if !checked
            .answer
            .claims
            .iter()
            .any(|claim| claim.citations.contains(&source.citation))
        {
            continue;
        }
        let _ = writeln!(
            text,
            "[{}] {}:{}–{}",
            source.citation,
            librarian_display(&source.id),
            source.line_start,
            source.line_end
        );
        let _ = writeln!(
            text,
            "  Recorded status: {}{}",
            librarian_display(source.status.as_deref().unwrap_or("not recorded")),
            if source.excerpt_truncated {
                "; partial body"
            } else {
                ""
            }
        );
        for line in source.excerpt.lines() {
            let _ = writeln!(text, "  | {}", librarian_display(line));
        }
    }
    write_stdout(&text)
}

pub(crate) fn render_librarian_sources(
    checked: &akasha_core::librarian::CheckedLibrarianSources,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(checked))?);
    }
    write_stdout(&librarian_sources_text(checked))
}

pub(crate) fn render_librarian_source_review(
    review: &akasha_core::librarian::LibrarianSourceReview,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(review))?);
    }
    let mut text = format!(
        "Complete source review — all {} prepared passages retained; relevance needs review.\n{} matched / {} scanned; {} matching notes outside this packet.\nHighlights are guidance, not an answer or a completeness verdict.\n",
        review.sources.matches.len(),
        review.matched_notes,
        review.scanned_notes,
        review.omitted_notes
    );
    if review.sources.matches.is_empty() {
        text.push_str("No sources retrieved. Retrieval may be incomplete.\n");
    }
    for highlighted in [true, false] {
        text.push_str(if highlighted {
            "\nHighlighted sources:\n"
        } else {
            "\nAdditional retrieved sources (not highlighted):\n"
        });
        let mut count = 0;
        for item in &review.sources.matches {
            if review.highlighted_citations.contains(&item.citation) == highlighted {
                count += 1;
                librarian_passage_text(&mut text, item);
            }
        }
        if count == 0 {
            text.push_str("None.\n");
        }
    }
    text.push_str(
        "Link scope: complete wikilinks in these quotes targeting this evidence packet.\n",
    );
    write_stdout(&text)
}

fn librarian_sources_text(checked: &akasha_core::librarian::CheckedLibrarianSources) -> String {
    let mut text = String::from("Source passages — exact text verified; relevance needs review.\n");
    if checked.matches.is_empty() {
        text.push_str("No source passages selected. Retrieval may be incomplete.\n");
    }
    for item in &checked.matches {
        librarian_passage_text(&mut text, item);
    }
    text.push_str(
        "Link scope: complete wikilinks in these quotes targeting this evidence packet.\n",
    );
    text
}

fn librarian_passage_text(
    text: &mut String,
    item: &akasha_core::librarian::LibrarianSourcePassage,
) {
    let _ = writeln!(
        text,
        "[{}] {}:{}–{}\nRecorded status: {}\nPartial body: {}",
        item.citation,
        librarian_display(&item.id),
        item.line_start,
        item.line_end,
        librarian_display(item.status.as_deref().unwrap_or("not recorded")),
        item.partial_body
    );
    for line in item.quote.lines() {
        let _ = writeln!(text, "  | {}", librarian_display(line));
    }
    for link in &item.links {
        let _ = writeln!(
            text,
            "Recorded wikilink at line {}: {} -> {}",
            link.line,
            librarian_display(&link.wikilink),
            librarian_display(&link.target)
        );
    }
    if item.omitted_links > 0 {
        let _ = writeln!(
            text,
            "{} quoted links omitted by scope/limit.",
            item.omitted_links
        );
    }
}

pub(crate) fn render_librarian_provenance(
    report: &akasha_core::librarian_provenance::LibrarianProvenance,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(report))?);
    }
    let mut text = librarian_sources_text(&report.sources);
    text.push_str(
        "\nRecorded repository evidence — unchanged bytes do not establish factual support.\n",
    );
    for note in &report.notes {
        let _ = writeln!(
            text,
            "[{}] Evidence: {}; {} recorded claims outside/ambiguous in this quote.",
            note.citation, note.evidence_state, note.unmatched_claims
        );
        for claim in &note.claims {
            let _ = writeln!(
                text,
                "  Recorded {} at note line {}: {:?}",
                claim.recorded_kind, claim.note_line, claim.claim
            );
            if let Some(reason) = &claim.rationale {
                let _ = writeln!(text, "  Rationale: {reason:?}");
            }
            for source in &claim.sources {
                let lines = match (source.line_start, source.line_end) {
                    (Some(start), Some(end)) => format!("{start}–{end}"),
                    _ => "not recorded".into(),
                };
                let _ = writeln!(
                    text,
                    "  Repository source: {:?} lines {}\n  Freshness: {}; excerpt: {}\n  Recorded: {:?}\n  Observed: {:?}",
                    source.path,
                    lines,
                    source.freshness,
                    source.excerpt_state,
                    source.recorded_fingerprint,
                    source
                        .observed_fingerprint
                        .as_deref()
                        .unwrap_or("not available")
                );
                if let Some(excerpt) = &source.excerpt {
                    for (index, line) in excerpt.lines().enumerate() {
                        let _ = writeln!(
                            text,
                            "    {} | {:?}",
                            source.line_start.unwrap_or(1) + index,
                            line
                        );
                    }
                }
            }
        }
    }
    write_stdout(&text)
}

fn librarian_display(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

pub(crate) fn render_library_report(
    report: &akasha_core::LibraryReport,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(report))?);
    }
    write_stdout(&library_report_text(report))
}

pub(crate) fn library_report_text(report: &akasha_core::LibraryReport) -> String {
    let display = |value: &str| {
        let mut chars = value.chars();
        let mut text: String = chars
            .by_ref()
            .take(120)
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect();
        if chars.next().is_some() {
            text.push('…');
        }
        text
    };
    let title = match report.kind {
        akasha_core::LibraryReportKind::Status => "Cross-project status (project order)",
        akasha_core::LibraryReportKind::Digest => {
            "Cross-project digest (latest canonical note date)"
        }
    };
    let dashboard = &report.dashboard;
    let mut text = format!(
        "{title}\nAll registered projects and global knowledge validated.\n{} projects; {} notes ({} global); {} configured categories\n{} open tasks; {} open problems; {} canonical outgoing links\nLatest canonical note date: {}\nShowing {} of {} projects{}\n",
        dashboard.projects,
        dashboard.notes,
        dashboard.global_notes,
        dashboard.configured_categories,
        dashboard.open_tasks,
        dashboard.open_problems,
        dashboard.validated_links,
        display(dashboard.latest_activity_date.as_deref().unwrap_or("none")),
        dashboard.project_metrics.len(),
        dashboard.projects,
        if report.truncated {
            " (truncated; increase --limit)"
        } else {
            ""
        },
    );
    for project in &dashboard.project_metrics {
        writeln!(
            text,
            "{} [{}]\n  {} notes; {} populated categories; {} open tasks; {} open problems; {} canonical outgoing links; latest date: {}",
            display(&project.project),
            display(&project.status),
            project.notes,
            project.populated_categories,
            project.open_tasks,
            project.open_problems,
            project.validated_links,
            display(project.latest_activity_date.as_deref().unwrap_or("none")),
        ).expect("writing to a string cannot fail");
    }
    text
}

pub(crate) fn render_search(
    result: &akasha_core::LibrarySearchResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        writeln!(
            rendered,
            "{} matches; showing {}{}",
            result.total_matches,
            result.hits.len(),
            if result.truncated {
                " (truncated; narrow the query or increase --limit)"
            } else {
                ""
            }
        )
        .expect("writing to a string cannot fail");
        for hit in &result.hits {
            let id: String = hit
                .id
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            writeln!(
                rendered,
                "{}{}\n  {}",
                id,
                hit.line.map(|line| format!(":{line}")).unwrap_or_default(),
                hit.snippet
            )
            .expect("writing to a string cannot fail");
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_note_links(
    result: &akasha_core::NoteLinksResult,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        return write_stdout(&json_line(serde_json::to_string_pretty(result))?);
    }
    write_stdout(&note_links_text(result))
}

pub(crate) fn note_links_text(result: &akasha_core::NoteLinksResult) -> String {
    note_links_report_text(result, false)
}

pub(crate) fn numbered_note_links_text(result: &akasha_core::NoteLinksResult) -> String {
    note_links_report_text(result, true)
}

fn note_links_report_text(result: &akasha_core::NoteLinksResult, numbered: bool) -> String {
    let display = |id: &str| {
        id.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
    };
    let direction = match result.direction {
        akasha_core::NoteLinkDirection::Outgoing => "Outgoing links",
        akasha_core::NoteLinkDirection::Incoming => "Backlinks",
    };
    let mut text = format!(
        "{direction}: {}\nScanned {} canonical notes and {} project projections across all registered projects and global knowledge.\n{} matches; showing {}{}\n",
        display(&result.note),
        result.scanned_canonical_notes,
        result.scanned_projections,
        result.total_matches,
        result.links.len(),
        if result.truncated {
            " (truncated; increase --limit)"
        } else {
            ""
        },
    );
    for (index, link) in result.links.iter().enumerate() {
        let kind = match link.kind {
            akasha_core::NoteLinkKind::Canonical => "canonical",
            akasha_core::NoteLinkKind::Projection => "projection",
            akasha_core::NoteLinkKind::Other => "other",
        };
        if numbered {
            write!(text, "{}. ", index + 1).expect("writing to a string cannot fail");
        }
        writeln!(text, "{} [{kind}]", display(&link.id)).expect("writing to a string cannot fail");
    }
    text
}

pub(crate) fn render_init(result: &InitResult, output: OutputMode) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "initialized", &result.project, output);
        print_field(
            &mut rendered,
            "repository",
            result.repository_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project directory",
            result.project_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "templates copied",
            result.template_files,
            output,
        );
        print_field(&mut rendered, "registry", result.registry.display(), output);
        print_field(&mut rendered, "pointer", result.pointer.display(), output);
        print_field(
            &mut rendered,
            "recovery",
            init_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn init_recovery_name(recovery: InitRecovery) -> &'static str {
    match recovery {
        InitRecovery::None => "none",
        InitRecovery::Discarded => "discarded",
        InitRecovery::RolledBack => "rolled-back",
        InitRecovery::Finalized => "finalized",
    }
}

pub(crate) fn init_recovery_plan_text(plan: &akasha_core::InitRecoveryPlan) -> String {
    let finalized = plan.recovery == InitRecovery::Finalized;
    let mut text = format!(
        "INITIALIZATION RECOVERY REVIEW · NO FILES CHANGED\n\nPlan ID: {}\nOutcome: {}\nProject: {:?}\nRoot: {:?}\nRepository: {:?}\nRepository present: {}\nProject directory: {:?}\nRegistry (retained): {:?}\nRegistry SHA-256: {}\nConfiguration SHA-256: {}\nJournal (removed on success): {:?}\nJournal SHA-256: {}\n\nFiles (absolute paths, exact expected hashes):\n",
        plan.plan_id,
        init_recovery_name(plan.recovery),
        plan.project,
        plan.root,
        plan.repository_dir,
        plan.repository_present,
        plan.project_dir,
        plan.registry,
        plan.registry_fingerprint,
        plan.configuration_fingerprint,
        plan.journal_path,
        plan.journal_fingerprint,
    );
    for file in &plan.files {
        let action = if !file.present {
            "absent"
        } else if finalized {
            "retain"
        } else {
            "remove"
        };
        text.push_str(&format!(
            "  {action}: {:?}\n    {}\n",
            file.path, file.expected_fingerprint
        ));
    }
    text.push_str("\nDirectories (absolute paths):\n");
    for directory in &plan.directories {
        let action = if !directory.present {
            "absent"
        } else if finalized {
            "retain"
        } else {
            "remove when empty"
        };
        text.push_str(&format!("  {action}: {:?}\n", directory.path));
    }
    text.push_str(&format!(
        "\nPlan ID: {}\n\nRecovery removes only exact uncommitted artifacts or finalizes the committed transaction. Registry bytes are retained. No new project is initialized. Stop other writers and make a private backup before applying. Changed bytes, paths or presence require a fresh review; keep the journal on refusal and consult docs/recovery.md. A persistent registry lock file may remain after apply/refusal.\n",
        plan.plan_id,
    ));
    text
}

pub(crate) fn init_recovery_result_text(result: &akasha_core::InitRecoveryResult) -> String {
    format!(
        "Initialization recovery completed.\nOutcome: {}\nProject: {:?}\nRoot: {:?}\nRepository: {:?}\nProject directory: {:?}\nJournal removed: {:?}\nRegistry retained: {:?}\n\nNo new project was initialized. {}\n",
        init_recovery_name(result.recovery),
        result.project,
        result.root,
        result.repository_dir,
        result.project_dir,
        result.journal_path,
        result.registry,
        if result.recovery == InitRecovery::Finalized {
            "Refresh and select the finalized project."
        } else {
            "Prepare a fresh init review to create the project."
        },
    )
}

pub(crate) fn render_init_recovery_plan(
    plan: Option<&akasha_core::InitRecoveryPlan>,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(&plan))?)
    } else if let Some(plan) = plan {
        write_stdout(&format!(
            "{}\nTo apply only this recovery: akasha --root PATH recover-init --plan-id {}\nUse the exact reviewed root for PATH. Omitting apply cancels without writing.\n",
            init_recovery_plan_text(plan),
            plan.plan_id
        ))
    } else {
        write_stdout("No pending initialization journal. No files changed.\n")
    }
}

pub(crate) fn render_init_recovery_result(
    result: &akasha_core::InitRecoveryResult,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(result))?)
    } else {
        write_stdout(&init_recovery_result_text(result))
    }
}

pub(crate) fn render_link(result: &LinkResult, output: OutputMode) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "linked", &result.project, output);
        print_field(
            &mut rendered,
            "repository",
            result.repository_dir.display(),
            output,
        );
        print_field(&mut rendered, "pointer", result.pointer.display(), output);
        print_field(
            &mut rendered,
            "project directory",
            result.project_dir.display(),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_agent_wiring_plan(
    plan: &AgentWiringPlan,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(plan))?;
    } else {
        print_status(
            &mut rendered,
            "prepared agent wiring",
            plan.client.as_str(),
            output,
        );
        print_field(&mut rendered, "data root", plan.root.display(), output);
        print_field(
            &mut rendered,
            "instruction source",
            plan.source.display(),
            output,
        );
        print_field(&mut rendered, "source sha256", &plan.source_sha256, output);
        print_field(&mut rendered, "target", plan.target.display(), output);
        print_field(
            &mut rendered,
            "operation",
            agent_wiring_operation_name(plan.operation),
            output,
        );
        print_field(
            &mut rendered,
            "action",
            agent_wiring_action_name(plan.action),
            output,
        );
        print_field(
            &mut rendered,
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(
            &mut rendered,
            "result sha256",
            plan.result_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(&mut rendered, "plan id", &plan.plan_id, output);
        print_field(
            &mut rendered,
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field(
            &mut rendered,
            "replacement bytes",
            plan.patch.replacement.len(),
            output,
        );
        if plan.patch.replacement.is_empty() {
            print_field(&mut rendered, "replacement", "none", output);
        } else {
            print_field(&mut rendered, "replacement", "", output);
            rendered.push_str(&plan.patch.replacement);
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_agent_wiring_result(
    result: &AgentWiringResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(
            &mut rendered,
            match result.operation {
                AgentWiringOperation::Apply => "applied agent wiring",
                AgentWiringOperation::Remove => "removed agent wiring",
            },
            result.client.as_str(),
            output,
        );
        print_field(&mut rendered, "target", result.target.display(), output);
        print_field(
            &mut rendered,
            "action",
            agent_wiring_action_name(result.action),
            output,
        );
        print_field(&mut rendered, "changed", result.changed, output);
        print_field(&mut rendered, "plan id", &result.plan_id, output);
        print_field(
            &mut rendered,
            "recovery",
            agent_wiring_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_session_hook_wiring_plan(
    plan: &SessionHookWiringPlan,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(plan))?;
    } else {
        print_status(
            &mut rendered,
            "prepared session hook",
            plan.client.as_str(),
            output,
        );
        print_field(&mut rendered, "data root", plan.root.display(), output);
        print_field(&mut rendered, "target", plan.target.display(), output);
        print_field(
            &mut rendered,
            "operation",
            session_hook_operation_name(plan.operation),
            output,
        );
        print_field(
            &mut rendered,
            "action",
            session_hook_action_name(plan.action),
            output,
        );
        print_field(
            &mut rendered,
            "current sha256",
            plan.current_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(
            &mut rendered,
            "result sha256",
            plan.result_sha256.as_deref().unwrap_or("absent"),
            output,
        );
        print_field(&mut rendered, "plan id", &plan.plan_id, output);
        print_field(
            &mut rendered,
            "patch range",
            format_args!("{}..{}", plan.patch.start, plan.patch.end),
            output,
        );
        print_field(
            &mut rendered,
            "replacement bytes",
            plan.patch.replacement.len(),
            output,
        );
        if plan.patch.replacement.is_empty() {
            print_field(&mut rendered, "replacement", "none", output);
        } else {
            print_field(&mut rendered, "replacement", "", output);
            rendered.push_str(&plan.patch.replacement);
        }
    }
    write_stdout(&rendered)
}

pub(crate) fn render_session_hook_wiring_result(
    result: &SessionHookWiringResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(
            &mut rendered,
            match result.operation {
                SessionHookWiringOperation::Apply => "applied session hook",
                SessionHookWiringOperation::Remove => "removed session hook",
            },
            result.client.as_str(),
            output,
        );
        print_field(&mut rendered, "target", result.target.display(), output);
        print_field(
            &mut rendered,
            "action",
            session_hook_action_name(result.action),
            output,
        );
        print_field(&mut rendered, "changed", result.changed, output);
        print_field(&mut rendered, "plan id", &result.plan_id, output);
        print_field(
            &mut rendered,
            "recovery",
            session_hook_recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

fn session_hook_operation_name(operation: SessionHookWiringOperation) -> &'static str {
    match operation {
        SessionHookWiringOperation::Apply => "apply",
        SessionHookWiringOperation::Remove => "remove",
    }
}

pub(crate) const fn session_hook_action_name(action: SessionHookWiringAction) -> &'static str {
    match action {
        SessionHookWiringAction::Create => "create",
        SessionHookWiringAction::AddHooks => "add-hooks",
        SessionHookWiringAction::AddSessionStart => "add-session-start",
        SessionHookWiringAction::AppendSessionStart => "append-session-start",
        SessionHookWiringAction::RemoveSessionStartEntry => "remove-session-start-entry",
        SessionHookWiringAction::RemoveSessionStartKey => "remove-session-start-key",
        SessionHookWiringAction::RemoveHooksKey => "remove-hooks-key",
        SessionHookWiringAction::RemoveManagedFile => "remove-managed-file",
        SessionHookWiringAction::NoChange => "no-change",
    }
}

fn session_hook_recovery_name(recovery: SessionHookWiringRecovery) -> &'static str {
    match recovery {
        SessionHookWiringRecovery::None => "none",
        SessionHookWiringRecovery::Discarded => "discarded",
        SessionHookWiringRecovery::Finalized => "finalized",
    }
}

pub(crate) fn render_event_creation(
    result: &EventCreationResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "created event", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(&mut rendered, "template", result.template.display(), output);
        print_field(
            &mut rendered,
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_mutable_note_creation(
    result: &MutableNoteCreationResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "created note", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(
            &mut rendered,
            "class",
            note_class_name(result.class),
            output,
        );
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(&mut rendered, "template", result.template.display(), output);
        print_field(
            &mut rendered,
            "template scope",
            template_scope_name(result.template_scope),
            output,
        );
        print_field(
            &mut rendered,
            "projection",
            result.projection.display(),
            output,
        );
        print_field(
            &mut rendered,
            "projection changed",
            if result.projection_changed {
                "yes"
            } else {
                "no"
            },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_record_update(
    result: &RecordUpdateResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "updated record", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(
            &mut rendered,
            "record changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field(&mut rendered, "roadmap", result.roadmap.display(), output);
        print_field(
            &mut rendered,
            "roadmap changed",
            if result.roadmap_changed { "yes" } else { "no" },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_entity_update(
    result: &EntityUpdateResult,
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        rendered = json_line(serde_json::to_string_pretty(result))?;
    } else {
        print_status(&mut rendered, "updated entity", &result.id, output);
        print_field(&mut rendered, "project", &result.project, output);
        print_field(&mut rendered, "type", &result.note_type, output);
        print_field(&mut rendered, "path", result.path.display(), output);
        print_field(
            &mut rendered,
            "entity changed",
            if result.changed { "yes" } else { "no" },
            output,
        );
        print_field(&mut rendered, "index", result.index.display(), output);
        print_field(
            &mut rendered,
            "index changed",
            if result.index_changed { "yes" } else { "no" },
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            result.state.display(),
            output,
        );
        print_field(
            &mut rendered,
            "recovery",
            recovery_name(result.recovery),
            output,
        );
    }
    write_stdout(&rendered)
}

pub(crate) fn render_resolution(resolved: &ResolvedProject, output: OutputMode) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(resolved))?)
    } else {
        write_stdout(&resolution_text_with_mode(resolved, output))
    }
}

pub(crate) fn resolution_text(resolved: &ResolvedProject) -> String {
    resolution_text_with_mode(
        resolved,
        OutputMode {
            json: false,
            color: false,
        },
    )
}

fn resolution_text_with_mode(resolved: &ResolvedProject, output: OutputMode) -> String {
    let mut rendered = String::new();
    print_field(&mut rendered, "root", resolved.root.display(), output);
    print_field(
        &mut rendered,
        "root source",
        root_source_name(resolved.root_source),
        output,
    );
    print_field(&mut rendered, "project", &resolved.project, output);
    print_field(
        &mut rendered,
        "project source",
        project_source_name(resolved.project_source),
        output,
    );
    match &resolved.pointer {
        Some(pointer) => print_field(&mut rendered, "pointer", pointer.display(), output),
        None => print_field(
            &mut rendered,
            "pointer",
            "none (project selected explicitly)",
            output,
        ),
    }
    print_field(
        &mut rendered,
        "registry",
        resolved.registry.display(),
        output,
    );
    print_field(
        &mut rendered,
        "repository",
        resolved.repository_dir.display(),
        output,
    );
    print_field(
        &mut rendered,
        "project directory",
        resolved.project_dir.display(),
        output,
    );
    rendered
}

pub(crate) fn render_validation(
    report: &ProjectValidationReport,
    context_warnings: &[ContextOmission],
    output: OutputMode,
) -> io::Result<()> {
    let mut rendered = String::new();
    if output.json {
        // Additive field: existing report keys are unchanged.
        let value = serde_json::to_value(report).and_then(|mut value| {
            value["context_warnings"] = serde_json::to_value(context_warnings)?;
            Ok(value)
        });
        rendered = json_line(value.and_then(|value| serde_json::to_string_pretty(&value)))?;
    } else {
        print_status(&mut rendered, "valid", &report.project, output);
        print_field(&mut rendered, "registry", report.registry.display(), output);
        print_field(
            &mut rendered,
            "repository",
            report.repository_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "project directory",
            report.project_dir.display(),
            output,
        );
        print_field(
            &mut rendered,
            "registry projects",
            report.registry_projects,
            output,
        );
        print_field(
            &mut rendered,
            "canonical notes",
            report.canonical_notes,
            output,
        );
        print_field(
            &mut rendered,
            "immutable events",
            report.immutable_events,
            output,
        );
        print_field(
            &mut rendered,
            "project state",
            report.state.display(),
            output,
        );
        for (name, projection) in &report.projections {
            print_field(
                &mut rendered,
                "projection",
                format_args!("{name} — {} sources", projection.sources),
                output,
            );
        }
        print_field(&mut rendered, "wikilinks", report.wikilinks, output);
        for (name, note_type) in &report.note_types {
            print_field(
                &mut rendered,
                "note type",
                format_args!(
                    "{name} ({}) — {}",
                    note_class_name(note_type.class),
                    note_type.notes
                ),
                output,
            );
        }
        for warning in context_warnings {
            print_field(
                &mut rendered,
                "context warning",
                context_warning_text(warning),
                output,
            );
        }
    }
    write_stdout(&rendered)
}

/// One orientation-core misfit, shared by named `validate` and terminal `/validate`.
pub(crate) fn context_warning_text(warning: &ContextOmission) -> String {
    format!(
        "{} `{}` ({} characters) does not fit the {}-character orientation-core limit; compact it",
        context_section_label(warning.section),
        warning.source.display(),
        warning.content_chars,
        DEFAULT_CONTEXT_MAX_CHARS
    )
}

/// Advisory text for a created handoff above the orientation guideline.
pub(crate) fn oversized_handoff_text(chars: usize) -> String {
    format!(
        "handoff body is {chars} characters, over the {}-character orientation guideline; keep handoffs concise and link records or entities for detail",
        akasha_core::HANDOFF_ORIENTATION_GUIDELINE_CHARS
    )
}

pub(crate) fn render_context(context: &ContextBundle, output: OutputMode) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(context))?)
    } else {
        write_stdout(&render_context_markdown(context))
    }
}

pub(crate) fn render_breadcrumb(
    breadcrumb: &SessionBreadcrumb,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(breadcrumb))?)
    } else {
        write_stdout(&render_session_breadcrumb(breadcrumb))
    }
}

fn print_status(rendered: &mut String, label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        writeln!(rendered, "\x1b[1;32m{label}\x1b[0m: {value}")
            .expect("writing to a string cannot fail");
    } else {
        writeln!(rendered, "{label}: {value}").expect("writing to a string cannot fail");
    }
}

fn print_field(rendered: &mut String, label: &str, value: impl Display, output: OutputMode) {
    if output.color {
        writeln!(rendered, "\x1b[1;36m{label}\x1b[0m: {value}")
            .expect("writing to a string cannot fail");
    } else {
        writeln!(rendered, "{label}: {value}").expect("writing to a string cannot fail");
    }
}

fn json_line(serialized: Result<String, serde_json::Error>) -> io::Result<String> {
    let mut rendered = serialized.map_err(io::Error::other)?;
    rendered.push('\n');
    Ok(rendered)
}

fn write_stdout(rendered: &str) -> io::Result<()> {
    io::stdout().lock().write_all(rendered.as_bytes())
}

const fn root_source_name(source: akasha_core::RootSource) -> &'static str {
    match source {
        akasha_core::RootSource::CommandLine => "command-line",
        akasha_core::RootSource::Environment => "environment",
        akasha_core::RootSource::UserConfig => "user-config",
    }
}

const fn project_source_name(source: akasha_core::ProjectSource) -> &'static str {
    match source {
        akasha_core::ProjectSource::CommandLine => "command-line",
        akasha_core::ProjectSource::Pointer => "pointer",
    }
}

const fn note_class_name(class: NoteClass) -> &'static str {
    match class {
        NoteClass::Event => "event",
        NoteClass::Record => "record",
        NoteClass::Entity => "entity",
    }
}

const fn template_scope_name(scope: akasha_core::NoteTemplateScope) -> &'static str {
    match scope {
        akasha_core::NoteTemplateScope::Project => "project",
        akasha_core::NoteTemplateScope::Root => "root",
    }
}

const fn recovery_name(recovery: NoteEditRecovery) -> &'static str {
    match recovery {
        NoteEditRecovery::None => "none",
        NoteEditRecovery::Discarded => "discarded",
        NoteEditRecovery::RolledBack => "rolled-back",
        NoteEditRecovery::Finalized => "finalized",
    }
}

pub(crate) const fn agent_wiring_action_name(action: AgentWiringAction) -> &'static str {
    match action {
        AgentWiringAction::Create => "create",
        AgentWiringAction::Append => "append",
        AgentWiringAction::RefreshManagedSection => "refresh-managed-section",
        AgentWiringAction::RemoveManagedSection => "remove-managed-section",
        AgentWiringAction::RemoveCreatedFile => "remove-created-file",
        AgentWiringAction::NoChange => "no-change",
    }
}

const fn agent_wiring_operation_name(operation: AgentWiringOperation) -> &'static str {
    match operation {
        AgentWiringOperation::Apply => "apply",
        AgentWiringOperation::Remove => "remove",
    }
}

const fn agent_wiring_recovery_name(recovery: AgentWiringRecovery) -> &'static str {
    match recovery {
        AgentWiringRecovery::None => "none",
        AgentWiringRecovery::Discarded => "discarded",
        AgentWiringRecovery::Finalized => "finalized",
    }
}

pub(crate) fn setup_plan_text(plan: &akasha_core::RootSetupPlan) -> String {
    let mut text = format!(
        "ROOT SETUP REVIEW · NO FILES CHANGED\n\nRoot: {:?}\nResume owned partial setup: {}\nPlan ID: {}\n\nDirectories (private on Unix):\n",
        plan.root, plan.resuming, plan.plan_id
    );
    for path in &plan.directories {
        writeln!(text, "  {path:?}").unwrap();
    }
    text.push_str("\nFiles (exact UTF-8 JSON strings; private on Unix):\n");
    for (path, source) in &plan.files {
        writeln!(
            text,
            "\n{path:?}\n{}",
            serde_json::to_string(source).unwrap()
        )
        .unwrap();
    }
    writeln!(text, "\nExisting matching partial paths: {:?}\n\nPlan ID: {}\n\nApply creates/reuses a persistent sibling setup lock and a temporary root setup journal. Existing roots are never overwritten. Partial setup is retained on error; fresh review resumes only matching files. No user or client configuration is changed. Use --root PATH for later commands or configure AKASHA_ROOT explicitly.\n", plan.present, plan.plan_id).unwrap();
    text
}

pub(crate) fn setup_result_text(result: &akasha_core::RootSetupResult) -> String {
    format!(
        "Root configured.\nRoot: {:?}\nFiles: {}\nResumed: {}\n\nNext: from your repository, run akasha --root PATH init PROJECT.\nThen run akasha --root PATH onboard for the optional connected-agent handoff.\nReplace PATH with the root above. No client or user configuration was changed.",
        result.root, result.files, result.resumed
    )
}

pub(crate) fn onboarding_handoff_text(handoff: &akasha_core::OnboardingHandoff) -> String {
    format!(
        "AGENT ONBOARDING HANDOFF · NO FILES CHANGED\n\nProject: {:?}\nRoot: {:?}\nRepository: {:?}\n\n{}\n\nStdio command (JSON string): {}\nArguments (JSON array): {}\n\nRequest for the connected agent:\n{}\n\nAfter approved application, run akasha --root PATH --project PROJECT validate with the identities above.\n",
        handoff.project,
        handoff.root,
        handoff.repository,
        handoff.next_step,
        serde_json::to_string(&handoff.command).unwrap(),
        serde_json::to_string(&handoff.args).unwrap(),
        handoff.agent_request
    )
}

pub(crate) fn render_setup_plan(
    plan: &akasha_core::RootSetupPlan,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(plan))?)
    } else {
        write_stdout(&format!(
            "{}\nTo apply: akasha setup-root PATH --plan-id {}\nReplace PATH with the reviewed root. Omitting apply cancels without writing.\n",
            setup_plan_text(plan),
            plan.plan_id
        ))
    }
}
pub(crate) fn render_setup_result(
    result: &akasha_core::RootSetupResult,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(result))?)
    } else {
        write_stdout(&format!("{}\n", setup_result_text(result)))
    }
}
pub(crate) fn render_onboarding_handoff(
    handoff: &akasha_core::OnboardingHandoff,
    output: OutputMode,
) -> io::Result<()> {
    if output.json {
        write_stdout(&json_line(serde_json::to_string_pretty(handoff))?)
    } else {
        write_stdout(&onboarding_handoff_text(handoff))
    }
}
