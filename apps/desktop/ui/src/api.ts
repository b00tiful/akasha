import { invoke } from "@tauri-apps/api/core";
import type { AssistedLibrarianReview, DesktopLibrarianReview, LibrarianPacket } from "./librarian-types";

/** Read-only, selected-project saved sources; repository inspection is explicit. */
export function inspectLibrarian(root: string, project: string, question: string, provenance: boolean, terms: string[] = []): Promise<DesktopLibrarianReview> {
  return invoke("inspect_librarian", { root: optional(root), project, question, provenance, terms });
}

export function assistLibrarian(requestId: string, root: string, project: string, question: string, provenance: boolean, profile: "vulkan" | "cpu"): Promise<AssistedLibrarianReview> {
  return invoke("assist_librarian_sources", { requestId, root: optional(root), project, question, provenance, profile });
}

export function followLibrarianSource(root: string, project: string, packet: LibrarianPacket, citation: number): Promise<LinkedLibraryDocument> {
  return invoke("follow_librarian_source", { root: optional(root), project, packet, citation });
}

import type {
  DesktopLibrary,
  LinkedLibraryDocument,
  LibrarySearchResult,
  LocalNavigationState,
  NoteEditResult,
  NoteLinksResult,
  PendingNoteEditInspection,
} from "./types";

function optional(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}

/** May roll back or finalize a pending project mutation before loading the library. */
export function loadLibraryWithRecovery(root: string, project: string): Promise<DesktopLibrary> {
  return invoke<DesktopLibrary>("load_library_with_recovery", {
    root: optional(root),
    project: optional(project),
  });
}

export function inspectRecovery(root: string, project: string): Promise<PendingNoteEditInspection> {
  return invoke<PendingNoteEditInspection>("inspect_recovery", {
    root: optional(root),
    project: optional(project),
  });
}

/** May roll back or finalize a pending project mutation before loading exact source. */
export function loadDocumentWithRecovery(
  root: string,
  project: string,
  id: string,
): Promise<LinkedLibraryDocument> {
  return invoke<LinkedLibraryDocument>("load_linked_document_with_recovery", {
    root: optional(root),
    project: optional(project),
    id,
  });
}

/** Revalidates source and occurrence; never performs recovery or writes. */
export function followDocumentLink(
  root: string, project: string, id: string, expectedSource: string, from: number,
): Promise<LinkedLibraryDocument> {
  return invoke<LinkedLibraryDocument>("follow_document_link", {
    root: optional(root), project: optional(project), id, expectedSource, from,
  });
}

export function searchProject(
  root: string,
  project: string,
  query: string,
): Promise<LibrarySearchResult> {
  return invoke<LibrarySearchResult>("search_project", {
    root: optional(root),
    project,
    query,
  });
}

/** Saved-file inspection bound to the displayed baseline; never recovers or writes. */
export function inspectDocumentBacklinks(
  root: string, project: string, id: string, expectedSource: string, limit: number,
): Promise<NoteLinksResult> {
  return invoke<NoteLinksResult>("inspect_document_backlinks", {
    root: optional(root), project: optional(project), id, expectedSource, limit,
  });
}

/** Revalidates the displayed baseline and incoming edge; loads only a canonical book. */
export function followDocumentBacklink(
  root: string, project: string, id: string, expectedSource: string, sourceId: string, limit: number,
): Promise<LinkedLibraryDocument> {
  return invoke<LinkedLibraryDocument>("follow_document_backlink", {
    root: optional(root), project: optional(project), id, expectedSource, sourceId, limit,
  });
}

export function saveDocument(
  root: string,
  project: string,
  id: string,
  expectedSource: string,
  replacementSource: string,
): Promise<NoteEditResult> {
  return invoke<NoteEditResult>("save_document", {
    root: optional(root),
    project: optional(project),
    id,
    expectedSource,
    replacementSource,
  });
}

export function loadLocalNavigation(
  root: string,
  project: string,
): Promise<LocalNavigationState | null> {
  return invoke<LocalNavigationState | null>("load_local_navigation", { root, project });
}

export function saveLocalNavigation(state: LocalNavigationState): Promise<void> {
  return invoke<void>("save_local_navigation", { state });
}

/** Explicit read-only repository preparation; core owns discovery, paths and source identities. */
export function prepareOnboardingSources(root: string, project: string, selection: import("./onboarding-types").OnboardingSelection): Promise<import("./onboarding-types").OnboardingSources> {
  return invoke("prepare_local_onboarding_sources", { root: optional(root), project, selection });
}

/** Recompile a captured plan against current selected-project inputs, without writes. */
export function reviewOnboarding(root: string, project: string, packetJson: string, planJson: string): Promise<import("./onboarding-types").OnboardingReview> {
  return invoke("review_local_onboarding", { root: optional(root), project, packetJson, planJson });
}

/** Explicit publication; shared core rechecks the exact plan, packet and approved ID. */
export function publishOnboarding(root: string, project: string, packetJson: string, planJson: string, approvedPreviewId: string): Promise<import("./onboarding-types").OnboardingResult> {
  return invoke("publish_local_onboarding", { root: optional(root), project, packetJson, planJson, approvedPreviewId });
}

export function localInferenceAvailability(): Promise<import("./onboarding-types").InferenceAvailability> {
  return invoke("local_inference_availability");
}
export function importInferenceAsset(requestId: string, asset: import("./inference-setup").InferenceAsset, source: string): Promise<import("./inference-setup").InferenceSetupResult> {
  return invoke("import_local_inference_asset", { requestId, asset, source });
}
export function verifyInferenceAsset(requestId: string, asset: import("./inference-setup").InferenceAsset): Promise<import("./inference-setup").InferenceSetupResult> {
  return invoke("verify_local_inference_asset", { requestId, asset });
}
export function reserveLocalGeneration(): Promise<string> { return invoke("reserve_local_generation"); }
export function cancelLocalGeneration(requestId: string): Promise<void> { return invoke("cancel_local_generation", { requestId }); }
export function generateOnboarding(requestId: string, root: string, project: string, packetJson: string, profile: "vulkan" | "cpu"): Promise<import("./onboarding-types").GeneratedOnboarding> {
  return invoke("generate_local_onboarding", { requestId, root: optional(root), project, packetJson, profile });
}
