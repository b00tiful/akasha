import { invoke } from "@tauri-apps/api/core";

import type { DesktopLibrary, LibraryDocument, NoteEditResult } from "./types";

function optional(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}

export function loadLibrary(root: string, project: string): Promise<DesktopLibrary> {
  return invoke<DesktopLibrary>("load_library", {
    root: optional(root),
    project: optional(project),
  });
}

export function loadDocument(
  root: string,
  project: string,
  id: string,
): Promise<LibraryDocument> {
  return invoke<LibraryDocument>("load_document", {
    root: optional(root),
    project: optional(project),
    id,
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
