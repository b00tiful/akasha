import { describe, expect, it } from "vitest";

import {
  VOLUME_SIZE,
  allBooks,
  libraryShelves,
  selectedShelfId,
  visibleLinks,
  volumeForBook,
  volumesForShelf,
} from "./projection";
import type { LibraryProjection } from "./types";

const projection: LibraryProjection = {
  root: "/synthetic",
  selected_project: "alpha",
  global: {
    categories: [
      {
        note_type: "entity",
        class: "entity",
        books: [book("Global/entities/pattern.md", ["Projects/alpha/entities/core.md"])],
      },
    ],
  },
  projects: [
    {
      project: "alpha",
      status: "active",
      categories: [
        {
          note_type: "entity",
          class: "entity",
          books: [
            book("Projects/alpha/entities/core.md", [
              "Projects/alpha/index.md",
              "Global/entities/pattern.md",
            ]),
          ],
        },
      ],
    },
  ],
  total_books: 2,
  dashboard: {
    validation_passed: true,
    projects: 1,
    notes: 2,
    global_notes: 1,
    configured_categories: 1,
    open_tasks: 0,
    open_problems: 0,
    validated_links: 3,
    latest_activity_date: null,
    project_metrics: [
      {
        project: "alpha",
        status: "active",
        notes: 1,
        populated_categories: 1,
        open_tasks: 0,
        open_problems: 0,
        validated_links: 2,
        latest_activity_date: null,
      },
    ],
  },
};

describe("desktop projection helpers", () => {
  it("keeps canonical identities supplied by the Rust projection", () => {
    expect(allBooks(projection).map((item) => item.id)).toEqual([
      "Global/entities/pattern.md",
      "Projects/alpha/entities/core.md",
    ]);
    expect(selectedShelfId(projection)).toBe("project:alpha");
    expect(libraryShelves(projection).map((shelf) => shelf.id)).toEqual([
      "project:alpha",
      "scope:global",
    ]);
  });

  it("renders traffic only when both validated books are visible", () => {
    expect(visibleLinks(projection)).toEqual([
      {
        source: "Global/entities/pattern.md",
        target: "Projects/alpha/entities/core.md",
      },
      {
        source: "Projects/alpha/entities/core.md",
        target: "Global/entities/pattern.md",
      },
    ]);
  });

  it("groups stable path-ordered notes into twenty-note volumes", () => {
    const books = Array.from({ length: VOLUME_SIZE * 2 + 1 }, (_, index) =>
      book(`Projects/alpha/entities/book-${String(index).padStart(2, "0")}.md`, []),
    );
    const shelf = libraryShelves({
      ...projection,
      global: { categories: [] },
      projects: [
        {
          project: "alpha",
          status: "active",
          categories: [{ note_type: "entity", class: "entity", books }],
        },
      ],
      total_books: books.length,
    })[0];
    expect(shelf).toBeDefined();
    if (!shelf) {
      throw new Error("expected project shelf");
    }
    const volumes = volumesForShelf(shelf);

    expect(volumes.map((volume) => [volume.label, volume.books.length])).toEqual([
      ["VOL I", 20],
      ["VOL II", 20],
      ["VOL III", 1],
    ]);
    expect(volumes.at(1)?.books.at(0)?.id).toBe(
      "Projects/alpha/entities/book-20.md",
    );
    expect(volumeForBook(shelf, "Projects/alpha/entities/book-40.md")?.label).toBe(
      "VOL III",
    );
  });
});

function book(id: string, outgoing_links: string[]) {
  return {
    id,
    label: id.split("/").at(-1)?.replace(/\.md$/u, "") ?? id,
    scope: id.startsWith("Global/")
      ? ({ kind: "global" } as const)
      : ({ kind: "project", project: "alpha" } as const),
    note_type: "entity",
    class: "entity" as const,
    status: "active",
    reviewed: "2026-07-13",
    date: null,
    outgoing_links,
    explanation: id,
  };
}
