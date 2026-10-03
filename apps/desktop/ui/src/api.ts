import { invoke } from "@tauri-apps/api/core";

import type {
  DesktopLibrary,
  LibraryDocument,
  LibrarySearchResult,
  LocalNavigationState,
  NoteEditResult,
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
): Promise<LibraryDocument> {
  return invoke<LibraryDocument>("load_document_with_recovery", {
    root: optional(root),
    project: optional(project),
    id,
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
