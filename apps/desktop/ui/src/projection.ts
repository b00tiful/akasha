import type { LibraryBook, LibraryProjection } from "./types";

export interface LibraryLink {
  source: string;
  target: string;
}

export interface PositionedBook {
  book: LibraryBook;
  x: number;
  y: number;
}

export function allBooks(projection: LibraryProjection): LibraryBook[] {
  return [
    ...projection.global.categories.flatMap((category) => category.books),
    ...projection.projects.flatMap((shelf) =>
      shelf.categories.flatMap((category) => category.books),
    ),
  ];
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

export function layoutBooks(projection: LibraryProjection): PositionedBook[] {
  const positioned: PositionedBook[] = [];
  layoutCategories(projection.global.categories, 68, positioned);
  projection.projects.forEach((shelf, shelfIndex) => {
    layoutCategories(shelf.categories, 230 + shelfIndex * 172, positioned);
  });
  return positioned;
}

function layoutCategories(
  categories: LibraryProjection["global"]["categories"],
  shelfX: number,
  output: PositionedBook[],
): void {
  categories.forEach((category, categoryIndex) => {
    category.books.forEach((book, bookIndex) => {
      output.push({
        book,
        x: shelfX + bookIndex * 20,
        y: 74 + categoryIndex * 68,
      });
    });
  });
}
