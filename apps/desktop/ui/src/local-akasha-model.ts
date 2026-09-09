import type { LibraryCategory, LibraryProjection } from "./types";

export interface LocalAkashaModel {
  key: string;
  project: string;
  root: string;
  sections: LibraryCategory[];
  noteCount: number;
}

// Presentation over the core's classification: never infer folders from note paths.
export function localAkashaModel(projection: LibraryProjection): LocalAkashaModel {
  const shelf = projection.projects.find((item) => item.project === projection.selected_project);
  if (!shelf) throw new Error("The selected project is absent from the validated library.");
  return {
    key: JSON.stringify([projection.root, shelf.project]),
    project: shelf.project,
    root: projection.root,
    sections: shelf.categories,
    noteCount: shelf.categories.reduce((count, section) => count + section.books.length, 0),
  };
}
