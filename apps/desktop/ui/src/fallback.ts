import type { LibraryBook, LibraryCategory, LibraryProjection } from "./types";

export function renderFallback(
  host: HTMLElement,
  projection: LibraryProjection,
  onSelect: (book: LibraryBook) => void,
): void {
  host.replaceChildren();
  appendCollection(host, "Global knowledge", projection.global.categories, onSelect);
  for (const shelf of projection.projects) {
    appendCollection(
      host,
      `${shelf.project} / ${shelf.status}`,
      shelf.categories,
      onSelect,
    );
  }
}

function appendCollection(
  host: HTMLElement,
  label: string,
  categories: LibraryCategory[],
  onSelect: (book: LibraryBook) => void,
): void {
  const section = document.createElement("section");
  const heading = document.createElement("h3");
  heading.textContent = label;
  section.append(heading);

  for (const category of categories) {
    const details = document.createElement("details");
    details.open = category.books.length > 0;
    const summary = document.createElement("summary");
    summary.textContent = `${category.note_type} / ${category.class} / ${category.books.length}`;
    details.append(summary);
    if (category.books.length === 0) {
      const empty = document.createElement("p");
      empty.textContent = "No books";
      details.append(empty);
    }
    for (const book of category.books) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "book-button";
      button.textContent = book.id;
      button.title = book.explanation;
      button.addEventListener("click", () => onSelect(book));
      details.append(button);
    }
    section.append(details);
  }
  host.append(section);
}
