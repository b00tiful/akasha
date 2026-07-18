export type NoteClass = "event" | "record" | "entity";

export type LibraryScope =
  | { kind: "global" }
  | { kind: "project"; project: string };

export interface LibraryBook {
  id: string;
  label: string;
  scope: LibraryScope;
  note_type: string;
  class: NoteClass;
  status: string | null;
  reviewed: string | null;
  date: string | null;
  outgoing_links: string[];
  explanation: string;
}

export interface LibraryCategory {
  note_type: string;
  class: NoteClass;
  books: LibraryBook[];
}

export interface LibraryShelf {
  project: string;
  status: string;
  categories: LibraryCategory[];
}

export interface LibraryProjectDashboard {
  project: string;
  status: string;
  notes: number;
  populated_categories: number;
  open_tasks: number;
  open_problems: number;
  validated_links: number;
  latest_activity_date: string | null;
}

export interface LibraryDashboard {
  validation_passed: boolean;
  projects: number;
  notes: number;
  global_notes: number;
  configured_categories: number;
  open_tasks: number;
  open_problems: number;
  validated_links: number;
  latest_activity_date: string | null;
  project_metrics: LibraryProjectDashboard[];
}

export interface LibraryProjection {
  root: string;
  selected_project: string;
  global: { categories: LibraryCategory[] };
  projects: LibraryShelf[];
  total_books: number;
  dashboard: LibraryDashboard;
}

export interface DesktopLibrary {
  projection: LibraryProjection;
  fallback_markdown: string;
  recovery: NoteEditRecovery;
}

export interface LibraryDocument {
  id: string;
  source: string;
}

export type NoteEditRecovery = "none" | "discarded" | "rolled-back" | "finalized";

export interface NoteEditResult {
  root: string;
  project: string;
  project_dir: string;
  id: string;
  changed: boolean;
  recovery: NoteEditRecovery;
}

export interface CommandError {
  code: number;
  message: string;
}
