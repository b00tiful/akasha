import type {
  LibraryBook,
  LibraryCategory,
  LibraryProjection,
  NoteClass,
} from "./types";

export const VOLUME_SIZE = 20;
export const GLOBAL_SHELF_ID = "scope:global";

export interface LibraryLink {
  source: string;
  target: string;
}

export interface VisualShelf {
  id: string;
  label: string;
  status: string;
  kind: "global" | "project";
  categories: LibraryCategory[];
  noteCount: number;
}

export interface LibraryVolume {
  id: string;
  shelfId: string;
  noteType: string;
  class: NoteClass;
  index: number;
  label: string;
  books: LibraryBook[];
}

export function allBooks(projection: LibraryProjection): LibraryBook[] {
  return [
    ...projection.global.categories.flatMap((category) => category.books),
    ...projection.projects.flatMap((shelf) =>
      shelf.categories.flatMap((category) => category.books),
    ),
  ];
}

export function libraryShelves(projection: LibraryProjection): VisualShelf[] {
  const projects = projection.projects.map((shelf) => ({
    id: projectShelfId(shelf.project),
    label: shelf.project,
    status: shelf.status,
    kind: "project" as const,
    categories: shelf.categories,
    noteCount: shelf.categories.reduce((sum, category) => sum + category.books.length, 0),
  }));
  const global: VisualShelf = {
    id: GLOBAL_SHELF_ID,
    label: "Global knowledge",
    status: "shared",
    kind: "global",
    categories: projection.global.categories,
    noteCount: projection.global.categories.reduce(
      (sum, category) => sum + category.books.length,
      0,
    ),
  };
  return [...projects, global];
}

export function selectedShelfId(projection: LibraryProjection): string {
  return projectShelfId(projection.selected_project);
}

export function volumesForShelf(shelf: VisualShelf): LibraryVolume[] {
  const volumes: LibraryVolume[] = [];
  for (const category of shelf.categories) {
    for (let offset = 0; offset < category.books.length; offset += VOLUME_SIZE) {
      const index = Math.floor(offset / VOLUME_SIZE) + 1;
      volumes.push({
        id: `${shelf.id}/${encodeURIComponent(category.note_type)}/${index}`,
        shelfId: shelf.id,
        noteType: category.note_type,
        class: category.class,
        index,
        label: `VOL ${romanNumeral(index)}`,
        books: category.books.slice(offset, offset + VOLUME_SIZE),
      });
    }
  }
  return volumes;
}

export function volumeForBook(shelf: VisualShelf, bookId: string): LibraryVolume | undefined {
  return volumesForShelf(shelf).find((volume) =>
    volume.books.some((book) => book.id === bookId),
  );
}

export function visibleLinks(projection: LibraryProjection): LibraryLink[] {
  const books = allBooks(projection);
  const projected = new Set(books.map((book) => book.id));
  const keys = new Set<string>();
  const links: LibraryLink[] = [];
  for (const book of books) {
    for (const target of book.outgoing_links) {
      const key = `${book.id}\u0000${target}`;
      if (!projected.has(target) || keys.has(key)) {
        continue;
      }
      keys.add(key);
      links.push({ source: book.id, target });
    }
  }
  return links;
}

export function projectShelfId(project: string): string {
  return `project:${project}`;
}

function romanNumeral(value: number): string {
  const numerals: ReadonlyArray<readonly [number, string]> = [
    [1000, "M"],
    [900, "CM"],
    [500, "D"],
    [400, "CD"],
    [100, "C"],
    [90, "XC"],
    [50, "L"],
    [40, "XL"],
    [10, "X"],
    [9, "IX"],
    [5, "V"],
    [4, "IV"],
    [1, "I"],
  ];
  let remaining = value;
  let output = "";
  for (const [unit, glyph] of numerals) {
    while (remaining >= unit) {
      output += glyph;
      remaining -= unit;
    }
  }
  return output;
}
