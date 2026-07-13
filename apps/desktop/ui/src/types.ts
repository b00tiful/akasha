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

export interface LibraryProjection {
  root: string;
  selected_project: string;
  global: { categories: LibraryCategory[] };
  projects: LibraryShelf[];
  total_books: number;
}

export interface DesktopLibrary {
  projection: LibraryProjection;
  fallback_markdown: string;
}

export interface LibraryDocument {
  id: string;
  source: string;
}

export interface CommandError {
  code: number;
  message: string;
}
